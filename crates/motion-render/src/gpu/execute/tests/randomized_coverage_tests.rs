//! Fixed-seed crossing curves, winding rules and unrelated-frame scratch reuse.

use super::*;
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
            let device = crate::prepared::prepare_validated_device(
                &list,
                validation,
                limits,
                1.0 / 256.0,
                None,
                true,
            )?;
            assert!(
                device.alphas.is_empty(),
                "seed {seed}: unexpected CPU coverage"
            );
            assert!(!device.geometry.device_curves.is_empty());
            let expected = cpu.compose_linear(limits)?.to_rgba8_frame(false, limits)?;
            let actual = gpu
                .render(&device, Plan::new(&device, limits)?)?
                .to_rgba8_frame(false, limits)?;
            crate::gpu::visual::compare_rgba8(actual.pixels(), expected.pixels(), 0.02)
                .unwrap_or_else(|error| panic!("seed {seed}, {rule:?}: {error}"));
            for top in (0..64).step_by(16) {
                for left in (0..64).step_by(16) {
                    let region = |pixels: &[[u8; 4]]| {
                        (top..top + 16)
                            .flat_map(|y| pixels[y * 64 + left..y * 64 + left + 16].iter().copied())
                            .collect::<Vec<_>>()
                    };
                    crate::gpu::visual::compare_rgba8(
                        &region(actual.pixels()),
                        &region(expected.pixels()),
                        0.05,
                    )
                    .unwrap_or_else(|error| {
                        panic!("seed {seed}, {rule:?}, tile {left},{top}: {error}")
                    });
                }
            }
        }
    }
    Ok(())
}

#[test]
#[ignore = "requires native GPU; duplicate winding must cancel before area integration"]
fn duplicate_subpixel_contours_cancel_under_even_odd() -> motion_core::Result {
    let mut path = Path2D::new();
    for _ in 0..2 {
        path.try_move_to(Point::new(0.1, 0.1))?;
        path.try_line_to(Point::new(31.8, 0.1))?;
        path.try_line_to(Point::new(31.8, 31.8))?;
        path.try_line_to(Point::new(0.1, 31.8))?;
        path.try_close()?;
    }
    let validation = ValidationLimits::default();
    let limits = RasterLimits::default();
    let list = Node::from(path.fill(Color::WHITE).fill_rule(FillRule::EvenOdd)).compile(
        Size::new(32.0, 32.0),
        PixelSize::new(32, 32),
        validation,
    )?;
    let frame = crate::prepared::prepare_validated_device(
        &list,
        validation,
        limits,
        1.0 / 256.0,
        None,
        true,
    )?;
    assert!(frame.alphas.is_empty());
    assert_eq!(frame.geometry.device_fills.len(), 1);
    let actual = Gpu::new()?.render(&frame, Plan::new(&frame, limits)?)?;
    assert!(actual.pixels().iter().all(|pixel| *pixel == [0.0; 4]));
    Ok(())
}
