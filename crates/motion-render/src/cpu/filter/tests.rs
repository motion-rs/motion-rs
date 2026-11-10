use super::blur::{BOX_SCRATCH_BYTES, BoxSums, Lines, box_pass, slide_box};
use super::*;
use motion_core::Shape;
use motion_core::{
    Color, DrawCommand, Node, NodeModifiers, Size, ValidationErrorKind, ValidationLimits,
};

fn tile(bounds: Rect, color: Color) -> Node {
    bounds.fill(color).into()
}

fn prepared(node: &Node) -> PreparedFrame {
    node.compile(
        Size::new(8.0, 4.0),
        PixelSize::new(8, 4),
        ValidationLimits::default(),
    )
    .unwrap()
    .prepare(ValidationLimits::default(), RasterLimits::default())
    .unwrap()
}

fn render(node: &Node) -> LinearFrame {
    prepared(node)
        .render_linear(RasterLimits::default())
        .unwrap()
}

fn white() -> Node {
    tile(Rect::new(2.0, 1.0, 3.0, 2.0), Color::WHITE)
}

fn close(actual: f32, expected: f64) {
    assert!(
        (f64::from(actual) - expected).abs() < 3e-7,
        "{actual} != {expected}"
    );
}

#[test]
fn single_fill_layers_match_isolated_rounding_blends_and_budgets() {
    use motion_core::{BlendMode, BlendSpace, Point, Shape};
    for space in [BlendSpace::LinearRgb, BlendSpace::Srgb] {
        for mode in [
            BlendMode::SourceOver,
            BlendMode::Plus,
            BlendMode::Multiply,
            BlendMode::Screen,
            BlendMode::Darken,
            BlendMode::Lighten,
            BlendMode::Difference,
            BlendMode::Exclusion,
            BlendMode::Overlay,
            BlendMode::HardLight,
            BlendMode::SoftLight,
            BlendMode::ColorDodge,
            BlendMode::ColorBurn,
            BlendMode::Hue,
            BlendMode::Saturation,
            BlendMode::Color,
            BlendMode::Luminosity,
            BlendMode::LinearPlus,
        ] {
            for opacity in [0.0, 0.125, 0.5, 1.0] {
                let gradient =
                    motion_core::LinearGradient::new(Point::ORIGIN, Point::new(1.0, 1.0))
                        .stop(0.0, Color::rgba(230, 30, 100, 179))
                        .stop(1.0, Color::rgba(30, 150, 220, 77));
                let mut crossing = motion_core::Path2D::polyline([
                    Point::new(1.25, 0.5),
                    Point::new(6.75, 3.25),
                    Point::new(6.75, 0.5),
                    Point::new(1.25, 3.25),
                ])
                .unwrap();
                crossing.try_close().unwrap();
                for fill in [
                    Node::from(Rect::new(1.25, 0.5, 6.75, 3.25).fill(gradient.clone())),
                    Node::from(
                        motion_core::Circle::new(Point::new(4.0, 2.0), 1.35)
                            .unwrap()
                            .fill(gradient.clone()),
                    ),
                    Node::from(
                        crossing
                            .fill(gradient)
                            .fill_rule(motion_core::FillRule::EvenOdd),
                    ),
                ] {
                    let node = Node::group([
                        tile(Rect::new(0.0, 0.0, 8.0, 4.0), Color::rgba(50, 80, 130, 179)),
                        fill.opacity(opacity).blend_mode(mode),
                    ]);
                    let mut frame = prepared(&node);
                    frame.blend_space = space;
                    let bounds = frame.output_bounds;
                    let run = |optimize, limits| {
                        let mut stats = LinearRenderStats {
                            peak_bytes: 0,
                            work: 0,
                        };
                        let raster = PixelRect {
                            left: 0,
                            top: 0,
                            right: frame.size.width,
                            bottom: frame.size.height,
                        };
                        if optimize {
                            frame.render_linear_bounds_mode::<true>(
                                limits,
                                &mut pool::SurfacePool::default(),
                                bounds,
                                raster,
                                &mut stats,
                            )
                        } else {
                            frame.render_linear_bounds_mode::<false>(
                                limits,
                                &mut pool::SurfacePool::default(),
                                bounds,
                                raster,
                                &mut stats,
                            )
                        }
                    };
                    let reference = run(false, RasterLimits::default()).unwrap();
                    let actual = run(true, RasterLimits::default()).unwrap();
                    assert_eq!(
                        actual.pixels(),
                        reference.pixels(),
                        "{space:?} {mode:?} {opacity}"
                    );
                    let has_layer = frame.items.windows(3).any(|items| matches!(
                    items,
                    [PreparedItem::PushLayer { layer }, PreparedItem::Fill { .. }, PreparedItem::PopLayer]
                        if layer.filters.is_none() && layer.alpha_clip.is_none() && layer.geometric_clip.is_none()
                ));
                    if has_layer {
                        assert!(actual.stats().peak_bytes < reference.stats().peak_bytes);
                    } else {
                        assert_eq!(actual.stats().peak_bytes, reference.stats().peak_bytes);
                    }
                    assert!(actual.stats().work <= reference.stats().work);
                    let exact = RasterLimits {
                        max_work: actual.stats().work,
                        max_bytes: actual.stats().peak_bytes,
                        ..RasterLimits::default()
                    };
                    assert_eq!(run(true, exact).unwrap().pixels(), actual.pixels());
                    for limits in [
                        RasterLimits {
                            max_work: exact.max_work - 1,
                            ..exact
                        },
                        RasterLimits {
                            max_bytes: exact.max_bytes - 1,
                            ..exact
                        },
                    ] {
                        assert!(run(true, limits).is_err());
                    }
                }
            }
        }
    }
}

#[test]
fn plus_capture_bounds_match_wide_layers_and_preserve_effect_boundaries() {
    check_plus_capture_bounds(|_, _| {});
}

#[cfg(feature = "gpu")]
#[test]
#[ignore = "requires separate authorization to initialize a native GPU device"]
fn gpu_plus_capture_bounds_preserve_pixels_and_replay() {
    let mut gpu = crate::gpu::Gpu::new().unwrap();
    let mut cases = 0;
    let mut max_difference = 0;
    check_plus_capture_bounds(|frame, wide| {
        let limits = RasterLimits::default();
        let actual = gpu
            .render(frame, crate::gpu::Plan::new(frame, limits).unwrap())
            .unwrap();
        let reference = gpu
            .render(wide, crate::gpu::Plan::new(wide, limits).unwrap())
            .unwrap();
        assert_eq!(actual.pixels(), reference.pixels());
        let cpu = frame
            .render_linear(limits)
            .unwrap()
            .to_srgb_rgba8(false, limits)
            .unwrap();
        let output = actual.to_srgb_rgba8(false, limits).unwrap();
        let mut large_difference = 0;
        for (actual, expected) in output.pixels().iter().zip(cpu.pixels()) {
            let difference = actual
                .iter()
                .zip(expected)
                .map(|(actual, expected)| actual.abs_diff(*expected))
                .max()
                .unwrap();
            assert!(difference <= 2);
            max_difference = max_difference.max(difference);
            large_difference += usize::from(difference > 1);
        }
        assert!((large_difference as f64) / (output.pixels().len() as f64) < 0.001);
        let replay = gpu
            .render(frame, crate::gpu::Plan::new(frame, limits).unwrap())
            .unwrap();
        assert_eq!(actual.pixels(), replay.pixels());
        cases += 1;
    });
    println!(
        "plus_capture_parity cases={cases} max_rgba_difference={max_difference} wide=exact replay=exact"
    );
}

fn check_plus_capture_bounds(mut check: impl FnMut(&PreparedFrame, &PreparedFrame)) {
    use motion_core::{BlendMode, BlendSpace, MaskMode, Path2D};
    let content = Node::group([
        tile(
            Rect::new(5.25, 4.5, 8.75, 7.25),
            Color::rgba(220, 50, 30, 179),
        ),
        tile(
            Rect::new(7.5, 6.5, 10.25, 9.75),
            Color::rgba(30, 100, 240, 128),
        ),
    ]);
    let clipped = content
        .clone()
        .clip(Path2D::from_rect(Rect::new(6.25, 5.5, 9.25, 8.75)).unwrap());
    let masked = content.clone().mask(
        tile(Rect::new(6.25, 5.5, 9.25, 8.75), Color::WHITE),
        MaskMode::InvertedAlpha,
    );
    for mode in [BlendMode::Plus, BlendMode::LinearPlus] {
        for (node, narrower) in [
            (content.clone(), true),
            (Node::group([content.clone().opacity(0.6)]), true),
            (
                Node::group([content.clone().filter(Filter::Blur { sigma: 2.0 })]),
                true,
            ),
            (clipped.clone(), false),
            (masked.clone(), false),
        ] {
            let node = Node::group([
                tile(
                    Rect::new(0.0, 0.0, 32.0, 16.0),
                    Color::rgba(30, 50, 90, 179),
                ),
                node.opacity(0.5).blend_mode(mode),
            ]);
            let mut source = node
                .compile(
                    Size::new(32.0, 16.0),
                    PixelSize::new(32, 16),
                    ValidationLimits::default(),
                )
                .unwrap()
                .display_list()
                .clone();
            for command in &mut source.commands {
                match command {
                    DrawCommand::PushLayer {
                        bounds, blend_mode, ..
                    }
                    | DrawCommand::PushRectLayer {
                        bounds, blend_mode, ..
                    }
                    | DrawCommand::PushShapeLayer {
                        bounds, blend_mode, ..
                    } if *blend_mode == mode => *bounds = Rect::new(0.0, 0.0, 32.0, 16.0),
                    _ => {}
                }
            }
            let actual = source
                .prepare(ValidationLimits::default(), RasterLimits::default())
                .unwrap();
            let mut reference = actual.clone();
            let mut reduced = false;
            for item in &mut reference.items {
                if let PreparedItem::PushLayer { layer } = item
                    && layer.blend_mode == mode
                {
                    reduced |= layer.bounds.pixels() < actual.output_bounds.pixels();
                    layer.bounds = actual.output_bounds;
                }
            }
            assert_eq!(reduced, narrower);
            let limits = RasterLimits {
                max_work: actual.stats.work,
                max_bytes: actual.stats.peak_bytes,
                ..RasterLimits::default()
            };
            source.prepare(ValidationLimits::default(), limits).unwrap();
            for short in [
                RasterLimits {
                    max_work: limits.max_work - 1,
                    ..limits
                },
                RasterLimits {
                    max_bytes: limits.max_bytes - 1,
                    ..limits
                },
            ] {
                assert!(source.prepare(ValidationLimits::default(), short).is_err());
            }
            for space in [BlendSpace::LinearRgb, BlendSpace::Srgb] {
                let mut actual = actual.clone();
                let mut reference = reference.clone();
                actual.blend_space = space;
                reference.blend_space = space;
                check(&actual, &reference);
                let actual = actual.render_linear(RasterLimits::default()).unwrap();
                let reference = reference.render_linear(RasterLimits::default()).unwrap();
                assert_eq!(actual.pixels(), reference.pixels());
                assert!(actual.stats().peak_bytes <= reference.stats().peak_bytes);
                assert!(actual.stats().work <= reference.stats().work);
                if narrower {
                    assert!(actual.stats().work < reference.stats().work);
                }
            }
        }
    }
}

fn matrix() -> [f64; 20] {
    [
        1.0, 0.0, 0.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 0.0, 0.0,
        1.0, 0.0,
    ]
}

#[test]
fn rolling_box_matches_scalar_taps_at_clipped_and_offset_edges() {
    for bounds in [
        PixelRect {
            left: 0,
            top: 0,
            right: 17,
            bottom: 9,
        },
        PixelRect {
            left: 3,
            top: 5,
            right: 20,
            bottom: 14,
        },
        PixelRect {
            left: 2,
            top: 3,
            right: 3,
            bottom: 4,
        },
    ] {
        for radius in [1, 2, 4, 9, 14, 50] {
            for horizontal in [true, false] {
                let size = PixelSize::new(bounds.right, bounds.bottom);
                let mut budget = Budget::new(size, RasterLimits::default()).unwrap();
                let mut actual = Surface::new(
                    bounds,
                    1.0,
                    motion_core::drawing::BlendMode::SourceOver,
                    [0.0; 4],
                    &mut budget,
                    "test",
                )
                .unwrap();
                for (index, pixel) in actual.pixels.iter_mut().enumerate() {
                    let alpha = ((index * 37 % 101) as f32 + 1.0) / 102.0;
                    *pixel = [alpha * 2.0, alpha * -0.5, alpha * 0.25, alpha];
                }
                let mut reference_budget = Budget::new(size, RasterLimits::default()).unwrap();
                let mut reference = Surface::new(
                    bounds,
                    1.0,
                    motion_core::drawing::BlendMode::SourceOver,
                    [0.0; 4],
                    &mut reference_budget,
                    "test",
                )
                .unwrap();
                reference.pixels.clone_from(&actual.pixels);
                let domain = Rect::new(
                    f64::from(bounds.left),
                    f64::from(bounds.top),
                    f64::from(bounds.right),
                    f64::from(bounds.bottom),
                );
                let weight = 1.0 / (radius * 2 + 1) as f64;
                pass(
                    &mut reference,
                    domain,
                    &mut reference_budget,
                    "test",
                    |column, row, surface| {
                        let mut sum = [0.0; 4];
                        for offset in -(radius as i64)..=radius as i64 {
                            let pixel = sample(
                                surface,
                                f64::from(column) + if horizontal { offset as f64 } else { 0.0 },
                                f64::from(row) + if horizontal { 0.0 } else { offset as f64 },
                            );
                            for channel in 0..4 {
                                sum[channel] += pixel[channel] * weight;
                            }
                        }
                        Ok(sum)
                    },
                )
                .unwrap();
                box_pass(&mut actual, domain, radius, horizontal, &mut budget, "test").unwrap();
                for (actual, reference) in actual.pixels.iter().zip(&reference.pixels) {
                    for channel in 0..4 {
                        close(actual[channel], f64::from(reference[channel]));
                    }
                }
                assert_eq!(
                    budget.live_bytes(),
                    bounds.pixels() * size_of::<[f32; 4]>() as u64
                );
            }
        }
    }
}

#[test]
fn rolling_gaussian_matches_scalar_for_quality_sigma_and_domains() {
    let bounds = PixelRect {
        left: 3,
        top: 2,
        right: 20,
        bottom: 11,
    };
    let size = PixelSize::new(bounds.right, bounds.bottom);
    for sigma in [
        0.0,
        0.1,
        0.5,
        (1.0_f64 / 3.0).sqrt(),
        0.6,
        1.0,
        2.0,
        5.0,
        9.0,
        14.0,
    ] {
        for quality in [
            motion_core::RenderQuality::Draft,
            motion_core::RenderQuality::Final,
        ] {
            for domain in [
                Rect::new(3.0, 2.0, 20.0, 11.0),
                Rect::new(5.25, 3.75, 17.5, 9.25),
            ] {
                let mut budget = Budget::new(size, RasterLimits::default()).unwrap();
                let mut actual = Surface::new(
                    bounds,
                    1.0,
                    motion_core::drawing::BlendMode::SourceOver,
                    [0.0; 4],
                    &mut budget,
                    "test",
                )
                .unwrap();
                for (index, pixel) in actual.pixels.iter_mut().enumerate() {
                    let alpha = (index * 37 % 101) as f32 / 102.0;
                    *pixel = [alpha * 2.0, alpha * -0.5, alpha * 0.25, alpha];
                }
                let mut reference_budget = Budget::new(size, RasterLimits::default()).unwrap();
                let mut reference = Surface::new(
                    bounds,
                    1.0,
                    motion_core::drawing::BlendMode::SourceOver,
                    [0.0; 4],
                    &mut reference_budget,
                    "test",
                )
                .unwrap();
                reference.pixels.clone_from(&actual.pixels);
                let mut working = domain;
                for radius in motion_core::filter::quality_box_radii(sigma, quality).unwrap() {
                    if radius == 0 {
                        continue;
                    }
                    let radius = radius as usize;
                    let next = Rect::new(
                        working.left - radius as f64,
                        working.top - radius as f64,
                        working.right + radius as f64,
                        working.bottom + radius as f64,
                    );
                    for horizontal in [true, false] {
                        let target = if horizontal {
                            Rect::new(next.left, working.top, next.right, working.bottom)
                        } else {
                            next
                        };
                        pass(
                            &mut reference,
                            target,
                            &mut reference_budget,
                            "test",
                            |column, row, surface| {
                                let mut pixel = [0.0; 4];
                                for tap in 0..radius * 2 + 1 {
                                    let weight =
                                        motion_core::filter::box_weight(sigma, radius, tap);
                                    let offset = tap as f64 - radius as f64;
                                    let sampled = sample(
                                        surface,
                                        f64::from(column) + if horizontal { offset } else { 0.0 },
                                        f64::from(row) + if horizontal { 0.0 } else { offset },
                                    );
                                    for channel in 0..4 {
                                        pixel[channel] += sampled[channel] * weight;
                                    }
                                }
                                Ok(pixel)
                            },
                        )
                        .unwrap();
                    }
                    working = next;
                }
                let output =
                    gaussian(&mut actual, domain, sigma, &mut budget, "test", quality).unwrap();
                assert_eq!(
                    output,
                    FilterOperation::Blur { sigma }.bounds(domain, domain)
                );
                for (actual, reference) in actual.pixels.iter().zip(&reference.pixels) {
                    for channel in 0..4 {
                        close(actual[channel], f64::from(reference[channel]));
                    }
                }
                let once = actual.pixels.clone();
                let mut replay_budget = Budget::new(size, RasterLimits::default()).unwrap();
                let mut replay = Surface::new(
                    bounds,
                    1.0,
                    motion_core::drawing::BlendMode::SourceOver,
                    [0.0; 4],
                    &mut replay_budget,
                    "test",
                )
                .unwrap();
                for (index, pixel) in replay.pixels.iter_mut().enumerate() {
                    let alpha = (index * 37 % 101) as f32 / 102.0;
                    *pixel = [alpha * 2.0, alpha * -0.5, alpha * 0.25, alpha];
                }
                gaussian(
                    &mut replay,
                    domain,
                    sigma,
                    &mut replay_budget,
                    "test",
                    quality,
                )
                .unwrap();
                assert_eq!(once, replay.pixels);
            }
        }
    }
}

#[test]
fn gaussian_work_and_scratch_have_exact_bounds() {
    let bounds = PixelRect {
        left: 0,
        top: 0,
        right: 8,
        bottom: 4,
    };
    let size = PixelSize::new(8, 4);
    let bytes = bounds.pixels() * size_of::<[f32; 4]>() as u64;
    let run = |sigma: f64, limits: RasterLimits| {
        let mut budget = Budget::new(size, limits).unwrap();
        let mut surface = Surface::new(
            bounds,
            1.0,
            motion_core::drawing::BlendMode::SourceOver,
            [1.0; 4],
            &mut budget,
            "test",
        )
        .unwrap();
        let result = gaussian(
            &mut surface,
            Rect::new(0.0, 0.0, 8.0, 4.0),
            sigma,
            &mut budget,
            "test",
            motion_core::RenderQuality::Final,
        );
        (result, budget)
    };
    for sigma in [2.0, 5.0, 9.0, 14.0, 1000.0] {
        let work = bounds.pixels() * if sigma == 2.0 { 29 } else { 25 };
        let (result, measured) = run(sigma, RasterLimits::default());
        result.unwrap();
        assert_eq!(measured.work_done(), work);
        assert_eq!(measured.live_bytes(), bytes);
        // Two surfaces, plus the sliding-sum table of boxes wider than five samples.
        let peak = measured.peak_bytes();
        assert!(peak >= bytes * 2);
        if sigma == 2.0 {
            assert_eq!(peak, bytes * 2);
        } else {
            assert!(peak - bytes * 2 <= BOX_SCRATCH_BYTES as u64 * 2);
        }
        for (max_work, max_bytes, succeeds) in [
            (work, peak, true),
            (work - 1, peak, false),
            (work, peak - 1, false),
        ] {
            let (result, budget) = run(
                sigma,
                RasterLimits {
                    max_work,
                    max_bytes,
                    ..RasterLimits::default()
                },
            );
            assert_eq!(result.is_ok(), succeeds);
            if succeeds {
                assert_eq!(budget.work_done(), work);
                assert_eq!(budget.peak_bytes(), peak);
                assert_eq!(budget.live_bytes(), bytes);
            } else {
                assert!(matches!(
                    result.unwrap_err().kind,
                    ValidationErrorKind::StructuralLimit
                ));
            }
        }
    }
}

/// `checked` as it was written before the finiteness tests were made branch-free.
fn checked_reference(pixel: [f64; 4], path: &str) -> Result<[f32; 4], ValidationError> {
    if pixel.iter().any(|value| !value.is_finite()) {
        return Err(ValidationError::invalid(
            path,
            "filters",
            "Filter arithmetic must remain finite",
        ));
    }
    let pixel = if pixel[3] == 0.0 { [0.0; 4] } else { pixel };
    let result = pixel.map(|value| value as f32);
    if pixel.iter().any(|value| !value.is_finite()) || result.iter().any(|value| !value.is_finite())
    {
        return Err(ValidationError::invalid(
            path,
            "filters",
            "Filter working color must remain finite and representable",
        ));
    }
    Ok(result)
}

#[test]
fn checked_matches_its_stepwise_definition() {
    let values = [
        0.0,
        -0.0,
        1.0,
        -2.5,
        1e-40,
        3.4e38,
        3.5e38,
        -3.5e38,
        1e300,
        f64::INFINITY,
        f64::NEG_INFINITY,
        f64::NAN,
    ];
    for a in values {
        for b in values {
            for alpha in values {
                let pixel = [a, b, a, alpha];
                assert_eq!(
                    checked(pixel, "p").map(|pixel| pixel.map(f32::to_bits)),
                    checked_reference(pixel, "p").map(|pixel| pixel.map(f32::to_bits)),
                    "{pixel:?}"
                );
                let message = |result: Result<[f32; 4], ValidationError>| {
                    result.err().map(|error| error.message.clone())
                };
                assert_eq!(
                    message(checked(pixel, "p")),
                    message(checked_reference(pixel, "p"))
                );
            }
        }
    }
}

/// `slide_box` as it was before clear blocks were skipped and whole in-line blocks took a
/// contiguous path: every block is summed through `Lines::element`.
fn slide_box_reference(
    lines: &Lines<'_>,
    radius: i64,
    outputs: std::ops::Range<i64>,
    weight: f64,
    sums: &mut BoxSums,
    destination: &mut [[f32; 4]],
    path: &str,
) -> Result<(), ValidationError> {
    let width = radius * 2 + 1;
    let lanes = lines.lanes;
    let mut base = 0;
    for position in outputs.clone() {
        if position == outputs.start || position % width == 0 {
            base = position;
            let end = (position / width + 1) * width;
            sums.running[..lanes].fill([0.0; 4]);
            for step in (0..(end - position) as usize).rev() {
                for lane in 0..lanes {
                    let value = lines.element(position + step as i64 - radius, lane);
                    let running = &mut sums.running[lane];
                    for (running, value) in running.iter_mut().zip(value) {
                        *running += value;
                    }
                    sums.suffix[step * lanes + lane] = *running;
                }
            }
            sums.prefix[..lanes].fill([0.0; 4]);
            for padded in end..position + width {
                for lane in 0..lanes {
                    let value = lines.element(padded - radius, lane);
                    for (prefix, value) in sums.prefix[lane].iter_mut().zip(value) {
                        *prefix += value;
                    }
                }
            }
        } else {
            for lane in 0..lanes {
                let value = lines.element(position + radius, lane);
                for (prefix, value) in sums.prefix[lane].iter_mut().zip(value) {
                    *prefix += value;
                }
            }
        }
        let suffix = &sums.suffix[(position - base) as usize * lanes..][..lanes];
        for lane in 0..lanes {
            let mut mean = [0.0; 4];
            for channel in 0..4 {
                mean[channel] = (suffix[lane][channel] + sums.prefix[lane][channel]) * weight;
            }
            destination[lines.origin + position as usize * lines.stride + lane] =
                checked_reference(mean, path)?;
        }
    }
    Ok(())
}

#[test]
fn slide_box_is_bit_identical_to_summing_every_block_including_sparse_lines() {
    let (width, height) = (83_usize, 41_usize);
    let mut state = 0x2468_ace1_u32;
    let mut next = move || {
        state = state.wrapping_mul(1664525).wrapping_add(1013904223);
        (state >> 8) as f32 / 16777215.0
    };
    // Dense noise, stripes of content between clear runs (with negative zeros and hidden
    // color inside the clear runs), a single lit pixel, nothing at all, and one NaN.
    let mut sources = Vec::new();
    let dense = (0..width * height)
        .map(|_| {
            let alpha = next() * 0.9 + 0.05;
            [alpha * next(), alpha * next(), alpha * next(), alpha]
        })
        .collect::<Vec<_>>();
    let stripes = (0..width * height)
        .map(|index| {
            let (column, row) = (index % width, index / width);
            if (column / 11) % 3 == 0 && (row / 7) % 2 == 0 {
                [0.2 * next(), 0.4, 0.6, 0.5 + next() * 0.5]
            } else if column % 17 == 5 {
                [-0.0, 0.0, -0.0, 0.0]
            } else if column % 23 == 9 {
                [0.3, 0.1, 0.2, 0.0]
            } else {
                [0.0; 4]
            }
        })
        .collect::<Vec<_>>();
    let mut lit = vec![[0.0; 4]; width * height];
    lit[20 * width + 41] = [1.0, 2.0, 3.0, 1.0];
    let mut poisoned = stripes.clone();
    poisoned[15 * width + 30] = [f32::NAN, 0.0, 0.0, 1.0];
    sources.extend([
        dense,
        stripes,
        lit,
        vec![[0.0; 4]; width * height],
        poisoned,
    ]);
    for source in &sources {
        for radius in [3_i64, 4, 9, 20, 45] {
            let box_width = radius * 2 + 1;
            let weight = 1.0 / box_width as f64;
            let run = |reference: bool, lines: &Lines<'_>, outputs: std::ops::Range<i64>| {
                let lanes = lines.lanes;
                let mut sums = BoxSums {
                    suffix: vec![[0.0; 4]; box_width as usize * lanes + 2 * lanes],
                    prefix: vec![[0.0; 4]; lanes],
                    running: vec![[0.0; 4]; lanes],
                };
                let mut destination = vec![[7.0_f32; 4]; width * height];
                let result = if reference {
                    slide_box_reference(
                        lines,
                        radius,
                        outputs,
                        weight,
                        &mut sums,
                        &mut destination,
                        "t",
                    )
                } else {
                    slide_box(
                        lines,
                        radius,
                        outputs,
                        weight,
                        &mut sums,
                        &mut destination,
                        "t",
                    )
                };
                (result.is_ok(), destination)
            };
            let bits = |pixels: &[[f32; 4]]| -> Vec<[u32; 4]> {
                pixels.iter().map(|pixel| pixel.map(f32::to_bits)).collect()
            };
            for row in 0..height {
                let lines = Lines {
                    source,
                    origin: row * width,
                    stride: 1,
                    lanes: 1,
                    length: width as i64,
                };
                for outputs in [0..width as i64, 3..width as i64 - 5, 17..18] {
                    let (reference_ok, reference) = run(true, &lines, outputs.clone());
                    let (actual_ok, actual) = run(false, &lines, outputs.clone());
                    assert_eq!(actual_ok, reference_ok, "radius {radius} row {row}");
                    if reference_ok {
                        assert_eq!(
                            bits(&actual),
                            bits(&reference),
                            "horizontal radius {radius} row {row} outputs {outputs:?}"
                        );
                    }
                }
            }
            for lanes in [1, 5, width] {
                for start in (0..width).step_by(lanes) {
                    let lines = Lines {
                        source,
                        origin: start,
                        stride: width,
                        lanes: lanes.min(width - start),
                        length: height as i64,
                    };
                    for outputs in [0..height as i64, 2..height as i64 - 3] {
                        let (reference_ok, reference) = run(true, &lines, outputs.clone());
                        let (actual_ok, actual) = run(false, &lines, outputs.clone());
                        assert_eq!(actual_ok, reference_ok, "radius {radius} start {start}");
                        if reference_ok {
                            assert_eq!(
                                bits(&actual),
                                bits(&reference),
                                "vertical radius {radius} lanes {lanes} start {start}"
                            );
                        }
                    }
                }
            }
        }
    }
}

#[test]
fn slide_box_places_a_single_lit_sample_exactly_like_the_block_sums() {
    // A lone sample at every position meets every block edge, which is where a skipped
    // block's window can end exactly on it.
    let length = 61_usize;
    for radius in [3_i64, 4, 9] {
        let box_width = radius * 2 + 1;
        let weight = 1.0 / box_width as f64;
        for lit in 0..length {
            let mut source = vec![[0.0_f32; 4]; length];
            source[lit] = [0.5, 0.25, 0.125, 1.0];
            let lines = Lines {
                source: &source,
                origin: 0,
                stride: 1,
                lanes: 1,
                length: length as i64,
            };
            for outputs in [0..length as i64, 5..length as i64 - 2] {
                let run = |reference: bool| {
                    let mut sums = BoxSums {
                        suffix: vec![[0.0; 4]; box_width as usize + 2],
                        prefix: vec![[0.0; 4]; 1],
                        running: vec![[0.0; 4]; 1],
                    };
                    let mut destination = vec![[7.0_f32; 4]; length];
                    let call = if reference {
                        slide_box_reference
                    } else {
                        slide_box
                    };
                    call(
                        &lines,
                        radius,
                        outputs.clone(),
                        weight,
                        &mut sums,
                        &mut destination,
                        "t",
                    )
                    .unwrap();
                    destination
                        .iter()
                        .map(|pixel| pixel.map(f32::to_bits))
                        .collect::<Vec<_>>()
                };
                assert_eq!(
                    run(false),
                    run(true),
                    "radius {radius} lit {lit} outputs {outputs:?}"
                );
            }
        }
    }
}

/// `channels` as it was written on top of `pass` and four-tap `sample`.
fn channels_reference(
    surface: &mut Surface,
    domain: Rect,
    offsets: [Vector2; 3],
    budget: &mut Budget,
    path: &str,
) -> Result<Rect, ValidationError> {
    let output = FilterOperation::ChannelOffset {
        red: offsets[0],
        green: offsets[1],
        blue: offsets[2],
    }
    .bounds(domain, domain);
    admitted(surface, 12, budget, path)?;
    pass(surface, output, budget, path, |column, row, surface| {
        let mut pixel = [0.0_f64; 4];
        for channel in 0..3 {
            let sampled = sample(
                surface,
                f64::from(column) - offsets[channel].x,
                f64::from(row) - offsets[channel].y,
            );
            pixel[channel] = sampled[channel];
            pixel[3] = pixel[3].max(sampled[3]);
        }
        Ok(pixel)
    })?;
    Ok(output)
}

#[test]
fn channel_offsets_are_bit_identical_to_four_tap_sampling() {
    let bounds = PixelRect {
        left: 3,
        top: 2,
        right: 44,
        bottom: 21,
    };
    let size = PixelSize::new(64, 32);
    let mut state = 0x0bad_cafe_u32;
    let mut next = move || {
        state = state.wrapping_mul(1664525).wrapping_add(1013904223);
        (state >> 8) as f32 / 16777215.0
    };
    let pixels = (0..bounds.pixels() as usize)
        .map(|index| match index % 7 {
            0 => [0.0; 4],
            1 => [0.4, 0.2, 0.1, 0.0],
            _ => {
                let alpha = 0.05 + next() * 0.95;
                [alpha * next(), alpha * next(), alpha * next(), alpha]
            }
        })
        .collect::<Vec<_>>();
    let cases: [[(f64, f64); 3]; 7] = [
        [(0.9, 0.0), (0.0, 0.0), (-0.9, 0.0)],
        [(0.3, -1.7), (2.25, 0.5), (-3.0, 0.0)],
        [(0.0, 0.0); 3],
        [(1.0, 0.0), (0.0, 0.0), (-1.0, 0.0)],
        [(0.5, 0.5); 3],
        [(100.0, 0.0), (0.0, -100.0), (1e3, 1e3)],
        [(-0.25, 0.75), (0.0, 1.0), (1.5, 0.0)],
    ];
    for domain in [
        Rect::new(5.0, 4.0, 36.0, 15.0),
        Rect::new(0.0, 0.0, 64.0, 32.0),
    ] {
        for case in cases {
            let offsets = case.map(|(x, y)| Vector2::new(x, y));
            let run = |reference: bool| {
                let mut budget = Budget::new(size, RasterLimits::default()).unwrap();
                let mut surface = Surface::new(
                    bounds,
                    1.0,
                    motion_core::drawing::BlendMode::SourceOver,
                    [0.0; 4],
                    &mut budget,
                    "test",
                )
                .unwrap();
                surface.pixels.clone_from(&pixels);
                let output = if reference {
                    channels_reference(&mut surface, domain, offsets, &mut budget, "test")
                } else {
                    channels(&mut surface, domain, offsets, &mut budget, "test")
                }
                .unwrap();
                let bits = surface
                    .pixels
                    .iter()
                    .map(|pixel| pixel.map(f32::to_bits))
                    .collect::<Vec<_>>();
                (
                    output,
                    bits,
                    budget.work_done(),
                    budget.live_bytes(),
                    budget.peak_bytes(),
                )
            };
            assert_eq!(run(false), run(true), "{case:?} {domain:?}");
        }
    }
}

#[test]
fn sliding_boxes_match_naive_windows_across_lane_chunks_and_partial_domains() {
    let bounds = PixelRect {
        left: 3,
        top: 2,
        right: 56,
        bottom: 9,
    };
    let size = PixelSize::new(64, 16);
    let value = |column: u32, row: u32, channel: u32| {
        let seed = column * 31 + row * 17 + channel * 7;
        ((seed.wrapping_mul(2654435761u32) % 1000) as f32 - 300.0)
            * if seed.is_multiple_of(11) { 64.0 } else { 0.01 }
    };
    // 700 makes the table wider than one lane chunk; 40 and 3 stay inside it.
    for radius in [3_usize, 4, 7, 40, 700] {
        for horizontal in [true, false] {
            let mut budget = Budget::new(size, RasterLimits::default()).unwrap();
            let mut surface = Surface::new(
                bounds,
                1.0,
                motion_core::drawing::BlendMode::SourceOver,
                [0.0; 4],
                &mut budget,
                "test",
            )
            .unwrap();
            for row in bounds.top..bounds.bottom {
                for column in bounds.left..bounds.right {
                    let index = surface.index(column, row);
                    surface.pixels[index] = [0, 1, 2, 3].map(|c| value(column, row, c));
                }
            }
            let source = surface.pixels.clone();
            let domain = Rect::new(7.0, 4.0, 50.0, 8.0);
            box_pass(
                &mut surface,
                domain,
                radius,
                horizontal,
                &mut budget,
                "test",
            )
            .unwrap();
            let weight = 1.0 / (radius * 2 + 1) as f64;
            let inner = PixelRect {
                left: 7,
                top: 4,
                right: 50,
                bottom: 8,
            };
            for row in bounds.top..bounds.bottom {
                for column in bounds.left..bounds.right {
                    let actual = surface.pixels[surface.index(column, row)];
                    let inside = column >= inner.left
                        && column < inner.right
                        && row >= inner.top
                        && row < inner.bottom;
                    for channel in 0..4 {
                        let mut sum = 0.0;
                        for offset in -(radius as i64)..=radius as i64 {
                            let (c, r) = if horizontal {
                                (i64::from(column) + offset, i64::from(row))
                            } else {
                                (i64::from(column), i64::from(row) + offset)
                            };
                            if c >= i64::from(bounds.left)
                                && c < i64::from(bounds.right)
                                && r >= i64::from(bounds.top)
                                && r < i64::from(bounds.bottom)
                            {
                                let index = ((r - i64::from(bounds.top))
                                    * i64::from(bounds.right - bounds.left)
                                    + c
                                    - i64::from(bounds.left))
                                    as usize;
                                sum += f64::from(source[index][channel]);
                            }
                        }
                        let expected = if inside { sum * weight } else { 0.0 };
                        assert!(
                            (f64::from(actual[channel]) - expected).abs()
                                <= expected.abs() * 1e-6 + 1e-5,
                            "radius {radius} horizontal {horizontal} at ({column}, {row}) channel {channel}: {} against {expected}",
                            actual[channel]
                        );
                    }
                }
            }
            assert_eq!(budget.live_bytes(), bounds.pixels() * 16);
        }
    }
}

#[test]
fn rolling_box_preserves_small_hdr_values_after_large_values_leave() {
    for radius in [1, 3, 5] {
        let size = PixelSize::new(16, 1);
        let mut budget = Budget::new(size, RasterLimits::default()).unwrap();
        let mut surface = Surface::new(
            PixelRect {
                left: 0,
                top: 0,
                right: 16,
                bottom: 1,
            },
            1.0,
            motion_core::drawing::BlendMode::SourceOver,
            [1.0; 4],
            &mut budget,
            "test",
        )
        .unwrap();
        surface.pixels[0][0] = 2.0_f32.powi(120);
        surface.pixels[1][0] = 2.0_f32.powi(60);
        box_pass(
            &mut surface,
            Rect::new(0.0, 0.0, 16.0, 1.0),
            radius,
            true,
            &mut budget,
            "test",
        )
        .unwrap();
        for pixel in &surface.pixels[(radius + 2)..(16 - radius)] {
            close(pixel[0], 1.0);
        }
    }
}

#[test]
fn rolling_box_rejects_nonfinite_pixels() {
    for invalid in [f32::NAN, f32::INFINITY, f32::NEG_INFINITY] {
        let size = PixelSize::new(2, 1);
        let mut budget = Budget::new(size, RasterLimits::default()).unwrap();
        let mut surface = Surface::new(
            PixelRect {
                left: 0,
                top: 0,
                right: 2,
                bottom: 1,
            },
            1.0,
            motion_core::drawing::BlendMode::SourceOver,
            [1.0; 4],
            &mut budget,
            "test",
        )
        .unwrap();
        surface.pixels[0][0] = invalid;
        assert!(
            box_pass(
                &mut surface,
                Rect::new(0.0, 0.0, 2.0, 1.0),
                1,
                true,
                &mut budget,
                "test",
            )
            .is_err()
        );
    }
}

#[test]
fn draft_filter_uses_one_box_and_final_retains_original_kernel() {
    let final_radii =
        motion_core::filter::quality_box_radii(2.0, motion_core::RenderQuality::Final).unwrap();
    assert_eq!(final_radii, motion_core::filter::box_radii(2.0).unwrap());
    let draft =
        motion_core::filter::quality_box_radii(2.0, motion_core::RenderQuality::Draft).unwrap();
    assert_eq!(draft, [0, 0, 3]);
    let frame = prepared(&white().filter(Filter::Blur { sigma: 2.0 }));
    assert_eq!(frame.quality, motion_core::RenderQuality::Final);
    frame
        .quality(motion_core::RenderQuality::Draft)
        .render_linear(RasterLimits::default())
        .unwrap();
}

#[test]
fn three_box_blur_matches_independent_impulse_and_preserves_hdr() {
    let color = Color::from_rgb_float(2.0, -0.5, 0.25).with_alpha(0.5);
    let output =
        render(&tile(Rect::new(3.0, 1.0, 4.0, 2.0), color).filter(Filter::Blur { sigma: 1.0 }));
    assert_eq!(motion_core::filter::box_radii(1.0).unwrap(), [0, 0, 1]);
    let color = color.to_linear_premultiplied();
    for row in 0..4 {
        for column in 0..8 {
            let horizontal = i64::from(column) - 3;
            let vertical = i64::from(row) - 1;
            let factor = if horizontal.abs() <= 1 && vertical.abs() <= 1 {
                1.0 / 9.0
            } else {
                0.0
            };
            for (channel, component) in color.iter().enumerate() {
                close(
                    output.pixel(column, row).unwrap()[channel],
                    component * factor,
                );
            }
        }
    }
}

#[test]
fn three_boxes_have_independent_sigma_two_coefficients() {
    assert_eq!(motion_core::filter::box_radii(2.0).unwrap(), [1, 1, 2]);
    let output = render(
        &tile(Rect::new(3.0, 1.0, 4.0, 2.0), Color::WHITE).filter(Filter::Blur { sigma: 2.0 }),
    );
    let coefficient = |offset: i64| {
        let mut count = 0;
        for first in -1..=1 {
            for second in -1..=1 {
                for third in -2..=2 {
                    if first + second + third == offset {
                        count += 1;
                    }
                }
            }
        }
        f64::from(count) / 45.0
    };
    for row in 0..4 {
        for column in 0..8 {
            close(
                output.pixel(column, row).unwrap()[3],
                coefficient(i64::from(column) - 3) * coefficient(i64::from(row) - 1),
            );
        }
    }
}

#[test]
fn fractional_offset_keeps_the_entire_quantized_input_support() {
    let node = tile(Rect::new(0.9, 1.0, 1.1, 2.0), Color::WHITE).filter(Filter::Custom(vec![
        FilterOperation::Offset { x: 0.5, y: 0.0 },
    ]));
    let output = render(&node);
    close(output.pixel(0, 1).unwrap()[3], 13.0 / 255.0);
    close(output.pixel(1, 1).unwrap()[3], 26.0 / 255.0);
    close(output.pixel(2, 1).unwrap()[3], 13.0 / 255.0);
}

#[test]
fn shadow_uses_alpha_color_and_completed_group_then_opacity() {
    let source = white().opacity(0.5);
    let output = render(
        &Node::group([source.clone(), source])
            .filter(Filter::DropShadow {
                offset: Vector2::new(2.0, 0.0),
                sigma: 0.0,
                color: Color::from_hex(0xff0000).with_alpha(0.5),
            })
            .opacity(0.5),
    );
    assert_eq!(output.pixel(2, 1), Some([0.375; 4]));
    assert_eq!(output.pixel(4, 1), Some([0.1875, 0.0, 0.0, 0.1875]));
}

#[test]
fn glow_threshold_and_intensity_keep_extended_linear_energy() {
    let output = render(&white().filter(Filter::Glow {
        threshold: 0.25,
        sigma: 0.0,
        intensity: 2.0,
    }));
    assert_eq!(output.pixel(2, 1), Some([2.5, 2.5, 2.5, 1.0]));
    assert_eq!(
        render(&white().filter(Filter::Glow {
            threshold: 2.0,
            sigma: 1.0,
            intensity: 5.0
        }))
        .pixels(),
        render(&white()).pixels()
    );
}

#[test]
fn adjustments_are_linear_alpha_preserving_and_ordered() {
    let color = Color::from_rgb_float(0.5, 0.25, -0.1).with_alpha(0.5);
    let output = render(
        &tile(Rect::new(2.0, 1.0, 3.0, 2.0), color).filter(Filter::ColorAdjust {
            exposure: 1.0,
            contrast: 1.0,
            saturation: 0.0,
            hue_degrees: 720.0,
        }),
    );
    let input = color.to_linear_premultiplied();
    let expected = 2.0 * (input[0] * 0.2126 + input[1] * 0.7152 + input[2] * 0.0722);
    for channel in 0..3 {
        close(output.pixel(2, 1).unwrap()[channel], expected);
    }
    assert_eq!(output.pixel(2, 1).unwrap()[3], 0.5);
    let mut tint = matrix();
    tint[0] = 0.0;
    tint[4] = 0.25;
    let adjust = Filter::ColorAdjust {
        exposure: 1.0,
        contrast: 1.0,
        saturation: 1.0,
        hue_degrees: 0.0,
    };
    assert_eq!(
        render(
            &white()
                .filter(Filter::Custom(vec![FilterOperation::ColorMatrix(tint)]))
                .filter(adjust.clone())
        )
        .pixel(2, 1)
        .unwrap()[0],
        0.5
    );
    assert_eq!(
        render(
            &white()
                .filter(adjust)
                .filter(Filter::Custom(vec![FilterOperation::ColorMatrix(tint)]))
        )
        .pixel(2, 1)
        .unwrap()[0],
        0.25
    );
}

#[test]
fn matrix_is_straight_rgba_clamps_alpha_and_clears_zero_alpha_rgb() {
    let mut matrix = matrix();
    matrix[4] = 2.0;
    matrix[18] = 0.0;
    matrix[19] = 0.25;
    let output =
        render(&white().filter(Filter::Custom(vec![FilterOperation::ColorMatrix(matrix)])));
    assert_eq!(output.pixel(2, 1), Some([0.75, 0.25, 0.25, 0.25]));
    matrix[19] = -1.0;
    assert_eq!(
        render(&white().filter(Filter::Custom(vec![FilterOperation::ColorMatrix(matrix)])))
            .pixel(2, 1),
        Some([0.0; 4])
    );
}

#[test]
fn channel_offsets_have_transformed_directions_and_max_sample_alpha() {
    let output = render(
        &white().filter(Filter::Custom(vec![FilterOperation::ChannelOffset {
            red: Vector2::new(1.0, 0.0),
            green: Vector2::new(0.0, 0.0),
            blue: Vector2::new(-1.0, 0.0),
        }])),
    );
    assert_eq!(output.pixel(1, 1), Some([0.0, 0.0, 1.0, 1.0]));
    assert_eq!(output.pixel(2, 1), Some([0.0, 1.0, 0.0, 1.0]));
    assert_eq!(output.pixel(3, 1), Some([1.0, 0.0, 0.0, 1.0]));
    let shifted = white()
        .filter(Filter::Custom(vec![FilterOperation::Offset {
            x: 1.0,
            y: 0.0,
        }]))
        .scale_xy(2.0, 1.0);
    assert!(render(&shifted).pixel(4, 1).unwrap()[3] > 0.0);
    let transformed = Filter::Custom(vec![FilterOperation::ChannelOffset {
        red: Vector2::new(1.0, 0.0),
        green: Vector2::new(0.0, 0.0),
        blue: Vector2::new(-1.0, 0.0),
    }])
    .device(
        motion_core::Transform::rotate(std::f64::consts::FRAC_PI_2),
        "filter",
    )
    .unwrap();
    let Filter::Custom(operations) = transformed else {
        panic!("channel primitive")
    };
    let FilterOperation::ChannelOffset { red, .. } = operations[0] else {
        panic!("channel offsets")
    };
    assert!(red.x.abs() < 1e-14);
    assert_eq!(red.y, 1.0);
}

#[test]
fn lens_vignette_is_bounded_and_neutral_strength_is_identity() {
    let source = tile(Rect::new(0.0, 0.0, 8.0, 4.0), Color::WHITE);
    let output = render(&source.clone().filter(Filter::Optics(
        motion_core::Optics::LensVignette {
            angle_of_view: 90.0,
            strength: 1.0,
            roundness: 0.0,
        },
    )));
    assert!(output.pixel(3, 1).unwrap()[0] > output.pixel(0, 0).unwrap()[0]);
    assert!(
        output
            .pixels()
            .iter()
            .all(|pixel| pixel[3] == 1.0 && (0.0..=1.0).contains(&pixel[0]))
    );
    assert_eq!(
        render(
            &source
                .clone()
                .filter(Filter::Optics(motion_core::Optics::LensVignette {
                    angle_of_view: 90.0,
                    strength: 0.0,
                    roundness: 0.0,
                }))
        )
        .pixels(),
        render(&source).pixels()
    );
}

#[test]
fn grain_is_seeded_and_stable_across_unrelated_guard_band_changes() {
    let grain = || {
        white().filter(Filter::Custom(vec![FilterOperation::Noise {
            seed: 13,
            amplitude: 0.1,
        }]))
    };
    let output = render(&grain());
    assert_eq!(output.pixels(), render(&grain()).pixels());
    assert_ne!(
        output.pixel(2, 1),
        render(&white().filter(Filter::Custom(vec![FilterOperation::Noise {
            seed: 14,
            amplitude: 0.1
        }])))
        .pixel(2, 1)
    );
    let unrelated = tile(Rect::new(100.0, 100.0, 101.0, 101.0), Color::WHITE)
        .filter(Filter::Blur { sigma: 1.0 });
    assert_eq!(
        output.pixels(),
        render(&Node::group([grain(), unrelated])).pixels()
    );
    assert_eq!(output.pixel(2, 1).unwrap()[3], 1.0);
}

#[test]
fn custom_primitives_refer_to_saved_filter_input_not_previous_result() {
    let node = white().filter(Filter::Custom(vec![
        FilterOperation::Offset { x: 1.0, y: 0.0 },
        FilterOperation::CompositeWithSource {
            mode: motion_core::drawing::BlendMode::SourceOver,
        },
        FilterOperation::Offset { x: 1.0, y: 0.0 },
        FilterOperation::CompositeWithSource {
            mode: motion_core::drawing::BlendMode::SourceOver,
        },
    ]));
    let output = render(&node);
    for column in 2..5 {
        assert_eq!(output.pixel(column, 1), Some([1.0; 4]));
    }
    assert_eq!(output.pixel(5, 1), Some([0.0; 4]));
    let primitives = white().filter(Filter::Custom(vec![
        FilterOperation::Threshold { value: 0.5 },
        FilterOperation::Noise {
            seed: 0,
            amplitude: 0.0,
        },
        FilterOperation::Blur { sigma: 0.0 },
    ]));
    assert_eq!(render(&primitives).pixel(2, 1), Some([0.5; 4]));
}

#[test]
fn node_mask_path_and_geometric_clips_apply_after_filtering() {
    let displaced = || {
        white().filter(Filter::Custom(vec![FilterOperation::Offset {
            x: 2.0,
            y: 0.0,
        }]))
    };
    let clip = Rect::new(4.0, 1.0, 5.0, 2.0);
    assert_eq!(
        render(&displaced().clip_with(clip, motion_core::ClipMode::Geometric)).pixel(4, 1),
        Some([1.0; 4])
    );
    assert_eq!(
        render(&displaced().clip_with(clip, motion_core::ClipMode::Geometric)).pixel(4, 1),
        Some([1.0; 4])
    );
    assert_eq!(
        render(&displaced().clip(motion_core::Path2D::from_rect(clip).unwrap())).pixel(4, 1),
        Some([1.0; 4])
    );
    assert_eq!(
        render(&displaced().mask(tile(clip, Color::WHITE), motion_core::MaskMode::Alpha))
            .pixel(4, 1),
        Some([1.0; 4])
    );
    let clipped_input = white().clip_with(
        Rect::new(2.0, 1.0, 2.5, 2.0),
        motion_core::ClipMode::Geometric,
    );
    let output = render(&Node::group([clipped_input]).filter(Filter::Custom(vec![
        FilterOperation::Offset { x: 2.0, y: 0.0 },
    ])));
    close(output.pixel(4, 1).unwrap()[3], 128.0 / 255.0);
}

#[test]
fn nested_filters_keep_child_effects_and_ancestor_domains_separate() {
    let child = white().filter(Filter::Custom(vec![FilterOperation::Offset {
        x: 1.0,
        y: 0.0,
    }]));
    let node = Node::group([child])
        .filter(Filter::Custom(vec![FilterOperation::Offset {
            x: 1.0,
            y: 0.0,
        }]))
        .clip_with(
            Rect::new(4.0, 1.0, 4.5, 2.0),
            motion_core::ClipMode::Geometric,
        )
        .opacity(0.5);
    close(render(&node).pixel(4, 1).unwrap()[3], 64.0 / 255.0);
}

#[test]
fn snapshots_and_layout_do_not_absorb_effect_halos() {
    let source = white();
    let bounds = source.measure(ValidationLimits::default()).unwrap();
    let filtered = source.clone().filter(Filter::Blur { sigma: 1.0 });
    assert_eq!(
        filtered.measure(ValidationLimits::default()).unwrap(),
        bounds
    );
    assert_eq!(render(&source).pixel(1, 1), Some([0.0; 4]));
    assert!(render(&filtered).pixel(1, 1).unwrap()[3] > 0.0);
    assert_eq!(render(&filtered), render(&filtered));
}

#[test]
fn filters_fail_before_hidden_or_empty_invalid_values_are_ignored() {
    for filter in [
        Filter::Blur { sigma: f64::NAN },
        Filter::Blur { sigma: -1.0 },
        Filter::Blur { sigma: f64::MAX },
        Filter::Optics(motion_core::Optics::LensVignette {
            angle_of_view: 90.0,
            strength: 2.0,
            roundness: 0.0,
        }),
        Filter::ColorAdjust {
            exposure: 1024.0,
            contrast: 1.0,
            saturation: 1.0,
            hue_degrees: 0.0,
        },
        Filter::Custom(vec![FilterOperation::Offset {
            x: f64::INFINITY,
            y: 0.0,
        }]),
    ] {
        for source in [white().opacity(0.0), Node::empty()] {
            assert!(
                source
                    .filter(filter.clone())
                    .compile(
                        Size::new(8.0, 4.0),
                        PixelSize::new(8, 4),
                        ValidationLimits::default()
                    )
                    .is_err()
            );
        }
    }
    let mut list = white()
        .compile(
            Size::new(8.0, 4.0),
            PixelSize::new(8, 4),
            ValidationLimits::default(),
        )
        .unwrap()
        .into_editable();
    list.commands.insert(
        0,
        DrawCommand::PushFilterLayer {
            bounds: Rect::new(2.0, 1.0, 3.0, 2.0),
            filters: vec![Filter::Blur { sigma: -1.0 }],
        },
    );
    list.commands.push(DrawCommand::PopLayer);
    assert!(
        list.prepare(ValidationLimits::default(), RasterLimits::default())
            .is_err()
    );
}

#[test]
fn filter_storage_work_and_guard_pixels_share_transactional_limits() {
    let node = white().filter(Filter::Blur { sigma: 0.5 });
    let list = node
        .compile(
            Size::new(8.0, 4.0),
            PixelSize::new(8, 4),
            ValidationLimits::default(),
        )
        .unwrap();
    assert_eq!(
        list.prepare(
            ValidationLimits::default(),
            RasterLimits {
                max_pixels: 32,
                ..RasterLimits::default()
            }
        )
        .unwrap_err()
        .field,
        "max_pixels"
    );
    let prepared = prepared(&node);
    let snapshot = prepared.clone();
    let output = prepared.render_linear(RasterLimits::default()).unwrap();
    let exact = RasterLimits {
        max_bytes: output.stats().peak_bytes,
        max_work: output.stats().work,
        ..RasterLimits::default()
    };
    assert_eq!(
        prepared.render_linear(exact).unwrap().pixels(),
        output.pixels()
    );
    assert_eq!(
        prepared
            .render_linear(RasterLimits {
                max_bytes: exact.max_bytes - 1,
                ..exact
            })
            .unwrap_err()
            .field,
        "max_bytes"
    );
    assert_eq!(
        prepared
            .render_linear(RasterLimits {
                max_work: exact.max_work - 1,
                ..exact
            })
            .unwrap_err()
            .field,
        "max_work"
    );
    assert_eq!(prepared, snapshot);
    assert_eq!(
        prepared
            .render_linear(RasterLimits::default())
            .unwrap()
            .pixels(),
        output.pixels()
    );
    let neutral = white().filter(Filter::Blur { sigma: 0.0 });
    let single = self::prepared(&neutral);
    let siblings = self::prepared(&Node::group([neutral.clone(), neutral]));
    let live_peak = |frame: &PreparedFrame| {
        frame
            .render_linear(RasterLimits::default())
            .unwrap()
            .stats()
            .peak_bytes
            - frame.stats().retained_bytes
    };
    assert_eq!(live_peak(&single), live_peak(&siblings));
}

#[test]
fn filter_color_overflow_returns_error_without_mutating_prepared_input() {
    let source = tile(
        Rect::new(2.0, 1.0, 3.0, 2.0),
        Color::from_rgb_float(10.0, 0.0, 0.0),
    );
    let prepared = prepared(&source.filter(Filter::ColorAdjust {
        exposure: 1023.0,
        contrast: 2.0,
        saturation: 1.0,
        hue_degrees: 0.0,
    }));
    let before = prepared.clone();
    assert_eq!(
        prepared
            .render_linear(RasterLimits::default())
            .unwrap_err()
            .kind,
        ValidationErrorKind::InvalidValue
    );
    assert_eq!(prepared, before);
}

#[test]
fn filter_primitives_count_against_source_and_output_structural_limits() {
    let node = white().filter(Filter::Custom(vec![
        FilterOperation::Noise {
            seed: 1,
            amplitude: 0.0
        };
        12
    ]));
    let limits = ValidationLimits {
        max_nodes: 10,
        ..ValidationLimits::default()
    };
    assert_eq!(
        node.clone()
            .opacity(0.0)
            .compile(Size::new(8.0, 4.0), PixelSize::new(8, 4), limits)
            .unwrap_err()
            .field,
        "max_nodes"
    );
    let list = node
        .compile(
            Size::new(8.0, 4.0),
            PixelSize::new(8, 4),
            ValidationLimits::default(),
        )
        .unwrap();
    assert_eq!(list.preflight(limits).unwrap_err().field, "max_nodes");
    let exact = list.preflight(ValidationLimits::default()).unwrap();
    list.preflight(ValidationLimits {
        max_nodes: exact.nodes,
        max_depth: exact.max_depth,
    })
    .unwrap();
    assert_eq!(
        list.preflight(ValidationLimits {
            max_nodes: exact.nodes,
            max_depth: exact.max_depth - 1
        })
        .unwrap_err()
        .field,
        "max_depth"
    );
}

#[test]
fn prepared_filter_metadata_and_work_have_exact_admission_boundaries() {
    let node = white().filter(Filter::Custom(vec![
        FilterOperation::Offset { x: 0.5, y: -0.5 },
        FilterOperation::Blur { sigma: 0.5 },
    ]));
    let list = node
        .compile(
            Size::new(8.0, 4.0),
            PixelSize::new(8, 4),
            ValidationLimits::default(),
        )
        .unwrap();
    let prepared = list
        .prepare(ValidationLimits::default(), RasterLimits::default())
        .unwrap();
    let exact = RasterLimits {
        max_bytes: prepared.stats().peak_bytes,
        max_work: prepared.stats().work,
        ..RasterLimits::default()
    };
    assert_eq!(
        list.prepare(ValidationLimits::default(), exact).unwrap(),
        prepared
    );
    assert_eq!(
        list.prepare(
            ValidationLimits::default(),
            RasterLimits {
                max_bytes: exact.max_bytes - 1,
                ..exact
            }
        )
        .unwrap_err()
        .field,
        "max_bytes"
    );
    assert_eq!(
        list.prepare(
            ValidationLimits::default(),
            RasterLimits {
                max_work: exact.max_work - 1,
                ..exact
            }
        )
        .unwrap_err()
        .field,
        "max_work"
    );
}

#[test]
fn blur_transform_uses_singular_value_and_mask_artwork_can_have_filters() {
    let Filter::Blur { sigma } = (Filter::Blur { sigma: 2.0 })
        .device(
            motion_core::Transform::scale_non_uniform(-3.0, 0.5),
            "filter",
        )
        .unwrap()
    else {
        panic!("blur")
    };
    assert_eq!(sigma, 6.0);
    let source = white().filter(Filter::DropShadow {
        offset: Vector2::new(2.0, 0.0),
        sigma: 0.0,
        color: Color::WHITE,
    });
    let mask = white().filter(Filter::Custom(vec![FilterOperation::Offset {
        x: 2.0,
        y: 0.0,
    }]));
    let output = render(&source.mask(mask, motion_core::MaskMode::Alpha));
    assert_eq!(output.pixel(2, 1), Some([0.0; 4]));
    assert_eq!(output.pixel(4, 1), Some([1.0; 4]));
    assert_eq!(
        render(&Node::empty().filter(Filter::Blur { sigma: 1.0 })).pixels(),
        &[[0.0; 4]; 32]
    );
}

#[test]
fn filter_result_blends_once_against_the_immediate_backdrop() {
    let content = white()
        .filter(Filter::Custom(vec![FilterOperation::Offset {
            x: 2.0,
            y: 0.0,
        }]))
        .opacity(0.5);
    let backdrop = tile(Rect::new(4.0, 1.0, 5.0, 2.0), Color::from_hex(0xff0000));
    let output = render(&Node::group([
        backdrop,
        content.blend_mode(motion_core::BlendMode::Multiply),
    ]));
    assert_eq!(output.pixel(4, 1), Some([1.0, 0.0, 0.0, 1.0]));
    assert_eq!(output.pixel(2, 1), Some([0.0; 4]));
}

fn optics_surface(bounds: PixelRect, budget: &mut Budget) -> Surface {
    let mut surface = Surface::new(
        bounds,
        1.0,
        motion_core::drawing::BlendMode::SourceOver,
        [0.0; 4],
        budget,
        "test",
    )
    .unwrap();
    let mut state = 0x9e3779b97f4a7c15_u64;
    for (index, pixel) in surface.pixels.iter_mut().enumerate() {
        state ^= state << 13;
        state ^= state >> 7;
        state ^= state << 17;
        // Sparse bright pixels over a field that mixes +0.0 and -0.0.
        *pixel = if state.is_multiple_of(37) {
            let value = (state >> 40) as f32 / (1 << 24) as f32;
            [value * 2.0, value, value * 0.5, value.min(1.0)]
        } else if index % 3 == 0 {
            [-0.0; 4]
        } else {
            [0.0; 4]
        };
    }
    surface
}

#[test]
fn optical_source_reads_whole_pixels_exactly_like_bilinear_sampling() {
    let bounds = PixelRect {
        left: 3,
        top: 2,
        right: 21,
        bottom: 13,
    };
    let mut budget = Budget::new(PixelSize::new(24, 16), RasterLimits::default()).unwrap();
    let mut surface = optics_surface(bounds, &mut budget);
    surface.pixels[0] = [-0.0, 0.25, -0.0, 0.5];
    for row in 0..16 {
        for column in 0..24 {
            let position = [f64::from(column), f64::from(row)];
            let expected = sample(&surface, position[0], position[1]);
            let actual = super::optics::source_pixel(&surface, position);
            assert_eq!(
                actual.map(f64::to_bits),
                expected.map(f64::to_bits),
                "{column},{row}"
            );
        }
    }
    for position in [[4.5, 3.0], [7.0, 5.25], [20.75, 12.5]] {
        assert_eq!(
            super::optics::source_pixel(&surface, position).map(f64::to_bits),
            sample(&surface, position[0], position[1]).map(f64::to_bits)
        );
    }
}

#[test]
fn bokeh_zero_footprint_skips_only_exactly_zero_output() {
    let bounds = PixelRect {
        left: 0,
        top: 0,
        right: 96,
        bottom: 80,
    };
    let mut budget = Budget::new(PixelSize::new(96, 80), RasterLimits::default()).unwrap();
    let mut surface = optics_surface(bounds, &mut budget);
    // Clear a wide band so some tiles have an all-zero footprint.
    for row in 0..80 {
        for column in 30..96 {
            surface.pixels[row * 96 + column] = if column % 2 == 0 { [0.0; 4] } else { [-0.0; 4] };
        }
    }
    for shape in [
        motion_core::ApertureShape::Disc,
        motion_core::ApertureShape::Polygon {
            sides: 6,
            rotation: 0.3,
        },
    ] {
        let optics = motion_core::Optics::Bokeh {
            radius: 9.5,
            shape,
            highlight_boost: 0.8,
        };
        let (values, kernel) = optics.parameters(Rect::new(0.0, 0.0, 96.0, 80.0));
        let live = budget.live_bytes();
        let zero = super::optics::ZeroFootprint::new(&surface, &kernel, &mut budget, "test")
            .unwrap()
            .expect("small finite reach");
        let mut skipped = 0;
        for row in 0..80 {
            for column in 0..96 {
                if zero.contains(&surface, column, row) {
                    skipped += 1;
                    let pixel = optical_pixel(
                        &surface,
                        [f64::from(column), f64::from(row)],
                        &values,
                        &kernel,
                    );
                    assert_eq!(
                        checked(pixel, "test").unwrap().map(f32::to_bits),
                        [0.0_f32; 4].map(f32::to_bits),
                        "{column},{row}"
                    );
                }
            }
        }
        assert!(skipped > 0 && skipped < 96 * 80);
        zero.release(&mut budget);
        assert_eq!(budget.live_bytes(), live);
    }
}
