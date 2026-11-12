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

fn prepare(crossing: bool) -> motion_core::Result<PreparedGpuFrame> {
    prepare_node(scene(crossing)?, PixelSize::new(240, 160))
}

/// CPU reference preparation of the same scene.
fn prepare_cpu(crossing: bool) -> motion_core::Result<crate::PreparedCpuFrame> {
    let size = PixelSize::new(240, 160);
    let validation = ValidationLimits::default();
    let list = scene(crossing)?.compile(motion_core::Size::new(240.0, 160.0), size, validation)?;
    Ok(list.prepare(validation, RasterLimits::default())?)
}

fn prepare_node(node: Node, size: PixelSize) -> motion_core::Result<PreparedGpuFrame> {
    let validation = ValidationLimits::default();
    let list = node.compile(
        motion_core::Size::new(f64::from(size.width), f64::from(size.height)),
        size,
        validation,
    )?;
    Ok(crate::gpu::raster::prepare::prepare_validated_device(
        &list,
        validation,
        RasterLimits::default(),
        1.0 / 256.0,
        None,
    )?)
}

#[test]
fn coverage_shader_constants_match_the_host_planner() {
    use crate::gpu::raster::device::{COVERAGE_CARRY_STRIDE, COVERAGE_TILE_SIZE};
    let source = include_str!("../../shaders/coverage.wgsl");
    let constant = |name: &str| -> u64 {
        let prefix = format!("const {name} = ");
        let line = source
            .lines()
            .find_map(|line| line.strip_prefix(prefix.as_str()))
            .unwrap_or_else(|| panic!("{name} missing"));
        line.trim_end_matches("u;").parse().unwrap()
    };
    assert_eq!(constant("TILE_SIZE"), u64::from(COVERAGE_TILE_SIZE));
    assert_eq!(constant("CARRY_STRIDE"), COVERAGE_CARRY_STRIDE);
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
fn device_fills_replace_cpu_coverage() -> motion_core::Result {
    let device = prepare(true)?;
    assert_eq!(device.geometry.device_fills.len(), 5);
    assert!(device.records.alphas.is_empty());
    assert!(device.has_device_coverage());
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
    let device = prepare(crossing)?;
    let expected = prepare_cpu(crossing)?
        .0
        .compose_linear(limits)?
        .to_rgba8_frame(false, limits)?;
    let mut gpu = Gpu::new()?;
    let actual = gpu
        .render(&device.records, Plan::new(&device, limits)?)?
        .to_rgba8_frame(false, limits)?;
    assert_visually_equivalent(actual.pixels(), expected.pixels());
    let replay = gpu
        .render(&device.records, Plan::new(&device, limits)?)?
        .to_rgba8_frame(false, limits)?;
    assert_visually_equivalent(actual.pixels(), replay.pixels());
    Ok(())
}

pub(super) fn assert_visually_equivalent(actual: &[[u8; 4]], expected: &[[u8; 4]]) {
    // This scene permits localized antialiasing differences, including at crossing edges.
    crate::gpu::visual::compare_rgba8(actual, expected, 0.01).unwrap();
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
        PixelSize::new(3, 2),
    )?;
    assert_eq!(frame.geometry.device_fills.len(), 2);
    assert_eq!(
        frame.records.strips[frame.geometry.device_fills[0].strip].alpha_offset,
        0
    );
    assert_eq!(
        frame.records.strips[frame.geometry.device_fills[1].strip].alpha_offset,
        1
    );
    let limits = RasterLimits::default();
    let plan = Plan::new(&frame, limits)?;
    assert_eq!(plan.device.samples, 2);
    assert_eq!(plan.device.fills[0].coverage, 0);
    for (index, fill) in frame.geometry.device_fills.iter().enumerate() {
        let strip = 4 + fill.strip * 5;
        assert_eq!(plan.coverage[strip + 4] as usize, index);
        assert_eq!(plan.coverage[strip + 3] >> 31, 1);
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
    let mixed = || -> motion_core::Result<Node> {
        Ok(Node::group([
            triangle(0.0)?,
            triangle(1.0)?,
            Node::from(motion_core::Rect::new(2.0, 0.0, 2.5, 1.0).fill(Color::WHITE)),
        ]))
    };
    let size = PixelSize::new(3, 2);
    let frame = prepare_node(mixed()?, size)?;
    let limits = RasterLimits::default();
    assert_eq!(frame.geometry.device_fills.len(), 2);
    assert!(!frame.records.alphas.is_empty());
    let mut gpu = Gpu::new()?;
    let actual = gpu.render(&frame.records, Plan::new(&frame, limits)?)?;
    for x in 0..2 {
        let pixel = actual.pixel(x, 0).unwrap();
        for value in pixel {
            assert!((value - 0.18).abs() < 1e-5, "{pixel:?}");
            assert!((value * 255.0 - (value * 255.0).round()).abs() > 0.05);
        }
    }
    let validation = ValidationLimits::default();
    let list = mixed()?.compile(motion_core::Size::new(3.0, 2.0), size, validation)?;
    let cpu = list.prepare(validation, limits)?.compose_linear(limits)?;
    assert_eq!(actual.pixel(2, 0), cpu.pixel(2, 0));
    assert_visually_equivalent(
        actual.to_rgba8_frame(false, limits)?.pixels(),
        cpu.to_rgba8_frame(false, limits)?.pixels(),
    );
    Ok(())
}
