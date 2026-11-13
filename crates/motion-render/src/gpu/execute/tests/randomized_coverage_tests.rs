//! Fixed-seed crossing curves, winding rules and unrelated-frame scratch reuse.

use super::*;
use crate::gpu::visual::VisualTolerance;
use motion_core::{Color, FillRule, Node, Path2D, Point, Size};

fn path(seed: u64, rule: FillRule) -> motion_core::Result<Node> {
    let mut state = seed;
    let mut point = || {
        let mut coordinate = || {
            state ^= state << 13;
            state ^= state >> 7;
            state ^= state << 17;
            (state % 20_000) as f64 / 256.0 - 8.0
        };
        Point::new(coordinate(), coordinate())
    };
    let mut path = Path2D::new();
    for _ in 0..3 {
        path.try_move_to(point())?;
        for _ in 0..3 {
            path.try_line_to(point())?;
            path.try_quadratic_to(point(), point())?;
            path.try_cubic_to(point(), point(), point())?;
        }
        path.try_close()?;
    }
    Ok(Node::from(
        path.fill(Color::rgb(40, 180, 110).with_alpha(0.7))
            .fill_rule(rule),
    ))
}

#[test]
#[ignore = "requires native GPU; fixed-seed randomized crossing curves and winding rules"]
fn fixed_seed_random_curves_match_cpu_without_local_dropouts() -> motion_core::Result {
    let validation = ValidationLimits::default();
    let limits = RasterLimits::default();
    let size = PixelSize::new(64, 64);
    let mut gpu = Gpu::new()?;
    for seed in 1..=24 {
        for rule in [FillRule::NonZero, FillRule::EvenOdd] {
            let list =
                path(seed * 0x9e37_79b9, rule)?.compile(Size::new(64.0, 64.0), size, validation)?;
            let cpu = list.prepare(validation, limits)?;
            let device = crate::gpu::raster::prepare::prepare_validated_device(
                &list,
                validation,
                limits,
                1.0 / 256.0,
                None,
            )?;
            assert!(
                device.records.alphas.is_empty(),
                "seed {seed}: unexpected CPU coverage"
            );
            assert!(!device.geometry.device_curves.is_empty());
            let expected = cpu.compose_linear(limits)?.to_rgba8_frame(false, limits)?;
            let actual = gpu
                .render(&device.records, Plan::new(&device, limits)?)?
                .to_rgba8_frame(false, limits)?;
            crate::gpu::visual::compare_rgba8(
                actual.pixels(),
                expected.pixels(),
                64,
                VisualTolerance::OVERLAP,
            )
            .unwrap_or_else(|error| panic!("seed {seed}, {rule:?}: {error}"));
        }
    }
    Ok(())
}

fn render_device(path: Path2D, rule: FillRule) -> motion_core::Result<Vec<[f32; 4]>> {
    let validation = ValidationLimits::default();
    let limits = RasterLimits::default();
    let list = Node::from(path.fill(Color::WHITE).fill_rule(rule)).compile(
        Size::new(32.0, 32.0),
        PixelSize::new(32, 32),
        validation,
    )?;
    let frame = crate::gpu::raster::prepare::prepare_validated_device(
        &list,
        validation,
        limits,
        1.0 / 256.0,
        None,
    )?;
    assert!(frame.records.alphas.is_empty());
    assert!(!frame.geometry.device_fills.is_empty());
    let actual = Gpu::new()?.render(&frame.records, Plan::new(&frame, limits)?)?;
    Ok(actual.pixels().to_vec())
}

fn rectangle(
    path: &mut Path2D,
    [left, top, right, bottom]: [f64; 4],
    reversed: bool,
) -> motion_core::Result {
    let mut corners = [
        Point::new(left, top),
        Point::new(right, top),
        Point::new(right, bottom),
        Point::new(left, bottom),
    ];
    if reversed {
        corners.reverse();
    }
    path.try_move_to(corners[0])?;
    for corner in &corners[1..] {
        path.try_line_to(*corner)?;
    }
    path.try_close()?;
    Ok(())
}

#[test]
#[ignore = "requires native GPU; accumulated winding folds under even-odd away from contour edges"]
fn duplicate_contours_cancel_under_even_odd_away_from_edges() -> motion_core::Result {
    for top in [0.1, 0.6] {
        let mut path = Path2D::new();
        for _ in 0..2 {
            rectangle(&mut path, [0.1, top, 31.8, 31.8], false)?;
        }
        let pixels = render_device(path, FillRule::EvenOdd)?;
        assert!(
            (2..30).all(|y| (2..30).all(|x| pixels[y * 32 + x][3] < 1e-3)),
            "top {top}: duplicated contour must cancel in the interior"
        );
    }
    Ok(())
}

#[test]
#[ignore = "requires native GPU; opposite windings sharing an edge stay covered away from that edge"]
fn opposite_windings_stay_covered_away_from_their_shared_edge() -> motion_core::Result {
    for rule in [FillRule::NonZero, FillRule::EvenOdd] {
        let mut path = Path2D::new();
        rectangle(&mut path, [-5.0, 0.0, 20.0, 8.5], false)?;
        rectangle(&mut path, [-5.0, 8.5, 20.0, 17.0], true)?;
        let pixels = render_device(path, rule)?;
        for y in [4, 8, 12] {
            let pixel = pixels[y * 32 + 10];
            assert!(pixel[3] > 0.99, "{rule:?}: row {y} alpha {}", pixel[3]);
        }
    }
    Ok(())
}
