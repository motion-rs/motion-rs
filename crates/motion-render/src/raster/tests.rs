use super::*;
use crate::prepared::PreparedLayer;
use motion_core::{
    Color, DisplayList, DrawCommand, Paint, PixelSize, Rect, ValidationError, ValidationErrorKind,
    ValidationLimits,
};
use motion_core::{FillRule, Path2D, Transform};
use std::sync::Arc;

fn list(bounds: Rect, transform: Transform) -> DisplayList {
    DisplayList {
        output_size: PixelSize::new(16, 9),
        background: None,
        commands: vec![DrawCommand::FillRect {
            bounds,
            transform,
            paint: Paint::Solid(Color::WHITE),
        }],
    }
}

fn prepare(list: &DisplayList) -> Result<PreparedFrame, ValidationError> {
    list.prepare(ValidationLimits::default(), RasterLimits::default())
}

pub(crate) fn coverage(frame: &PreparedFrame) -> Vec<u8> {
    let mut result = vec![0; (frame.size.width * frame.size.height) as usize];
    let mut visits = result.clone();
    for strip in &frame.strips {
        assert_eq!(strip.y % 4, 0);
        for row in 0..strip.height {
            for column in 0..strip.width {
                let offset = ((strip.y + row) * frame.size.width + strip.x + column) as usize;
                visits[offset] += 1;
                result[offset] =
                    frame.alphas[strip.alpha_offset + (row * strip.width + column) as usize];
            }
        }
    }
    for span in &frame.spans {
        assert_eq!((span.start_x % 4, span.end_x % 4, span.y % 4), (0, 0, 0));
        for row in span.y..span.y + span.height {
            for column in span.start_x..span.end_x {
                let offset = (row * frame.size.width + column) as usize;
                visits[offset] += 1;
                result[offset] = 255;
            }
        }
    }
    assert!(visits.into_iter().all(|count| count <= 1));
    result
}

pub(crate) fn rectangle_cases() -> Vec<Rect> {
    let mut cases = vec![
        Rect::new(1.25, 0.5, 14.75, 8.25),
        Rect::new(0.0, 0.0, 16.0, 9.0),
        Rect::new(-2.25, -0.5, 9.5, 20.0),
        Rect::new(4.0, 4.0, 4.25, 4.5),
    ];
    for left in [-1.25, 0.0, 0.25, 3.999999, 4.0, 8.0] {
        for width in [0.0, 0.25, 3.0, 7.75, 16.0] {
            for top in [-0.5, 0.0, 3.5, 4.0, 8.75] {
                for height in [0.0, 0.25, 1.5, 4.0, 11.0] {
                    cases.push(Rect::new(left, top, left + width, top + height));
                }
            }
        }
    }
    cases
}

#[test]
fn rectangle_strips_and_spans_match_analytic_area_without_overlap() {
    for bounds in rectangle_cases() {
        let frame = prepare(&list(bounds, Transform::IDENTITY)).unwrap();
        let actual = coverage(&frame);
        for row in 0..9 {
            for column in 0..16 {
                let width = (bounds.right.min(f64::from(column + 1))
                    - bounds.left.max(f64::from(column)))
                .max(0.0);
                let height = (bounds.bottom.min(f64::from(row + 1))
                    - bounds.top.max(f64::from(row)))
                .max(0.0);
                assert_eq!(
                    actual[(row * 16 + column) as usize],
                    (width * height * 255.0).round() as u8
                );
            }
        }
    }
    let full = prepare(&list(Rect::new(0.0, 0.0, 16.0, 9.0), Transform::IDENTITY)).unwrap();
    assert!(full.strips.is_empty());
    assert!(full.alphas.is_empty());
    assert_eq!(full.spans.len(), 3);
}

#[test]
fn reflections_axis_swaps_singular_and_offscreen_geometry_are_explicit() {
    let bounds = Rect::new(1.0, 2.0, 5.0, 7.0);
    let transform = Transform::scale_non_uniform(-1.0, 1.0).then(Transform::translate(10.0, 0.0));
    let reflected = prepare(&list(bounds, transform)).unwrap();
    assert_eq!(
        coverage(&reflected),
        coverage(&prepare(&list(Rect::new(5.0, 2.0, 9.0, 7.0), Transform::IDENTITY)).unwrap())
    );
    let transform =
        Transform::rotate(std::f64::consts::FRAC_PI_2).then(Transform::translate(10.0, 0.0));
    assert_eq!(
        coverage(&prepare(&list(bounds, transform)).unwrap()),
        coverage(&prepare(&list(Rect::new(3.0, 1.0, 8.0, 5.0), Transform::IDENTITY)).unwrap())
    );
    for (bounds, transform) in [
        (bounds, Transform::scale(0.0)),
        (Rect::new(1.0e40, 0.0, 2.0e40, 1.0), Transform::IDENTITY),
        (Rect::new(0.25, 0.0, 0.25, 9.0), Transform::IDENTITY),
    ] {
        let frame = prepare(&list(bounds, transform)).unwrap();
        assert_eq!(frame.items.len(), 1);
        assert!(frame.strips.is_empty() && frame.spans.is_empty());
    }
}

#[test]
fn layer_metadata_and_item_indices_preserve_original_order() {
    let mut source = list(Rect::new(0.0, 0.0, 4.0, 4.0), Transform::IDENTITY);
    source.commands.insert(
        0,
        DrawCommand::PushLayer {
            opacity: f64::from_bits(1),
            bounds: Rect::new(-2.5, 1.25, 8.125, 20.0),
            blend_mode: motion_core::drawing::BlendMode::Multiply,
            clip: None,
        },
    );
    source.commands.push(DrawCommand::PopLayer);
    let frame = prepare(&source).unwrap();
    assert_eq!(frame.items.len(), source.commands.len());
    assert!(
        matches!(frame.items[0], PreparedItem::PushLayer { layer: PreparedLayer { opacity, bounds, blend_mode, .. } } if opacity == f64::from_bits(1) && bounds == PixelRect { left: 0, top: 1, right: 9, bottom: 9 } && blend_mode == motion_core::drawing::BlendMode::Multiply)
    );
    assert!(matches!(
        frame.items[1],
        PreparedItem::Fill { paint: 0, .. }
    ));
    assert!(matches!(frame.items[2], PreparedItem::PopLayer));
    assert_eq!(frame, prepare(&source).unwrap());
}

#[test]
fn curves_are_admitted_but_invalid_layers_are_never_hidden() {
    let mut source = list(Rect::new(0.0, 0.0, 4.0, 4.0), Transform::rotate(0.25));
    assert!(prepare(&source).is_ok());
    let mut path = Path2D::new();
    path.try_move_to(motion_core::Point::new(100.0, 100.0))
        .unwrap();
    path.try_quadratic_to(
        motion_core::Point::new(102.0, 104.0),
        motion_core::Point::new(104.0, 100.0),
    )
    .unwrap();
    let path = Arc::new(path);
    for rule in [FillRule::NonZero, FillRule::EvenOdd] {
        source.commands = vec![
            DrawCommand::PushLayer {
                opacity: 0.0,
                bounds: Rect::default(),
                blend_mode: motion_core::drawing::BlendMode::SourceOver,
                clip: None,
            },
            DrawCommand::Fill {
                path: path.clone(),
                transform: Transform::IDENTITY,
                paint: Paint::Solid(Color::WHITE),
                rule,
            },
            DrawCommand::PopLayer,
        ];
        assert_eq!(prepare(&source).unwrap().items.len(), 3);
    }
    source.commands[0] = DrawCommand::PopLayer;
    assert_eq!(prepare(&source).unwrap_err().field, "layers");
}

#[test]
fn pixel_byte_and_work_limits_reject_one_unit_short_and_recover() {
    let source = list(Rect::new(0.25, 0.5, 15.75, 8.5), Transform::IDENTITY);
    let frame = prepare(&source).unwrap();
    let exact = RasterLimits {
        max_pixels: 144,
        max_bytes: frame.stats.peak_bytes,
        max_work: frame.stats.work,
    };
    assert_eq!(
        source.prepare(ValidationLimits::default(), exact).unwrap(),
        frame
    );
    for (limits, field) in [
        (
            RasterLimits {
                max_pixels: 143,
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
        assert_eq!(
            source
                .prepare(ValidationLimits::default(), limits)
                .unwrap_err()
                .field,
            field
        );
        assert_eq!(prepare(&source).unwrap(), frame);
    }
}

#[test]
fn extreme_output_coordinates_and_invalid_limits_fail_before_pixel_allocation() {
    let mut source = list(
        Rect::new(f64::from(u32::MAX - 3), 0.0, f64::from(u32::MAX), 1.0),
        Transform::IDENTITY,
    );
    source.output_size = PixelSize::new(u32::MAX, 1);
    assert_eq!(prepare(&source).unwrap_err().field, "max_pixels");
    let wide = RasterLimits {
        max_pixels: u64::MAX,
        max_work: u64::MAX,
        ..RasterLimits::default()
    };
    let prepared = source.prepare(ValidationLimits::default(), wide).unwrap();
    assert_eq!(prepared.strips[0].x, u32::MAX - 3);
    assert_eq!(prepared.alphas, vec![255; 3]);
    assert_eq!(
        prepared.compose_linear(wide).unwrap_err().field,
        "max_bytes"
    );
    source.commands.clear();
    source.output_size = PixelSize::new(u32::MAX, u32::MAX);
    assert_eq!(
        source
            .prepare(ValidationLimits::default(), wide)
            .unwrap()
            .compose_linear(wide)
            .unwrap_err()
            .field,
        "max_bytes"
    );
    source.output_size = PixelSize::new(1, 1);
    for (limits, field) in [
        (
            RasterLimits {
                max_pixels: 0,
                ..wide
            },
            "max_pixels",
        ),
        (
            RasterLimits {
                max_bytes: 0,
                ..wide
            },
            "max_bytes",
        ),
        (
            RasterLimits {
                max_work: 0,
                ..wide
            },
            "max_work",
        ),
    ] {
        let error = source
            .prepare(ValidationLimits::default(), limits)
            .unwrap_err();
        assert_eq!(error.field, field);
        assert_eq!(error.kind, ValidationErrorKind::InvalidValue);
    }
}

#[test]
fn linear_paints_keep_extended_channels_and_reject_working_overflow() {
    let mut source = list(Rect::default(), Transform::IDENTITY);
    source.background = Some(Color::from_rgb_float(1.25, -0.125, 0.333).with_alpha(0.5));
    let frame = prepare(&source).unwrap();
    let color = frame.background.unwrap().linear_premultiplied;
    assert!(color[0] > 0.5 && color[1] < 0.0);
    assert_eq!(color[3], 0.5);
    source.background = Some(Color::from_rgb_float(1.0e30, 0.0, 0.0));
    assert_eq!(prepare(&source).unwrap_err().field, "paint");
    source.background = source.background.map(|color| color.with_alpha(0.0));
    assert_eq!(
        prepare(&source)
            .unwrap()
            .background
            .unwrap()
            .linear_premultiplied,
        [0.0; 4]
    );
}

#[test]
fn mesh_paint_evaluates_analytic_patches_and_validates_explicit_bake_resolution() {
    use motion_core::{MeshGradient, MeshPaint};
    let mesh = MeshGradient::grid(
        2,
        2,
        &[
            Color::from_hex(0xff0000),
            Color::from_hex(0x00ff00),
            Color::from_hex(0x0000ff),
            Color::WHITE,
        ],
    )
    .unwrap();
    let bounds = Rect::new(2.0, 1.0, 14.0, 8.0);
    let size = PixelSize::new(16, 16);
    let mut source = list(bounds, Transform::IDENTITY);
    let pattern = mesh
        .pattern(bounds, size, 4, RasterLimits::default())
        .unwrap();
    source.commands = vec![DrawCommand::FillRect {
        bounds,
        transform: Transform::IDENTITY,
        paint: Paint::Pattern(pattern),
    }];
    let explicit = prepare(&source).unwrap();
    source.commands = vec![DrawCommand::FillRect {
        bounds,
        transform: Transform::IDENTITY,
        paint: Paint::Mesh(MeshPaint::new(mesh.clone(), size, 4)),
    }];
    let analytic = prepare(&source).unwrap();
    let patch = mesh.patch(0, 0).unwrap();
    let mut differs_from_bake = false;
    for point in [(3.0, 2.0), (8.5, 4.5), (13.5, 7.5)] {
        let point = motion_core::Point::new(point.0, point.1);
        let expected = patch
            .sample(motion_core::Point::new(
                (point.x - bounds.left) / bounds.width(),
                (point.y - bounds.top) / bounds.height(),
            ))
            .unwrap()
            .unwrap();
        assert_eq!(analytic.paint_at(0, point).unwrap(), expected);
        differs_from_bake |=
            analytic.paint_at(0, point).unwrap() != explicit.paint_at(0, point).unwrap();
    }
    assert!(
        differs_from_bake,
        "Authored meshes must not sample a baked image"
    );
    source.commands = vec![DrawCommand::FillRect {
        bounds,
        transform: Transform::IDENTITY,
        paint: Paint::Mesh(MeshPaint::new(mesh, size, 0)),
    }];
    assert!(prepare(&source).is_err());
}
