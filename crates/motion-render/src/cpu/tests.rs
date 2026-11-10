use super::*;
use motion_core::Shape;
use motion_core::{Color, Node, NodeModifiers, Rect, ValidationLimits};

fn frame(node: &Node) -> PreparedFrame {
    node.compile(
        motion_core::Size::new(8.0, 4.0),
        PixelSize::new(8, 4),
        ValidationLimits::default(),
    )
    .unwrap()
    .prepare(ValidationLimits::default(), RasterLimits::default())
    .unwrap()
}

fn tile(color: Color) -> Node {
    Rect::new(0.0, 0.0, 4.0, 4.0).fill(color).into()
}

#[test]
fn solid_spans_preserve_pixel_blending_bits_and_failure_prefixes() {
    let values = [
        0.0,
        -0.0,
        f32::from_bits(1),
        0.5,
        1.0,
        -1.0,
        f32::MAX,
        f32::INFINITY,
        f32::NAN,
    ];
    for count in [0, 1, 2, 3, 4, 5, 6, 7, 8, 9, 16, 17, 31] {
        for source_index in 0..values.len() {
            let source =
                std::array::from_fn(|channel| values[(source_index + channel) % values.len()]);
            for destination_index in 0..values.len() {
                let mut expected: Vec<[f32; 4]> = (0..count)
                    .map(|index| {
                        std::array::from_fn(|channel| {
                            values[(destination_index + index + channel) % values.len()]
                        })
                    })
                    .collect();
                let mut actual = expected.clone();
                let reference = expected
                    .iter_mut()
                    .try_for_each(|pixel| blend(pixel, source, 1.0, "span.test"));
                let result = blend_span(&mut actual, source, "span.test");
                assert_eq!(result, reference);
                assert_eq!(
                    actual
                        .iter()
                        .map(|pixel| pixel.map(f32::to_bits))
                        .collect::<Vec<_>>(),
                    expected
                        .iter()
                        .map(|pixel| pixel.map(f32::to_bits))
                        .collect::<Vec<_>>()
                );
            }
        }
    }
}

#[test]
fn solid_spans_preserve_every_scalar_failure_position() {
    let source = [0.25, 0.5, 0.75, -1.0];
    for count in [2, 3, 8, 17, 31] {
        for failure in 0..count {
            for value in [f32::MAX, f32::INFINITY, f32::NAN] {
                let mut expected = vec![[0.125, 0.25, 0.5, 1.0]; count];
                expected[failure][2] = value;
                let mut actual = expected.clone();
                let reference = expected.iter_mut().try_for_each(|destination| {
                    let result = std::array::from_fn(|channel| {
                        source[channel] * 1.0 + destination[channel] * (1.0 - source[3] * 1.0)
                    });
                    store_blended(destination, result, "span.failure")
                });
                assert_eq!(blend_span(&mut actual, source, "span.failure"), reference);
                assert_eq!(
                    actual
                        .iter()
                        .map(|pixel| pixel.map(f32::to_bits))
                        .collect::<Vec<_>>(),
                    expected
                        .iter()
                        .map(|pixel| pixel.map(f32::to_bits))
                        .collect::<Vec<_>>()
                );
            }
        }
    }
}

#[test]
fn source_over_matches_reference_order_and_preserves_failed_destinations() {
    let values = [
        0.0,
        -0.0,
        f32::from_bits(1),
        -f32::from_bits(1),
        f32::EPSILON,
        0.25,
        0.5,
        1.0,
        -1.0,
        f32::MAX,
        -f32::MAX,
        f32::MAX / 2.0,
        f32::INFINITY,
        f32::NEG_INFINITY,
        f32::NAN,
    ];
    for source_index in 0..values.len() {
        for destination_index in 0..values.len() {
            for coverage in [0.0, f32::from_bits(1), 0.25, 0.5, 1.0, -1.0, 2.0, f32::NAN] {
                let source: [f32; 4] =
                    std::array::from_fn(|channel| values[(source_index + channel) % values.len()]);
                let destination: [f32; 4] = std::array::from_fn(|channel| {
                    values[(destination_index + channel * 3) % values.len()]
                });
                let remaining = 1.0 - source[3] * coverage;
                let expected: [f32; 4] = std::array::from_fn(|channel| {
                    source[channel] * coverage + destination[channel] * remaining
                });
                let mut actual = destination;
                let result = blend(&mut actual, source, coverage, "test.source_over");
                if expected.iter().all(|value| value.is_finite()) {
                    result.unwrap();
                    assert_eq!(actual.map(f32::to_bits), expected.map(f32::to_bits));
                } else {
                    assert!(result.is_err());
                    assert_eq!(actual.map(f32::to_bits), destination.map(f32::to_bits));
                }
            }
        }
    }
}

#[test]
fn pooled_render_clears_previous_layers_and_recovers_after_budget_failure() {
    let limits = RasterLimits::default();
    let mut pool = pool::SurfacePool::default();
    let prepared = frame(
        &Node::group([
            tile(Color::WHITE),
            tile(Color::rgb(190, 40, 125)).opacity(0.5),
        ])
        .opacity(0.75),
    );
    let reference = prepared.render_linear(limits).unwrap();
    let first = prepared.render_linear_pooled(limits, &mut pool).unwrap();
    assert_eq!(first, reference);
    first.recycle(&mut pool);
    assert!(pool.bytes() > 0);
    assert!(
        prepared
            .render_linear_pooled(
                RasterLimits {
                    max_work: 1,
                    ..limits
                },
                &mut pool
            )
            .is_err()
    );
    let empty = frame(&Node::empty());
    let actual = empty.render_linear_pooled(limits, &mut pool).unwrap();
    assert_eq!(actual, empty.render_linear(limits).unwrap());
    assert!(actual.pixels().iter().all(|pixel| *pixel == [0.0; 4]));
    actual.recycle(&mut pool);
    assert!(pool.bytes() <= 64 * 1024 * 1024);
    assert_eq!(
        prepared.render_linear_pooled(limits, &mut pool).unwrap(),
        reference
    );
}

#[test]
fn zero_blur_preserves_the_completed_group() {
    let content = Node::group([tile(Color::WHITE), tile(Color::WHITE)]);
    let plain = frame(&content)
        .render_linear(RasterLimits::default())
        .unwrap();
    let filtered = frame(&content.filter(motion_core::Filter::Blur { sigma: 0.0 }))
        .render_linear(RasterLimits::default())
        .unwrap();
    assert_eq!(plain.pixels(), filtered.pixels());
}

#[test]
fn blur_retains_offscreen_inputs_and_expands_parent_captures() {
    let source = Node::from(Rect::new(-1.0, 1.0, 0.0, 2.0).fill(Color::WHITE));
    let node = Node::group([source.filter(motion_core::Filter::Blur { sigma: 0.5 })]).opacity(0.5);
    let output = frame(&node).render_linear(RasterLimits::default()).unwrap();
    assert_eq!(output.size(), PixelSize::new(8, 4));
    assert!(output.pixel(0, 1).unwrap()[3] > 0.03);
    assert_eq!(output.pixel(3, 1), Some([0.0; 4]));
}

#[test]
fn overlapping_group_fades_once_instead_of_fading_each_primitive() {
    let pair = [tile(Color::WHITE), tile(Color::WHITE).translate(2.0, 0.0)];
    let isolated = frame(&Node::group(pair.clone()).opacity(0.5))
        .render_linear(RasterLimits::default())
        .unwrap();
    assert_eq!(isolated.pixel(0, 0), Some([0.5; 4]));
    assert_eq!(isolated.pixel(3, 0), Some([0.5; 4]));
    assert_eq!(isolated.pixel(5, 0), Some([0.5; 4]));
    assert_eq!(isolated.pixel(6, 0), Some([0.0; 4]));
    assert_eq!(isolated.pixel(8, 0), None);
    let individual = frame(&Node::group(pair.map(|node| node.opacity(0.5))))
        .render_linear(RasterLimits::default())
        .unwrap();
    assert_eq!(individual.pixel(3, 0), Some([0.75; 4]));
}

#[test]
fn source_over_is_linear_and_nested_alpha_preserves_extended_rgb() {
    let source = tile(Color::WHITE.with_alpha(0.5))
        .compile(
            motion_core::Size::new(8.0, 4.0),
            PixelSize::new(8, 4),
            ValidationLimits::default(),
        )
        .unwrap();
    let source = source.with_background(Some(Color::BLACK));
    let prepared = source
        .prepare(ValidationLimits::default(), RasterLimits::default())
        .unwrap();
    let output = prepared.render_linear(RasterLimits::default()).unwrap();
    assert_eq!(output.pixel(0, 0), Some([0.5, 0.5, 0.5, 1.0]));
    assert_eq!(output.pixel(4, 0), Some([0.0, 0.0, 0.0, 1.0]));
    let red = tile(Color::from_hex(0xff0000).with_alpha(0.5));
    let blue = tile(Color::from_hex(0x0000ff).with_alpha(0.5));
    let output = frame(&Node::group([red.clone(), blue.clone()]))
        .render_linear(RasterLimits::default())
        .unwrap();
    assert_eq!(output.pixel(0, 0), Some([0.25, 0.0, 0.5, 0.75]));
    let reverse = frame(&Node::group([blue, red]))
        .render_linear(RasterLimits::default())
        .unwrap();
    assert_eq!(reverse.pixel(0, 0), Some([0.5, 0.0, 0.25, 0.75]));
    let color = Color::from_rgb_float(1.25, -0.125, 0.333);
    let node = Node::group([tile(color).opacity(0.5)]).opacity(0.5);
    let output = frame(&node).render_linear(RasterLimits::default()).unwrap();
    assert_eq!(
        output.pixel(0, 0).unwrap(),
        color
            .to_linear_premultiplied()
            .map(|value| (value as f32) * 0.25)
    );
}

#[test]
fn fractional_strips_and_solid_spans_produce_the_same_working_pixels() {
    let node: Node = Rect::new(0.25, 0.5, 7.5, 4.0).fill(Color::WHITE).into();
    let prepared = frame(&node);
    let expected = crate::raster::tests::coverage(&prepared);
    let output = prepared.render_linear(RasterLimits::default()).unwrap();
    for (pixel, coverage) in output.pixels().iter().zip(expected) {
        assert_eq!(*pixel, [f32::from(coverage) / 255.0; 4]);
    }
    let empty = frame(&Node::empty())
        .render_linear(RasterLimits::default())
        .unwrap();
    assert!(empty.pixels().iter().all(|pixel| *pixel == [0.0; 4]));
}

#[test]
fn tiny_opacity_multiplies_hdr_before_f32_narrowing() {
    let color = Color::from_rgb_float(1.0e10, 0.0, 0.0);
    let opacity = 1.0e-50;
    assert_eq!(opacity as f32, 0.0);
    let output = frame(&tile(color).opacity(opacity))
        .render_linear(RasterLimits::default())
        .unwrap();
    let expected = (f64::from(color.to_linear_premultiplied()[0] as f32) * opacity) as f32;
    assert!(expected > 0.0);
    assert_eq!(output.pixel(0, 0).unwrap()[0], expected);
}

#[test]
fn captures_use_clipped_bounds_and_release_before_the_next_sibling() {
    let child = tile(Color::WHITE).opacity(0.5);
    let single = frame(&child);
    let pair = frame(&Node::group([child.clone(), child]));
    let one = single.render_linear(RasterLimits::default()).unwrap();
    let two = pair.render_linear(RasterLimits::default()).unwrap();
    assert_eq!(
        one.stats.peak_bytes - single.stats.retained_bytes,
        two.stats.peak_bytes - pair.stats.retained_bytes
    );
    let hidden = frame(&tile(Color::WHITE).translate(1000.0, 1000.0).opacity(0.0))
        .render_linear(RasterLimits::default())
        .unwrap();
    assert!(hidden.pixels().iter().all(|pixel| *pixel == [0.0; 4]));
    let clipped = frame(&Node::group([tile(Color::WHITE).translate(-2.0, 0.0)]).opacity(0.5))
        .render_linear(RasterLimits::default())
        .unwrap();
    assert_eq!(clipped.pixel(0, 0), Some([0.5; 4]));
    assert_eq!(clipped.pixel(2, 0), Some([0.0; 4]));
}

#[test]
fn live_surface_bytes_pixels_and_work_have_exact_independent_budgets() {
    let prepared = frame(&Node::group([tile(Color::WHITE).opacity(0.5)]).opacity(0.5));
    let output = prepared.render_linear(RasterLimits::default()).unwrap();
    let exact = RasterLimits {
        max_pixels: 32,
        max_bytes: output.stats.peak_bytes,
        max_work: output.stats.work,
    };
    assert_eq!(prepared.render_linear(exact).unwrap(), output);
    for (limits, field) in [
        (
            RasterLimits {
                max_pixels: 31,
                ..exact
            },
            "max_pixels",
        ),
        (
            RasterLimits {
                max_bytes: exact.max_bytes - 1,
                ..exact
            },
            "max_bytes",
        ),
        (
            RasterLimits {
                max_work: exact.max_work - 1,
                ..exact
            },
            "max_work",
        ),
    ] {
        assert_eq!(prepared.render_linear(limits).unwrap_err().field, field);
        assert_eq!(prepared.render_linear(exact).unwrap(), output);
    }
}

#[test]
fn finite_working_overflow_fails_without_mutating_the_prepared_input() {
    let color = Color::from_rgb_float(1.5e16, 0.0, 0.0).with_alpha(0.5);
    let prepared = frame(&Node::group([tile(color), tile(color)]));
    let original = prepared.clone();
    let error = prepared.render_linear(RasterLimits::default()).unwrap_err();
    assert_eq!(error.field, "color");
    assert_eq!(prepared, original);
    assert!(
        frame(&tile(Color::WHITE))
            .render_linear(RasterLimits::default())
            .is_ok()
    );
}
