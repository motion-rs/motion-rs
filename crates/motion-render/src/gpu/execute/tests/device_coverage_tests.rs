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

fn prepare(device: bool, crossing: bool) -> motion_core::Result<crate::PreparedFrame> {
    let validation = ValidationLimits::default();
    let list = scene(crossing)?.compile(
        motion_core::Size::new(240.0, 160.0),
        PixelSize::new(240, 160),
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
    assert_eq!(device.device_fills.len(), 5);
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
    // Curves, slivers and circles without crossing edges are exact to the CPU within 2 LSB.
    assert_matches_cpu(false, 2)?;
    // Pixels where edges cross inside one pixel sum signed areas, so only they may deviate.
    assert_matches_cpu(true, 64)
}

fn assert_matches_cpu(crossing: bool, tolerance: u8) -> motion_core::Result {
    let limits = RasterLimits::default();
    let device = prepare(true, crossing)?;
    let expected = prepare(false, crossing)?
        .compose_linear(limits)?
        .to_rgba8_frame(false, limits)?;
    let mut gpu = Gpu::new()?;
    let actual = gpu
        .render(&device, Plan::new(&device, limits)?)?
        .to_rgba8_frame(false, limits)?;
    let differences = actual
        .pixels()
        .iter()
        .zip(expected.pixels())
        .map(|(actual, expected)| {
            actual
                .iter()
                .zip(expected)
                .map(|(a, e)| a.abs_diff(*e))
                .max()
                .unwrap()
        })
        .collect::<Vec<_>>();
    let maximum = *differences.iter().max().unwrap();
    let over = differences.iter().filter(|value| **value > 1).count();
    eprintln!(
        "device_fill max_diff={maximum} over_1lsb={over}/{}",
        differences.len()
    );
    assert!(maximum <= tolerance, "maximum difference {maximum}");
    assert!(
        over * 1000 < differences.len(),
        "{over} pixels differ by more than 1"
    );
    let replay = gpu
        .render(&device, Plan::new(&device, limits)?)?
        .to_rgba8_frame(false, limits)?;
    assert_eq!(actual, replay);
    Ok(())
}
