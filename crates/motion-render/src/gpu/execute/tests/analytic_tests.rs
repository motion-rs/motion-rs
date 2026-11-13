//! Device-independent analytic admission and compute-output consumption fixtures.

use super::*;
use motion_core::instances::{
    AnalyticInstanceState, AnalyticInstanceUpload, AnalyticTrajectoryKind, InstanceBatch,
    InstanceComposite, InstanceKind, InstanceRecord, InstanceUpload,
};
use motion_core::{BlendSpace, Point, Rect, Transform};
use std::sync::Arc;

#[derive(Debug)]
struct AnalyticFixture {
    program: AnalyticInstanceUpload,
    count: usize,
}

impl AnalyticFixture {
    fn new(count: usize) -> Result<Self, ValidationError> {
        let program = AnalyticInstanceUpload::new(
            AnalyticTrajectoryKind::Ballistic,
            42,
            1.0,
            3.0,
            [0.0; 8],
            InstanceKind::Disc,
            false,
            (0..count).map(|index| AnalyticInstanceState {
                position: [10.0, 10.0],
                velocity: [4.0, 0.0],
                custom: [0.0; 2],
                index: index as u32,
                rotation: 0.0,
            }),
        )?;
        Ok(Self { program, count })
    }
}

impl InstanceBatch for AnalyticFixture {
    fn len(&self) -> usize {
        self.count
    }

    fn bounds(&self) -> Rect {
        Rect::new(10.0, 6.0, 18.0, 14.0)
    }

    fn bytes(&self) -> u64 {
        self.program.bytes()
    }

    fn work(&self) -> u64 {
        self.count as u64 * 64
    }

    fn validate(&self) -> Result<(), ValidationError> {
        Ok(())
    }

    fn sample(
        &self,
        point: Point,
        antialias: f64,
        mut destination: [f32; 4],
        space: BlendSpace,
        composite: InstanceComposite,
    ) -> Result<[f32; 4], ValidationError> {
        let coverage = ((3.0 - libm::hypot(point.x - 14.0, point.y - 10.0)) / antialias + 0.5)
            .clamp(0.0, 1.0) as f32;
        for _ in 0..self.count {
            composite(&mut destination, [coverage; 4], space)?;
        }
        Ok(destination)
    }

    fn upload_bytes(&self) -> u64 {
        16 + self.count as u64 * 48
    }

    fn upload(&self) -> Result<Option<InstanceUpload>, ValidationError> {
        InstanceUpload::new(
            (0..self.count).map(|_| InstanceRecord {
                // Deliberately wrong: only evaluating the analytic program can draw the expected disc.
                position: [25.0, 20.0],
                start: [25.0, 20.0],
                color: [1.0; 4],
                radius: 3.0,
                rotation: 0.0,
                width: 0.0,
                kind: InstanceKind::Disc,
            }),
            [0, 0],
            [],
        )
        .map(Some)
    }

    fn analytic_upload(&self) -> Option<&AnalyticInstanceUpload> {
        Some(&self.program)
    }

    fn as_any(&self) -> &dyn std::any::Any {
        self
    }

    fn content_eq(&self, other: &dyn InstanceBatch) -> bool {
        other
            .as_any()
            .downcast_ref::<Self>()
            .is_some_and(|other| self.program == other.program)
    }
}

fn frame(count: usize) -> motion_core::Result<PreparedCpuFrame> {
    let mut frame = motion_core::Node::empty()
        .compile(
            motion_core::Size::new(32.0, 24.0),
            PixelSize::new(32, 24),
            ValidationLimits::default(),
        )?
        .prepare(ValidationLimits::default(), RasterLimits::default())?;
    frame.items.push(PreparedItem::ParticleInstances {
        batch: Arc::new(AnalyticFixture::new(count)?),
        transform: Transform::IDENTITY,
    });
    Ok(frame)
}

#[test]
fn analytic_shader_validates_without_a_device() {
    let shader =
        wgpu::naga::front::wgsl::parse_str(include_str!("../../shaders/analytic.wgsl")).unwrap();
    wgpu::naga::valid::Validator::new(
        wgpu::naga::valid::ValidationFlags::all(),
        wgpu::naga::valid::Capabilities::all(),
    )
    .validate(&shader)
    .unwrap();
}

#[test]
fn analytic_planning_zeros_reference_positions_and_admits_exact_boundaries() -> motion_core::Result
{
    let frame = frame(257)?;
    let limits = RasterLimits::default();
    let plan = Plan::new(&frame, limits)?;
    let step = plan
        .steps
        .iter()
        .find(|step| step.analytic.is_some())
        .unwrap();
    assert_eq!(step.parameters.flags.particle_tiles, 2);
    let bounded = Plan::new(
        &frame,
        RasterLimits {
            max_work: 1_000_000,
            ..limits
        },
    )?;
    assert_eq!(
        bounded
            .steps
            .iter()
            .find(|step| step.analytic.is_some())
            .unwrap()
            .parameters
            .flags
            .particle_tiles,
        1
    );
    assert_eq!(step.analytic.as_ref().unwrap().len(), 257);
    assert!(
        step.data[4..]
            .as_chunks::<12>()
            .0
            .iter()
            .all(|record| record[..4] == [0.0; 4])
    );
    let admit = |limits| -> motion_core::Result<(u64, u64)> {
        let mut planner = Planner {
            frame: frame.gpu_view(),
            steps: Vec::new(),
            budget: Budget::new(frame.size, limits)?,
        };
        planner.admit_analytic_program(step.analytic.as_ref().unwrap())?;
        Ok((planner.budget.peak_bytes(), planner.budget.work_done()))
    };
    let (bytes, work) = admit(limits)?;
    let exact = RasterLimits {
        max_bytes: bytes,
        max_work: work,
        ..limits
    };
    assert_eq!(admit(exact)?, (bytes, work));
    assert!(
        admit(RasterLimits {
            max_bytes: bytes - 1,
            ..exact
        })
        .is_err()
    );
    assert!(
        admit(RasterLimits {
            max_work: work - 1,
            ..exact
        })
        .is_err()
    );
    Ok(())
}

#[test]
#[ignore = "requires native GPU; analytic output must feed primitive support, tiles and composition"]
fn analytic_compute_records_are_consumed_and_replay() -> motion_core::Result {
    let limits = RasterLimits::default();
    let mut gpu = Gpu::new()?;
    let format = crate::PixelFormat::SrgbRgba8 {
        premultiplied: false,
    };
    for count in [1, 257] {
        let frame = frame(count)?;
        let expected = frame.render_linear(limits)?.to_format(format, limits)?;
        let actual = gpu
            .render_linear(&frame, limits)?
            .to_format(format, limits)?;
        assert!(actual.bytes()[(10 * 32 + 14) * 4 + 3] > 0);
        assert_eq!(actual.bytes()[(20 * 32 + 25) * 4 + 3], 0);
        assert!(
            expected
                .bytes()
                .iter()
                .zip(actual.bytes())
                .all(|(a, b)| a.abs_diff(*b) <= 1)
        );
        let replay = gpu
            .render_linear(&frame, limits)?
            .to_format(format, limits)?;
        assert_eq!(actual, replay);
    }
    Ok(())
}
