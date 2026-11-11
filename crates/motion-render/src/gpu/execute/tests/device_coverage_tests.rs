//! GPU-computed fill coverage against the CPU reference.

use super::*;
use motion_core::{Color, FillRule, Node, Path2D, Point, Shape};

fn star(center: Point, radius: f64, rule: FillRule, color: Color) -> motion_core::Result<Node> {
    let mut path = Path2D::new();
    for index in 0..5 {
        let angle = f64::from(index) * 4.0 * std::f64::consts::PI / 5.0 - 1.5;
        let point = Point::new(
            center.x + radius * angle.cos(),
            center.y + radius * angle.sin(),
        );
        if index == 0 {
            path.try_move_to(point)?;
        } else {
            path.try_line_to(point)?;
        }
    }
    path.try_close()?;
    Ok(Node::from(path.fill(color).fill_rule(rule)))
}

fn blob(color: Color) -> motion_core::Result<Node> {
    let mut path = Path2D::new();
    path.try_move_to(Point::new(120.0, 20.3))?;
    path.try_cubic_to(
        Point::new(190.0, 10.0),
        Point::new(200.0, 80.0),
        Point::new(150.5, 90.7),
    )?;
    path.try_cubic_to(
        Point::new(110.0, 120.0),
        Point::new(90.0, 60.0),
        Point::new(120.0, 20.3),
    )?;
    path.try_close()?;
    Ok(Node::from(path.fill(color)))
}

fn sliver(color: Color) -> motion_core::Result<Node> {
    let mut path = Path2D::new();
    path.try_move_to(Point::new(10.2, 140.1))?;
    path.try_line_to(Point::new(210.7, 150.4))?;
    path.try_line_to(Point::new(210.7, 150.9))?;
    path.try_line_to(Point::new(10.2, 140.6))?;
    path.try_close()?;
    Ok(Node::from(path.fill(color)))
}

fn scene(crossing: bool) -> motion_core::Result<Node> {
    let stars = if crossing {
        vec![
            star(
                Point::new(50.0, 55.0),
                40.0,
                FillRule::NonZero,
                Color::rgb(230, 90, 60),
            )?,
            star(
                Point::new(50.0, 55.0),
                30.0,
                FillRule::EvenOdd,
                Color::rgb(30, 80, 200).with_alpha(0.7),
            )?,
        ]
    } else {
        Vec::new()
    };
    Ok(Node::group(
        stars.into_iter().chain([
            blob(Color::rgb(60, 200, 120))?,
            sliver(Color::rgb(250, 240, 40))?,
            Node::from(
                motion_core::Circle::new(Point::new(200.0, 40.0), 24.0)?
                    .fill(Color::rgb(180, 60, 220).with_alpha(0.6)),
            ),
        ]),
    ))
}

fn prepare(
    device: bool,
    crossing: bool,
) -> motion_core::Result<crate::raster::gpu_prepare::GpuFrame> {
    prepare_node(scene(crossing)?, device, PixelSize::new(240, 160))
}

fn prepare_node(
    node: Node,
    device: bool,
    size: PixelSize,
) -> motion_core::Result<crate::raster::gpu_prepare::GpuFrame> {
    let validation = ValidationLimits::default();
    let list = node.compile(
        motion_core::Size::new(f64::from(size.width), f64::from(size.height)),
        size,
        validation,
    )?;
    Ok(crate::prepared::prepare_validated_device(
        &list,
        validation,
        RasterLimits::default(),
        1.0 / 256.0,
        None,
        device,
    )?)
}

#[test]
fn coverage_shader_validates_without_a_device() {
    let shader =
        wgpu::naga::front::wgsl::parse_str(include_str!("../../shaders/coverage.wgsl")).unwrap();
    wgpu::naga::valid::Validator::new(
        wgpu::naga::valid::ValidationFlags::all(),
        wgpu::naga::valid::Capabilities::all(),
    )
    .validate(&shader)
    .unwrap();
}

#[test]
fn device_fills_replace_cpu_coverage_and_materialize_back() -> motion_core::Result {
    let limits = RasterLimits::default();
    let device = prepare(true, true)?;
    let cpu = prepare(false, true)?;
    assert_eq!(device.geometry.device_fills.len(), 5);
    assert!(device.alphas.is_empty());
    let mut restored = device.clone();
    restored.materialize_device_coverage(limits)?;
    assert!(!restored.has_device_coverage());
    assert_eq!(
        restored
            .compose_linear(limits)?
            .to_rgba8_frame(false, limits)?,
        cpu.compose_linear(limits)?.to_rgba8_frame(false, limits)?
    );
    assert_eq!(
        device
            .compose_linear(limits)?
            .to_rgba8_frame(false, limits)?,
        cpu.compose_linear(limits)?.to_rgba8_frame(false, limits)?
    );
    Ok(())
}

#[test]
#[ignore = "requires native GPU; device fill coverage CPU/GPU parity"]
fn device_fill_coverage_matches_cpu_and_replays() -> motion_core::Result {
    assert_matches_cpu(false)?;
    assert_matches_cpu(true)
}

fn assert_matches_cpu(crossing: bool) -> motion_core::Result {
    let limits = RasterLimits::default();
    let device = prepare(true, crossing)?;
    let expected = prepare(false, crossing)?
        .compose_linear(limits)?
        .to_rgba8_frame(false, limits)?;
    let mut gpu = Gpu::new()?;
    let actual = gpu
        .render(&device, Plan::new(&device, limits)?)?
        .to_rgba8_frame(false, limits)?;
    assert_visually_equivalent(actual.pixels(), expected.pixels());
    let replay = gpu
        .render(&device, Plan::new(&device, limits)?)?
        .to_rgba8_frame(false, limits)?;
    assert_visually_equivalent(actual.pixels(), replay.pixels());
    Ok(())
}

fn assert_visually_equivalent(actual: &[[u8; 4]], expected: &[[u8; 4]]) {
    assert_eq!(actual.len(), expected.len());
    let mut squared = 0.0;
    let mut samples = 0;
    for (actual, expected) in actual.iter().zip(expected) {
        let a = actual.map(|value| f64::from(value) / 255.0);
        let b = expected.map(|value| f64::from(value) / 255.0);
        if a[3] == 0.0 && b[3] == 0.0 {
            continue;
        }
        squared += (a[3] - b[3]).powi(2);
        samples += 1;
        for background in [0.0, 1.0] {
            for channel in 0..3 {
                let first = a[channel] * a[3] + background * (1.0 - a[3]);
                let second = b[channel] * b[3] + background * (1.0 - b[3]);
                squared += (first - second).powi(2);
                samples += 1;
            }
        }
    }
    assert!(samples > 0, "Fixture must contain visible artwork");
    let rms = (squared / f64::from(samples)).sqrt();
    eprintln!("device_fill visual_rms={rms:.6}");
    // This scene permits localized antialiasing differences, including at crossing edges.
    assert!(rms < 0.01, "Visual RMS difference {rms}");
}

fn triangle(left: f64) -> motion_core::Result<Node> {
    let path = Path2D::builder()
        .move_to(Point::new(left + 0.1, 0.1))
        .line_to(Point::new(left + 0.7, 0.1))
        .line_to(Point::new(left + 0.1, 0.7))
        .close()
        .build()?;
    Ok(Node::from(path.fill(Color::WHITE)))
}

#[test]
fn device_float_coverage_layout_and_admission() -> motion_core::Result {
    let frame = prepare_node(
        Node::group([triangle(0.0)?, triangle(1.0)?]),
        true,
        PixelSize::new(3, 2),
    )?;
    assert_eq!(frame.geometry.device_fills.len(), 2);
    assert_eq!(
        frame.strips[frame.geometry.device_fills[0].strip].alpha_offset,
        0
    );
    assert_eq!(
        frame.strips[frame.geometry.device_fills[1].strip].alpha_offset,
        1
    );
    let limits = RasterLimits::default();
    let plan = Plan::new(&frame, limits)?;
    assert_eq!(plan.device.samples, 2);
    let base = plan.device.fills[4] as usize;
    assert_eq!(plan.coverage.len(), base + 2);
    for (index, fill) in frame.geometry.device_fills.iter().enumerate() {
        let strip = 4 + fill.strip * 6;
        assert_eq!(plan.coverage[strip + 4] as usize, base + index);
        assert_eq!(plan.coverage[strip + 5], 1);
    }
    assert!(
        Plan::new(
            &frame,
            RasterLimits {
                max_work: 1,
                ..limits
            }
        )
        .is_err()
    );
    Ok(())
}

#[test]
#[ignore = "requires native GPU; verifies unquantized coverage and mixed CPU/GPU records"]
fn device_float_coverage_survives_composition() -> motion_core::Result {
    let frame = prepare_node(
        Node::group([
            triangle(0.0)?,
            triangle(1.0)?,
            Node::from(motion_core::Rect::new(2.0, 0.0, 2.5, 1.0).fill(Color::WHITE)),
        ]),
        true,
        PixelSize::new(3, 2),
    )?;
    let limits = RasterLimits::default();
    assert_eq!(frame.geometry.device_fills.len(), 2);
    assert!(!frame.alphas.is_empty());
    let mut gpu = Gpu::new()?;
    let actual = gpu.render(&frame, Plan::new(&frame, limits)?)?;
    for x in 0..2 {
        let pixel = actual.pixel(x, 0).unwrap();
        for value in pixel {
            assert!((value - 0.18).abs() < 1e-5, "{pixel:?}");
            assert!((value * 255.0 - (value * 255.0).round()).abs() > 0.05);
        }
    }
    let cpu = frame.compose_linear(limits)?;
    assert_eq!(actual.pixel(2, 0), cpu.pixel(2, 0));
    assert_visually_equivalent(
        actual.to_rgba8_frame(false, limits)?.pixels(),
        cpu.to_rgba8_frame(false, limits)?.pixels(),
    );
    Ok(())
}
