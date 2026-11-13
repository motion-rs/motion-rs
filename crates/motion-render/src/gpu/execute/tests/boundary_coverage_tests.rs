//! Sampled refinement at coincident, intersecting and offscreen contours.

use super::*;
use crate::gpu::visual::{self, VisualTolerance};
use motion_core::{Color, FillRule, Node, Path2D, Point, Size};

fn contour(path: &mut Path2D, points: &[[f64; 2]], reversed: bool) -> motion_core::Result {
    let mut points = points
        .iter()
        .copied()
        .map(|[x, y]| Point::new(x, y))
        .collect::<Vec<_>>();
    if reversed {
        points.reverse();
    }
    path.try_move_to(points[0])?;
    for point in &points[1..] {
        path.try_line_to(*point)?;
    }
    path.try_close()?;
    Ok(())
}

#[test]
#[ignore = "requires native GPU; overlapping contours refined before the fill rule"]
fn overlapping_contours_match_cpu_across_winding_rules() -> motion_core::Result {
    let rectangle = [[-4.25, 4.25], [35.25, 4.25], [35.25, 30.75], [-4.25, 30.75]];
    let triangle = [[2.3, 1.7], [41.2, 36.8], [-3.1, 25.4]];
    let crossing = [[0.2, 28.7], [43.5, 2.8], [39.6, 39.1]];
    let upper = [[0.25, 0.25], [35.25, 0.25], [35.25, 16.5], [0.25, 16.5]];
    let lower = [[0.25, 16.5], [35.25, 16.5], [35.25, 38.75], [0.25, 38.75]];
    let size = PixelSize::new(48, 40);
    let validation = ValidationLimits::default();
    let limits = RasterLimits::default();
    let mut gpu = Gpu::new()?;
    for rule in [FillRule::NonZero, FillRule::EvenOdd] {
        for case in 0..7 {
            let mut path = Path2D::new();
            match case {
                0 => {
                    for _ in 0..2 {
                        contour(&mut path, &rectangle, false)?;
                    }
                }
                1 => {
                    for _ in 0..3 {
                        contour(&mut path, &triangle, false)?;
                    }
                }
                2 => {
                    contour(&mut path, &triangle, false)?;
                    contour(&mut path, &triangle, true)?;
                }
                3 => {
                    contour(&mut path, &upper, false)?;
                    contour(&mut path, &lower, true)?;
                }
                4 => {
                    contour(&mut path, &triangle, false)?;
                    contour(&mut path, &crossing, false)?;
                }
                5 => {
                    contour(&mut path, &triangle, false)?;
                    contour(&mut path, &crossing, true)?;
                }
                _ => {
                    for _ in 0..41 {
                        contour(&mut path, &rectangle, false)?;
                    }
                }
            }
            let node = Node::from(path.fill(Color::WHITE).fill_rule(rule));
            let list = node
                .clone()
                .compile(Size::new(48.0, 40.0), size, validation)?;
            let reference = list.prepare(validation, limits)?.compose_linear(limits)?;
            let frame = super::device_coverage_tests::prepare_node(node, size)?;
            assert!(frame.records.alphas.is_empty());
            assert!(frame.has_device_coverage());
            let actual = gpu.render(&frame.records, Plan::new(&frame, limits)?)?;
            if reference.pixels().iter().any(|pixel| pixel[3] > 0.0) {
                let expected = reference.to_rgba8_frame(false, limits)?;
                let actual = actual.to_rgba8_frame(false, limits)?;
                visual::compare_rgba8(
                    actual.pixels(),
                    expected.pixels(),
                    48,
                    VisualTolerance::OVERLAP,
                )?;
                let replay = gpu
                    .render(&frame.records, Plan::new(&frame, limits)?)?
                    .to_rgba8_frame(false, limits)?;
                visual::compare_rgba8(
                    replay.pixels(),
                    actual.pixels(),
                    48,
                    VisualTolerance::REPLAY,
                )?;
            }
        }
    }
    Ok(())
}

#[test]
#[ignore = "requires native GPU; fixed-point area wrapping at high winding multiplicity"]
fn high_winding_interiors_remain_covered() -> motion_core::Result {
    let mut path = Path2D::new();
    for _ in 0..4096 {
        contour(
            &mut path,
            &[[-4.25, 0.25], [95.25, 0.25], [-4.25, 100.0]],
            false,
        )?;
    }
    let node = Node::from(path.fill(Color::WHITE));
    let frame = super::device_coverage_tests::prepare_node(node, PixelSize::new(112, 16))?;
    assert!(frame.has_device_coverage());
    let limits = RasterLimits::default();
    let actual = Gpu::new()?.render(&frame.records, Plan::new(&frame, limits)?)?;
    for x in [16, 32, 48, 64, 80] {
        assert_eq!(actual.pixel(x, 8).unwrap()[3], 1.0, "column {x}");
    }
    Ok(())
}
