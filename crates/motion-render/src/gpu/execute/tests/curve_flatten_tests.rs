//! Curve-only uploads, conservative generated storage and native curved-fill composition.

use super::*;
use motion_core::{ClipMode, Color, FillRule, Filter, Node, Path2D, Point, Rect, Size};

fn curve(rule: FillRule) -> motion_core::Result<Node> {
    let path = Path2D::builder()
        .move_to(Point::new(4.25, 16.0))
        .quadratic_to(Point::new(16.0, -3.0), Point::new(30.25, 16.5))
        .cubic_to(
            Point::new(38.0, 40.0),
            Point::new(-8.0, 32.0),
            Point::new(4.25, 16.0),
        )
        .close()
        .move_to(Point::new(12.25, 16.0))
        .quadratic_to(Point::new(20.0, 7.0), Point::new(26.25, 16.5))
        .cubic_to(
            Point::new(29.0, 27.0),
            Point::new(9.0, 26.0),
            Point::new(12.25, 16.0),
        )
        .close()
        .build()?;
    Ok(Node::from(
        path.fill(Color::rgb(40, 180, 90)).fill_rule(rule),
    ))
}

fn prepare(node: &Node) -> motion_core::Result<crate::gpu::raster::prepare::GpuFrame> {
    let validation = ValidationLimits::default();
    let list = node.compile(Size::new(40.0, 40.0), PixelSize::new(40, 40), validation)?;
    Ok(crate::gpu::raster::prepare::prepare_validated_device(
        &list,
        validation,
        RasterLimits::default(),
        1.0 / 256.0,
        None,
    )?)
}

#[test]
fn gpu_curve_flattening_has_no_host_edges_and_exact_admission() -> motion_core::Result {
    let frame = prepare(&curve(FillRule::EvenOdd)?)?;
    assert!(frame.geometry.device_edges.is_empty());
    assert!(!frame.geometry.device_curves.is_empty());
    assert!(frame.alphas.is_empty());
    let limits = RasterLimits::default();
    let plan = Plan::new(&frame, limits)?;
    assert!(plan.device.edges.is_empty());
    assert_eq!(plan.device.curves.len(), frame.geometry.device_curves.len());
    let generated = plan
        .device
        .curves
        .iter()
        .map(|curve| curve.segments)
        .sum::<u32>();
    assert_eq!(plan.device.edge_count, generated);
    assert_eq!(plan.device.buffer_sizes()[1], u64::from(generated) * 32);
    assert_eq!(
        plan.device.buffer_sizes()[5],
        plan.device.curves.len() as u64 * 48
    );
    for curve in &plan.device.curves {
        assert!(curve.first_edge + curve.segments <= plan.device.edge_count);
        assert!(curve.segments > 0);
    }
    let admit = |limits| -> motion_core::Result<(u64, u64)> {
        let mut planner = Planner {
            frame: frame.gpu_view(),
            steps: Vec::new(),
            budget: Budget::new(frame.size, limits)?,
        };
        planner.device_coverage()?;
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
    let reduced = wgpu::Limits {
        max_storage_buffer_binding_size: plan.device.buffer_sizes()[1] - 1,
        ..Default::default()
    };
    assert!(plan.validate_device(&reduced).is_err());
    Ok(())
}

#[test]
fn packed_output_validates_its_own_buffer_instead_of_unused_linear_storage() -> motion_core::Result
{
    let validation = ValidationLimits::default();
    let limits = RasterLimits::default();
    let frame = Node::from(Rect::new(0.0, 0.0, 64.0, 64.0).fill(Color::WHITE))
        .compile(Size::new(64.0, 64.0), PixelSize::new(64, 64), validation)?
        .prepare(validation, limits)?;
    let format = crate::PixelFormat::SrgbRgba8 {
        premultiplied: false,
    };
    let linear = Plan::new(&frame, limits)?;
    let packed = Plan::new_packed(&frame, limits, format)?;
    let device = wgpu::Limits {
        max_storage_buffer_binding_size: 32 * 1024,
        ..Default::default()
    };
    assert_eq!(
        linear.validate_device(&device).unwrap_err().field,
        "buffer_size"
    );
    packed.validate_device(&device)?;
    assert_eq!(
        packed
            .validate_device(&wgpu::Limits {
                max_storage_buffer_binding_size: 64 * 64 * 4 - 1,
                ..device
            })
            .unwrap_err()
            .field,
        "buffer_size"
    );
    Ok(())
}

#[test]
#[ignore = "requires native GPU; GPU quadratic/cubic flattening with clip/filter and replay"]
fn gpu_curve_flattening_cpu_gpu_equivalence_and_replay() -> motion_core::Result {
    let limits = RasterLimits::default();
    let mut gpu = Gpu::new()?;
    for rule in [FillRule::NonZero, FillRule::EvenOdd] {
        let node = curve(rule)?
            .clip_with(Rect::new(3.0, 4.0, 35.0, 33.0), ClipMode::Geometric)
            .filter(Filter::Blur { sigma: 0.3 });
        let device = prepare(&node)?;
        assert!(device.geometry.device_edges.is_empty());
        assert!(!device.geometry.device_curves.is_empty());
        let cpu = node
            .compile(
                Size::new(40.0, 40.0),
                PixelSize::new(40, 40),
                ValidationLimits::default(),
            )?
            .prepare(ValidationLimits::default(), limits)?;
        let reference = cpu
            .0
            .compose_linear(limits)?
            .to_rgba8_frame(false, limits)?;
        let actual = gpu
            .render(&device, Plan::new(&device, limits)?)?
            .to_rgba8_frame(false, limits)?;
        let replay = gpu
            .render(&device, Plan::new(&device, limits)?)?
            .to_rgba8_frame(false, limits)?;
        crate::gpu::visual::compare_rgba8(actual.pixels(), reference.pixels(), 0.01).unwrap();
        assert_eq!(actual, replay);
    }
    Ok(())
}
