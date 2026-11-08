use super::*;
use crate::{RasterLimits, RenderPrepare};
use motion_core::{
    Color, Filter, FilterOperation, MaskMode, Node, NodeModifiers, Point, Rect, Size,
    ValidationLimits, Vector2,
};

#[test]
fn cinematic_grain_is_identical_across_band_heights() {
    let prepared = prepared(
        Rect::new(0.0, 0.0, 34.0, 70.0)
            .fill(Color::rgb(90, 130, 180))
            .into(),
        true,
    );
    let display = crate::DisplayTransform {
        tone_map: crate::ToneMap::AgX,
        grain: Some(crate::FilmGrain {
            seed: 42,
            amount: 0.02,
            size: 1.5,
            luma_response: 1.0,
            chroma: 0.2,
        }),
        ..crate::DisplayTransform::default()
    };
    let settings = crate::display::DisplaySettings {
        display: Some(&display),
        origin: [0, 0],
        frame: 7,
    };
    let limits = RasterLimits::default();
    let linear = prepared.render_linear(limits).unwrap();
    for format in [
        PixelFormat::SrgbRgba8 {
            premultiplied: false,
        },
        PixelFormat::Yuv420 { bit_depth: 8 },
        PixelFormat::Yuv420 { bit_depth: 10 },
    ] {
        let reference =
            FrameBuffer::from_linear_display(&linear, format, limits, &display, 7).unwrap();
        for height in [2, 16, 64] {
            let (actual, _, _) = FrameBuffer::from_prepared_transformed(
                &prepared,
                format,
                limits,
                &mut crate::cpu::pool::SurfacePool::default(),
                &OutputPool::default(),
                (height, settings),
            )
            .unwrap();
            assert_eq!(reference, actual, "{format:?}, bands {height}");
        }
    }
}

fn prepared(node: Node, opaque: bool) -> PreparedFrame {
    prepared_size(node, opaque, PixelSize::new(34, 70))
}

fn prepared_size(node: Node, opaque: bool, size: PixelSize) -> PreparedFrame {
    node.compile(
        Size::new(f64::from(size.width), f64::from(size.height)),
        size,
        ValidationLimits::default(),
    )
    .unwrap()
    .with_background(opaque.then_some(Color::BLACK))
    .prepare(ValidationLimits::default(), RasterLimits::default())
    .unwrap()
}

#[test]
fn filter_halo_uses_nested_chains_not_sibling_sums() {
    let tile = Node::from(Rect::new(2.0, 2.0, 30.0, 66.0).fill(Color::WHITE));
    let siblings = Node::group([
        tile.clone().filter(Filter::Blur { sigma: 5.0 }),
        tile.clone().filter(Filter::Blur { sigma: 9.0 }),
    ]);
    assert_eq!(prepared(siblings.clone(), false).filter_halo, 25);
    assert_eq!(
        prepared(siblings.filter(Filter::Blur { sigma: 2.0 }), false).filter_halo,
        29
    );
    assert_eq!(
        prepared(tile.clone().filter(Filter::Blur { sigma: 1.0 }), false).filter_halo,
        2
    );
    let masked = tile
        .clone()
        .filter(Filter::Blur { sigma: 5.0 })
        .mask(
            tile.clone().filter(Filter::Blur { sigma: 9.0 }),
            MaskMode::Alpha,
        )
        .filter(Filter::Blur { sigma: 2.0 });
    assert_eq!(prepared(masked.clone(), false).filter_halo, 25);
    assert_eq!(
        prepared(
            Node::group([masked]).filter(Filter::Blur { sigma: 2.0 }),
            false,
        )
        .filter_halo,
        29
    );
    assert_eq!(prepared(tile, false).filter_halo, 0);
}

#[test]
fn filter_padding_uses_nested_chains_not_shutter_sibling_sums() {
    let tile = Node::from(Rect::new(2.0, 2.0, 30.0, 66.0).fill(Color::WHITE));
    let siblings = Node::group((0..64).map(|_| tile.clone().filter(Filter::Blur { sigma: 9.0 })));
    assert_eq!(
        prepared(siblings.clone(), false).size,
        PixelSize::new(88, 124)
    );
    assert_eq!(
        prepared(siblings.filter(Filter::Blur { sigma: 2.0 }), false).size,
        PixelSize::new(100, 136)
    );
}

#[test]
fn adaptive_band_height_is_bounded_and_keeps_conversion_phase() {
    for (halo, height, expected) in [
        (0, 360, 16),
        (4, 360, 32),
        (22, 360, 176),
        (23, 360, 360),
        (u32::MAX, 360, 360),
        (0, 7, 7),
    ] {
        assert_eq!(band_height(halo, height), expected);
    }
    assert_eq!(smaller_band(70), 32);
    assert_eq!(smaller_band(32), 16);
}

#[test]
fn adaptive_bands_shrink_within_byte_limits_without_refunding_failed_work() {
    let frame = prepared_size(
        Rect::new(1.25, -2.5, 32.75, 515.5)
            .fill(Color::rgb(20, 150, 210))
            .filter(Filter::Blur { sigma: 14.0 }),
        true,
        PixelSize::new(34, 512),
    );
    let mut surfaces = crate::cpu::pool::SurfacePool::default();
    let pool = OutputPool::default();
    for format in [
        PixelFormat::SrgbRgba8 {
            premultiplied: false,
        },
        PixelFormat::SrgbRgba8 {
            premultiplied: true,
        },
        PixelFormat::LinearRgba16Float,
        PixelFormat::Yuv420 { bit_depth: 8 },
        PixelFormat::Yuv420 { bit_depth: 10 },
    ] {
        let limits = RasterLimits::default();
        let (reference, linear, output) =
            FrameBuffer::from_prepared_banded(&frame, format, limits, &mut surfaces, &pool, 16)
                .unwrap();
        let tight = RasterLimits {
            max_bytes: linear.peak_bytes.max(output.peak_bytes),
            ..limits
        };
        assert!(matches!(
            frame.render_linear(tight).unwrap_err().kind,
            motion_core::ValidationErrorKind::StructuralLimit
        ));
        let (actual, attempted, converted) =
            FrameBuffer::from_prepared(&frame, format, tight, &mut surfaces, &pool).unwrap();
        assert_eq!(actual, reference);
        assert!(attempted.work > linear.work);
        assert!(attempted.peak_bytes <= tight.max_bytes);
        let exact = RasterLimits {
            max_work: attempted.work + converted.work,
            ..tight
        };
        assert_eq!(
            FrameBuffer::from_prepared(&frame, format, exact, &mut surfaces, &pool,)
                .unwrap()
                .0,
            reference
        );
        assert_eq!(
            FrameBuffer::from_prepared(
                &frame,
                format,
                RasterLimits {
                    max_work: exact.max_work - 1,
                    ..exact
                },
                &mut surfaces,
                &pool,
            )
            .unwrap_err()
            .field,
            "max_work"
        );
        assert_eq!(
            FrameBuffer::from_prepared(&frame, format, exact, &mut surfaces, &pool,)
                .unwrap()
                .0,
            reference
        );
    }
}

#[test]
fn fused_bands_match_full_frames_for_layers_masks_filters_and_all_formats() {
    let gradient = motion_core::LinearGradient::new(Point::ORIGIN, Point::new(1.0, 1.0))
        .stop(0.0, Color::from_rgb_float(2.0, -0.5, 0.25).with_alpha(0.75))
        .stop(1.0, Color::rgb(20, 150, 210));
    let tile = Node::from(Rect::new(2.25, -1.5, 30.75, 67.25).fill(gradient));
    let filters = [
        Filter::Blur { sigma: 1.25 },
        Filter::Blur { sigma: 5.0 },
        Filter::Blur { sigma: 9.0 },
        Filter::Blur { sigma: 14.0 },
        Filter::DropShadow {
            offset: Vector2::new(1.25, 8.5),
            sigma: 1.0,
            color: Color::WHITE,
        },
        Filter::Glow {
            threshold: 0.1,
            sigma: 0.75,
            intensity: 1.5,
        },
        Filter::Optics(motion_core::Optics::LensVignette {
            angle_of_view: 90.0,
            strength: 0.4,
            roundness: 0.0,
        }),
        Filter::Custom(vec![FilterOperation::Noise {
            seed: 12,
            amplitude: 0.2,
        }]),
        Filter::Custom(vec![
            FilterOperation::Offset { x: -0.5, y: -7.5 },
            FilterOperation::Blur { sigma: 0.5 },
            FilterOperation::CompositeWithSource {
                mode: motion_core::BlendMode::Plus,
            },
        ]),
    ];
    for node in std::iter::once(tile.clone())
        .chain(
            filters
                .into_iter()
                .map(|filter| tile.clone().filter(filter)),
        )
        .chain([
            Node::group([tile.clone().opacity(0.4), tile.clone().translate(1.0, 1.0)])
                .mask(
                    Rect::new(0.0, 0.0, 28.5, 69.25).fill(Color::WHITE),
                    MaskMode::InvertedLuminance,
                )
                .filter(Filter::Blur { sigma: 1.25 })
                .opacity(0.6),
        ])
    {
        for opaque in [false, true] {
            for quality in [
                motion_core::RenderQuality::Draft,
                motion_core::RenderQuality::Final,
            ] {
                let frame = prepared(node.clone(), opaque).quality(quality);
                let limits = RasterLimits::default();
                let linear = frame.render_linear(limits).unwrap();
                for format in [
                    PixelFormat::SrgbRgba8 {
                        premultiplied: false,
                    },
                    PixelFormat::SrgbRgba8 {
                        premultiplied: true,
                    },
                    PixelFormat::LinearRgba16Float,
                    PixelFormat::Yuv420 { bit_depth: 8 },
                    PixelFormat::Yuv420 { bit_depth: 10 },
                ] {
                    let mut surfaces = crate::cpu::pool::SurfacePool::default();
                    let pool = OutputPool::default();
                    let actual =
                        FrameBuffer::from_prepared(&frame, format, limits, &mut surfaces, &pool);
                    let expected = FrameBuffer::from_linear(&linear, format, limits);
                    match (actual, expected) {
                        (Ok((actual, _, _)), Ok(expected)) => {
                            assert_eq!(actual, expected, "{format:?} opaque={opaque}")
                        }
                        (Err(actual), Err(expected)) => {
                            assert_eq!((actual.kind, actual.field), (expected.kind, expected.field))
                        }
                        other => panic!("Output mismatch: {other:?}"),
                    }
                }
            }
        }
    }
}

#[test]
fn fused_storage_work_recycling_and_failure_retry_are_bounded() {
    let frame = prepared(
        Rect::new(0.0, 0.0, 34.0, 70.0).fill(Color::WHITE).into(),
        true,
    );
    let format = PixelFormat::Yuv420 { bit_depth: 10 };
    let limits = RasterLimits::default();
    let mut surfaces = crate::cpu::pool::SurfacePool::default();
    let pool = OutputPool::default();
    let (reference, linear, output) =
        FrameBuffer::from_prepared(&frame, format, limits, &mut surfaces, &pool).unwrap();
    let exact = RasterLimits {
        max_work: linear.work + output.work,
        max_bytes: linear.peak_bytes.max(output.peak_bytes),
        ..limits
    };
    let (actual, _, _) =
        FrameBuffer::from_prepared(&frame, format, exact, &mut surfaces, &pool).unwrap();
    assert_eq!(actual, reference);
    let pointers = actual
        .planes
        .iter()
        .map(|plane| plane.bytes.as_ptr())
        .collect::<Vec<_>>();
    drop(actual);
    assert!(pool.bytes() > 0);
    let (actual, _, _) =
        FrameBuffer::from_prepared(&frame, format, exact, &mut surfaces, &pool).unwrap();
    assert!(
        actual
            .planes
            .iter()
            .any(|plane| pointers.contains(&plane.bytes.as_ptr()))
    );
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
        assert!(FrameBuffer::from_prepared(&frame, format, limits, &mut surfaces, &pool).is_err());
    }
    assert_eq!(
        FrameBuffer::from_prepared(&frame, format, exact, &mut surfaces, &pool)
            .unwrap()
            .0,
        reference
    );
    drop(actual);
    pool.trim(1);
    assert_eq!(pool.bytes(), 0);
}
