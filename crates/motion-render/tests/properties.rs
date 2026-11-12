//! Property tests comparing prepared coverage with CPU raster output.

use motion_core::{
    Color, Node, PixelSize, Point, RasterLimits, Rect, Shape, Size, ValidationLimits,
};
use motion_render::{PreparedCpuFrame, RenderPrepare};

fn prepare(node: Node) -> PreparedCpuFrame {
    node.compile(
        Size::new(32.0, 32.0),
        PixelSize::new(32, 32),
        ValidationLimits::default(),
    )
    .unwrap()
    .prepare(ValidationLimits::default(), RasterLimits::default())
    .unwrap()
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
