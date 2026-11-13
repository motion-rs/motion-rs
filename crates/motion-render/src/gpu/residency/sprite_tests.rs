//! Sprite atlas separation, exact admission and reuse across changing primitive records.

use crate::gpu::{FrameInput, Gpu, Plan, Planner};
use crate::prepared::{PreparedCpuFrame, PreparedItem};
use crate::{RasterLimits, RenderPrepare};
use motion_core::budget::Budget;
use motion_core::instances::{
    ExtendedInstanceRecord, InstanceBatch, InstanceComposite, InstanceKind, InstanceRecord,
    InstanceUpload,
};
use motion_core::{
    BlendSpace, Node, PixelSize, Point, Rect, Size, Transform, ValidationError, ValidationLimits,
};
use std::sync::Arc;

#[derive(Debug)]
struct SpriteFixture {
    time: f32,
    count: usize,
    size: u32,
    extended: bool,
    atlas: Arc<[[f32; 4]]>,
}

impl SpriteFixture {
    fn new(time: f32, count: usize, size: u32, extended: bool) -> Self {
        let atlas = (0..size)
            .flat_map(|row| {
                (0..size).map(move |column| {
                    [
                        column as f32 / size as f32 * 0.6,
                        row as f32 / size as f32 * 0.6,
                        0.2,
                        0.6,
                    ]
                })
            })
            .collect::<Vec<_>>()
            .into();
        Self {
            time,
            count,
            size,
            extended,
            atlas,
        }
    }

    fn record(&self) -> InstanceRecord {
        InstanceRecord {
            position: [8.25 + self.time * 8.0, 9.125 + self.time * 3.0],
            start: [0.0; 2],
            color: [1.0; 4],
            radius: 3.25,
            rotation: self.time * 0.35,
            width: 1.0,
            kind: if self.extended {
                InstanceKind::SpriteSheet
            } else {
                InstanceKind::Sprite
            },
        }
    }
}

impl InstanceBatch for SpriteFixture {
    fn len(&self) -> usize {
        self.count
    }

    fn bounds(&self) -> Rect {
        Rect::new(0.0, 0.0, 32.0, 24.0)
    }

    fn bytes(&self) -> u64 {
        self.atlas.len() as u64 * 16
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
        _: f64,
        mut destination: [f32; 4],
        space: BlendSpace,
        composite: InstanceComposite,
    ) -> Result<[f32; 4], ValidationError> {
        let record = self.record();
        let relative = [
            point.x - f64::from(record.position[0]),
            point.y - f64::from(record.position[1]),
        ];
        let cosine = libm::cos(f64::from(record.rotation));
        let sine = libm::sin(f64::from(record.rotation));
        let local = [
            relative[0] * cosine + relative[1] * sine,
            -relative[0] * sine + relative[1] * cosine,
        ];
        let radius = f64::from(record.radius);
        if local.iter().any(|value| value.abs() >= radius) {
            return Ok(destination);
        }
        let position =
            local.map(|value| (value / (radius * 2.0) + 0.5) * f64::from(self.size) - 0.5);
        let origin = position.map(|value| value.floor() as i32);
        let amount = position.map(|value| (value - value.floor()) as f32);
        let texel = |column: i32, row: i32| {
            self.atlas[(row.clamp(0, self.size as i32 - 1) as u32 * self.size
                + column.clamp(0, self.size as i32 - 1) as u32) as usize]
        };
        let a = texel(origin[0], origin[1]);
        let b = texel(origin[0] + 1, origin[1]);
        let c = texel(origin[0], origin[1] + 1);
        let d = texel(origin[0] + 1, origin[1] + 1);
        let source = std::array::from_fn(|channel| {
            (a[channel] * (1.0 - amount[0]) + b[channel] * amount[0]) * (1.0 - amount[1])
                + (c[channel] * (1.0 - amount[0]) + d[channel] * amount[0]) * amount[1]
        });
        for _ in 0..self.count {
            composite(&mut destination, source, space)?;
        }
        Ok(destination)
    }

    fn upload_bytes(&self) -> u64 {
        (4 + self.count as u64 * if self.extended { 16 } else { 12 } + self.atlas.len() as u64 * 4)
            * 4
    }

    fn upload(&self) -> Result<Option<InstanceUpload>, ValidationError> {
        if self.extended {
            InstanceUpload::new_extended(
                (0..self.count).map(|_| ExtendedInstanceRecord {
                    base: self.record(),
                    extra: [0.0, 0.0, 1.0, 1.0],
                }),
                [self.size; 2],
                self.atlas.iter().copied(),
                motion_core::instances::InstanceBlend::SourceOver,
            )
            .map(Some)
        } else {
            InstanceUpload::new(
                (0..self.count).map(|_| self.record()),
                [self.size; 2],
                self.atlas.iter().copied(),
            )
            .map(Some)
        }
    }

    fn as_any(&self) -> &dyn std::any::Any {
        self
    }

    fn content_eq(&self, other: &dyn InstanceBatch) -> bool {
        other.as_any().downcast_ref::<Self>().is_some_and(|other| {
            self.time == other.time
                && self.count == other.count
                && self.size == other.size
                && self.extended == other.extended
                && self.atlas == other.atlas
        })
    }
}

fn frame(batch: Arc<dyn InstanceBatch>) -> motion_core::Result<PreparedCpuFrame> {
    let mut frame = Node::empty()
        .compile(
            Size::new(32.0, 24.0),
            PixelSize::new(32, 24),
            ValidationLimits::default(),
        )?
        .prepare(ValidationLimits::default(), RasterLimits::default())?;
    frame.items.push(PreparedItem::ParticleInstances {
        batch,
        transform: Transform::IDENTITY,
    });
    Ok(frame)
}

#[test]
fn sprite_atlas_separation_preserves_raw_upload_and_admits_exact_bytes_work() -> motion_core::Result
{
    for extended in [false, true] {
        let batch = Arc::new(SpriteFixture::new(0.5, 3, 8, extended));
        let raw = batch.upload()?.unwrap().into_words();
        let frame = frame(batch)?;
        let limits = RasterLimits::default();
        let plan = Plan::new(&frame, limits)?;
        let step = plan
            .steps
            .iter()
            .find(|step| step.resource.is_some())
            .unwrap();
        let first = 4 + 3 * if extended { 16 } else { 12 };
        assert_eq!(step.data, raw[..first]);
        assert_eq!(
            step.resource.as_ref().unwrap().words().as_slice(),
            &raw[first..]
        );
        let admit = |limits| -> motion_core::Result<(u64, u64)> {
            let mut planner = Planner {
                frame: frame.gpu_view(),
                steps: Plan::new(&frame, RasterLimits::default())?.steps,
                budget: Budget::new(frame.size(), limits)?,
            };
            // Mirrors the original checked upload's admission before splitting its atlas.
            planner
                .budget
                .bytes(raw.len() as u64 * 4, "gpu.particles")?;
            planner.budget.work(raw.len() as u64, "gpu.particles")?;
            let mut data = raw.clone();
            let atlas = planner.particle_resource(&mut data)?.unwrap();
            planner.attach_resource(1, atlas)?;
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
    }
    Ok(())
}

#[test]
fn sprite_atlas_device_limits_are_validated_without_a_device() -> motion_core::Result {
    let frame = frame(Arc::new(SpriteFixture::new(0.0, 1, 64, false)))?;
    let plan = Plan::new(&frame, RasterLimits::default())?;
    let bytes = 64 * 64 * 16;
    for limits in [
        wgpu::Limits {
            max_storage_buffer_binding_size: bytes - 1,
            ..Default::default()
        },
        wgpu::Limits {
            max_buffer_size: bytes - 1,
            ..Default::default()
        },
    ] {
        assert_eq!(
            plan.validate_device(&limits).unwrap_err().field,
            "buffer_size"
        );
    }
    Ok(())
}

#[test]
#[ignore = "requires native GPU; immutable sprite atlas reuse across changed records and times"]
fn sprite_atlas_stays_resident_across_records_times_and_strides() -> motion_core::Result {
    let limits = RasterLimits::default();
    let mut gpu = Gpu::new()?;
    gpu.context.clear_resident_resources()?;
    let initial = gpu.context.residency_stats();
    let mut replay = None;
    for (time, count, extended) in [
        (0.0, 1, false),
        (0.5, 257, false),
        (1.0, 5, true),
        (0.0, 1, false),
    ] {
        let frame = frame(Arc::new(SpriteFixture::new(time, count, 8, extended)))?;
        let reference = frame.compose_linear(limits)?;
        let actual = gpu.render(&frame, Plan::new(&frame, limits)?)?;
        assert!(actual.pixels().iter().any(|pixel| pixel[3] > 0.1));
        for (reference, actual) in reference.pixels().iter().zip(actual.pixels()) {
            assert!(
                reference
                    .iter()
                    .zip(actual)
                    .all(|(a, b)| (a - b).abs() <= 3e-4)
            );
        }
        if time == 0.0 {
            if let Some(first) = &replay {
                assert_eq!(actual.pixels(), first);
            }
            replay = Some(actual.pixels().to_vec());
        }
        let stats = gpu.context.residency_stats();
        assert_eq!(stats.misses, initial.misses + 1);
        assert_eq!(stats.upload_bytes, initial.upload_bytes + 8 * 8 * 16);
    }
    assert_eq!(gpu.context.residency_stats().hits, initial.hits + 3);
    Ok(())
}
