//! Property tests comparing prepared coverage with CPU raster output.

use motion_core::{
    Color, Node, Path2D, PixelSize, Point, RasterLimits, Rect, Shape, Size, ValidationLimits,
};
use motion_render::{PreparedFrame, RenderPrepare};
use proptest::prelude::*;

fn prepare(node: Node) -> PreparedFrame {
    node.compile(
        Size::new(32.0, 32.0),
        PixelSize::new(32, 32),
        ValidationLimits::default(),
    )
    .unwrap()
    .prepare(ValidationLimits::default(), RasterLimits::default())
    .unwrap()
}

fn coverage(frame: &PreparedFrame) -> Vec<u8> {
    let mut pixels = vec![0; 32 * 32];
    for span in frame.spans() {
        for row in span.y()..span.y() + span.height() {
            for column in span.start_x()..span.end_x() {
                pixels[(row * 32 + column) as usize] = 255;
            }
        }
    }
    for strip in frame.strips() {
        for row in 0..strip.height() {
            for column in 0..strip.width() {
                let offset = strip.alpha_offset() + (row * strip.width() + column) as usize;
                pixels[((strip.y() + row) * 32 + strip.x() + column) as usize] =
                    frame.alphas()[offset];
            }
        }
    }
    pixels
}

#[test]
fn zero_optical_parameters_preserve_pixels_and_invalid_settings_fail() {
    use motion_core::{ApertureShape, Filter, FilterOperation, NodeModifiers, Optics, Transform};
    let source: Node = Rect::new(3.0, 4.0, 25.0, 26.0)
        .fill(Color::rgb(30, 120, 210))
        .into();
    let limits = RasterLimits::default();
    let reference = prepare(source.clone()).render_linear(limits).unwrap();
    for filter in [
        Filter::Optics(Optics::Lens {
            distortion: 0.0,
            chromatic: 0.0,
            auto_fit: true,
            center: None,
        }),
        Filter::Optics(Optics::LensVignette {
            angle_of_view: 45.0,
            strength: 0.0,
            roundness: 1.0,
        }),
        Filter::Optics(Optics::Bokeh {
            radius: 0.0,
            shape: ApertureShape::Disc,
            highlight_boost: 0.0,
        }),
        Filter::Optics(Optics::RadialBlur {
            center: Point::new(16.0, 16.0),
            strength: 0.0,
            samples: 8,
            decay: 1.0,
        }),
        Filter::Optics(Optics::Anamorphic {
            threshold: 0.0,
            length: 6.0,
            stretch: 8.0,
            tint: Color::WHITE,
            intensity: 0.0,
        }),
        Filter::Custom(vec![
            FilterOperation::Transform(Transform::IDENTITY),
            FilterOperation::BlurXY {
                sigma_x: 0.0,
                sigma_y: 0.0,
            },
        ]),
    ] {
        let actual = prepare(source.clone().filter(filter))
            .render_linear(limits)
            .unwrap();
        assert_eq!(reference, actual);
    }
    assert!(
        Filter::Optics(Optics::Bokeh {
            radius: -1.0,
            shape: ApertureShape::Disc,
            highlight_boost: 0.0
        })
        .validate("test")
        .is_err()
    );
    assert!(Filter::halation(-1.0, 2.0).is_err());
    assert!(Filter::anamorphic(-1.0, 2.0, 1.0, Color::WHITE).is_err());
    assert!(
        Filter::Custom(vec![FilterOperation::Transform(Transform::scale(0.0))])
            .validate("test")
            .is_err()
    );
}

#[test]
fn anamorphic_streak_extends_bright_highlights_horizontally_and_validates() {
    use motion_core::{Filter, NodeModifiers, Optics};
    let streak = |intensity: f64, threshold: f64| {
        let node: Node = Rect::new(14.0, 15.0, 18.0, 17.0)
            .fill(Color::rgb(255, 255, 255))
            .into();
        let node = Node::group([
            Rect::new(0.0, 0.0, 32.0, 32.0)
                .fill(Color::rgb(0, 0, 0))
                .into(),
            node.filter(Filter::ColorAdjust {
                exposure: 2.0,
                contrast: 1.0,
                saturation: 1.0,
                hue_degrees: 0.0,
            }),
        ]);
        prepare(node.filter(Filter::Optics(Optics::Anamorphic {
            threshold,
            length: 4.0,
            stretch: 16.0,
            tint: Color::rgb(80, 160, 255),
            intensity,
        })))
        .render_linear(RasterLimits::default())
        .unwrap()
    };
    let base = streak(0.0, 0.5);
    let lit = streak(1.0, 0.5);
    let at = |frame: &motion_render::FrameBuffer, x: u32, y: u32| frame.linear_pixel(x, y).unwrap();
    // The streak reaches far left/right of the 4px highlight but barely above and below it.
    assert!(at(&lit, 24, 16)[2] > at(&base, 24, 16)[2] + 1e-3);
    assert!(at(&lit, 8, 16)[2] > at(&base, 8, 16)[2] + 1e-3);
    assert!(
        at(&lit, 24, 16)[2] > at(&lit, 24, 16)[0],
        "tint favours blue"
    );
    assert!(at(&lit, 16, 26)[2] < (at(&lit, 24, 16)[2] * 0.25).max(1e-4));
    // Above the highlight's luminance nothing is extracted.
    let none = streak(1.0, 1.0e6);
    assert_eq!(base, none);
    for invalid in [
        Optics::Anamorphic {
            threshold: -1.0,
            length: 1.0,
            stretch: 2.0,
            tint: Color::WHITE,
            intensity: 1.0,
        },
        Optics::Anamorphic {
            threshold: 0.5,
            length: f64::NAN,
            stretch: 2.0,
            tint: Color::WHITE,
            intensity: 1.0,
        },
        Optics::Anamorphic {
            threshold: 0.5,
            length: 1.0,
            stretch: 0.5,
            tint: Color::WHITE,
            intensity: 1.0,
        },
        Optics::Anamorphic {
            threshold: 0.5,
            length: 1.0,
            stretch: 2.0,
            tint: Color::WHITE,
            intensity: -1.0,
        },
    ] {
        assert!(Filter::Optics(invalid).validate("test").is_err());
    }
    let bounds = Optics::Anamorphic {
        threshold: 0.5,
        length: 4.0,
        stretch: 4.0,
        tint: Color::WHITE,
        intensity: 1.0,
    }
    .bounds(Rect::new(10.0, 10.0, 20.0, 20.0));
    assert_eq!((bounds.left, bounds.right), (-2.0, 32.0));
    assert_eq!((bounds.top, bounds.bottom), (7.0, 23.0));
}

#[test]
fn elevation_shadow_renders_below_the_card_and_spreads_with_height() {
    use motion_core::{Elevation, NodeModifiers};
    let alpha_at = |height: f64, x: u32, y: u32| {
        let card: Node = Rect::new(10.0, 8.0, 22.0, 16.0).fill(Color::WHITE).into();
        let frame = prepare(card.elevated(Elevation::shadow(height).unwrap()))
            .render_linear(RasterLimits::default())
            .unwrap();
        frame.linear_pixel(x, y).unwrap()[3]
    };
    // Just under the card bottom edge and far below it, outside the card itself.
    let (near_low, far_low) = (alpha_at(2.0, 16, 16), alpha_at(2.0, 16, 26));
    let (near_high, far_high) = (alpha_at(10.0, 16, 16), alpha_at(10.0, 16, 26));
    assert!(near_low > 0.1 && near_high > 0.1);
    assert!(far_high > far_low + 0.02, "{far_low} {far_high}");
    assert_eq!(alpha_at(0.0, 16, 16), 0.0);
    assert_eq!(alpha_at(6.0, 16, 12), 1.0);
}

proptest! {
    #![proptest_config({
        let mut config = ProptestConfig::default();
        if std::env::var_os("PROPTEST_CASES").is_none() {
            config.cases = 128;
        }
        config
    })]

    #[test]
    fn compact_rectangle_coverage_matches_independent_pixel_areas(
        left in -64_i16..=512,
        top in -64_i16..=512,
        width in 1_u16..=256,
        height in 1_u16..=256,
    ) {
        let left = f64::from(left) / 16.0;
        let top = f64::from(top) / 16.0;
        let right = left + f64::from(width) / 16.0;
        let bottom = top + f64::from(height) / 16.0;
        let actual = coverage(&prepare(Rect::new(left, top, right, bottom).fill(Color::WHITE).into()));
        for row in 0..32 {
            for column in 0..32 {
                let horizontal = (right.min(f64::from(column + 1)) - left.max(f64::from(column))).max(0.0);
                let vertical = (bottom.min(f64::from(row + 1)) - top.max(f64::from(row))).max(0.0);
                let expected = (horizontal * vertical * 255.0).round() as u8;
                prop_assert_eq!(actual[(row * 32 + column) as usize], expected);
            }
        }
    }

    #[test]
    fn polygon_coverage_is_finite_bounded_and_consistent_with_records(
        points in prop::array::uniform6(-64_i16..=576),
    ) {
        let mut path = Path2D::new();
        path.try_move_to(Point::new(f64::from(points[0]) / 16.0, f64::from(points[1]) / 16.0)).unwrap();
        for pair in points[2..].as_chunks::<2>().0 {
            path.try_line_to(Point::new(f64::from(pair[0]) / 16.0, f64::from(pair[1]) / 16.0)).unwrap();
        }
        path.try_close().unwrap();
        let frame = prepare(path.fill(Color::WHITE).into());
        let expected = coverage(&frame);
        let linear = frame.render_linear(RasterLimits::default()).unwrap();
        for row in 0..32 {
            for column in 0..32 {
                let pixel = linear.linear_pixel(column, row).unwrap();
                prop_assert!(pixel.iter().all(|value| value.is_finite() && (0.0..=1.0).contains(value)));
                let alpha = f32::from(expected[(row * 32 + column) as usize]) / 255.0;
                prop_assert!((pixel[3] - alpha).abs() <= f32::EPSILON);
                prop_assert!(pixel[..3].iter().all(|value| (*value - alpha).abs() <= f32::EPSILON));
            }
        }
    }
}
