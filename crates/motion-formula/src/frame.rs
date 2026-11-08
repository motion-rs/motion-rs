//! Conversion of laid-out Typst frames into vector groups and formula symbols.

use crate::FormulaCompileError;
use crate::compiler::Input;
use motion_content::{VectorGroup, VectorItem, VectorPath};
use motion_core::{Color, FillRule, Paint, Path2D, Point, Rect, Transform, ValidationError};
use rustybuzz::ttf_parser::{GlyphId, OutlineBuilder};
use std::{ops::Range, sync::Arc};
use typst::layout::{Frame, FrameItem};

/// Source-bearing glyph or structural mathematical rule occurrence.
#[derive(Clone, Debug)]
pub struct FormulaSymbol {
    /// Stable paint-order ID within this compiled resource.
    pub id: Arc<str>,
    /// Original UTF-8 math source span, when the compiler supplies provenance.
    pub source: Option<Range<usize>>,
    /// Actual rendered glyph text, or empty for a structural line/shape.
    pub text: Arc<str>,
    /// NFKC semantic characters for style-independent character matching.
    pub character: Arc<str>,
    /// Whether this occurrence is a layout rule/shape rather than a glyph.
    pub structural: bool,
    /// Local family used by the math shaper, empty for structural rules.
    pub family: Arc<str>,
}

struct Pen {
    path: Path2D,
    error: Option<ValidationError>,
}

impl OutlineBuilder for Pen {
    fn move_to(&mut self, horizontal: f32, vertical: f32) {
        let result = self
            .path
            .try_move_to(Point::new(f64::from(horizontal), f64::from(vertical)))
            .map(|_| ());
        if let Err(error) = result {
            self.error = Some(error);
        }
    }

    fn line_to(&mut self, horizontal: f32, vertical: f32) {
        if let Err(error) = self
            .path
            .try_line_to(Point::new(f64::from(horizontal), f64::from(vertical)))
        {
            self.error = Some(error);
        }
    }

    fn quad_to(&mut self, control_x: f32, control_y: f32, end_x: f32, end_y: f32) {
        if let Err(error) = self.path.try_quadratic_to(
            Point::new(f64::from(control_x), f64::from(control_y)),
            Point::new(f64::from(end_x), f64::from(end_y)),
        ) {
            self.error = Some(error);
        }
    }

    fn curve_to(
        &mut self,
        first_x: f32,
        first_y: f32,
        second_x: f32,
        second_y: f32,
        end_x: f32,
        end_y: f32,
    ) {
        if let Err(error) = self.path.try_cubic_to(
            Point::new(f64::from(first_x), f64::from(first_y)),
            Point::new(f64::from(second_x), f64::from(second_y)),
            Point::new(f64::from(end_x), f64::from(end_y)),
        ) {
            self.error = Some(error);
        }
    }

    fn close(&mut self) {
        if let Err(error) = self.path.try_close() {
            self.error = Some(error);
        }
    }
}

fn point(value: typst::layout::Point) -> Point {
    Point::new(value.x.to_pt(), value.y.to_pt())
}

fn matrix(value: typst::layout::Transform) -> Transform {
    Transform::from_coefficients([
        value.sx.get(),
        value.ky.get(),
        value.kx.get(),
        value.sy.get(),
        value.tx.to_pt(),
        value.ty.to_pt(),
    ])
}

fn curve(source: &typst::visualize::Curve) -> Result<Path2D, ValidationError> {
    let mut path = Path2D::new();
    for item in &source.0 {
        match item {
            typst::visualize::CurveItem::Move(end) => path.try_move_to(point(*end)),
            typst::visualize::CurveItem::Line(end) => path.try_line_to(point(*end)),
            typst::visualize::CurveItem::Cubic(first, second, end) => {
                path.try_cubic_to(point(*first), point(*second), point(*end))
            }
            typst::visualize::CurveItem::Close => path.try_close(),
        }?;
    }
    Ok(path)
}

fn paint(source: &typst::visualize::Paint) -> Result<Paint, FormulaCompileError> {
    let typst::visualize::Paint::Solid(color) = source else {
        return Err(FormulaCompileError::new(
            "FormulaAsset conversion accepts solid paints only",
        ));
    };
    let color = color.to_rgb();
    Ok(Color::from_rgb_float(
        f64::from(color.red),
        f64::from(color.green),
        f64::from(color.blue),
    )
    .with_alpha(f64::from(color.alpha))
    .into())
}

fn stroke(
    source: &typst::visualize::FixedStroke,
) -> Result<(motion_core::StrokeStyle, Paint), FormulaCompileError> {
    if source.dash.is_some() {
        return Err(FormulaCompileError::new(
            "Dashed formula rules are unsupported",
        ));
    }
    let style = motion_core::StrokeStyle {
        width: source.thickness.to_pt(),
        cap: match source.cap {
            typst::visualize::LineCap::Butt => motion_core::LineCap::Butt,
            typst::visualize::LineCap::Round => motion_core::LineCap::Round,
            typst::visualize::LineCap::Square => motion_core::LineCap::Square,
        },
        join: match source.join {
            typst::visualize::LineJoin::Miter => motion_core::LineJoin::Miter {
                limit: source.miter_limit.get(),
            },
            typst::visualize::LineJoin::Round => motion_core::LineJoin::Round,
            typst::visualize::LineJoin::Bevel => motion_core::LineJoin::Bevel,
        },
    };
    Ok((style, paint(&source.paint)?))
}

fn walk(
    frame: &Frame,
    input: &Input,
    symbols: &mut Vec<FormulaSymbol>,
    count: &mut usize,
    depth: usize,
    parent: Transform,
    baseline: &mut Option<f64>,
) -> Result<VectorGroup, FormulaCompileError> {
    if depth > 40 || *count > 4096 {
        return Err(FormulaCompileError::new(
            "FormulaAsset frame exceeds depth/item allowance",
        ));
    }
    if baseline.is_none() && frame.has_baseline() {
        *baseline = Some(parent.apply(Point::new(0.0, frame.baseline().to_pt())).y * 4.0 / 3.0);
    }
    let mut children = Vec::new();
    for (position, item) in frame.items() {
        *count += 1;
        if *count > 4096 || symbols.len() > 2048 {
            return Err(FormulaCompileError::new(
                "FormulaAsset exceeds item/symbol allowance",
            ));
        }
        let offset = point(*position);
        let translation = Transform::translate(offset.x, offset.y);
        let content = match item {
            FrameItem::Group(group) => {
                let transform = matrix(group.transform).then(translation);
                let mut child = walk(
                    &group.frame,
                    input,
                    symbols,
                    count,
                    depth + 1,
                    transform.then(parent),
                    baseline,
                )?;
                child.transform = transform;
                if let Some(clip) = &group.clip {
                    child.clip = Some(curve(clip)?.into());
                }
                VectorItem::Group(child)
            }
            FrameItem::Text(text) => convert_text(text, input, symbols, translation)?,
            FrameItem::Shape(shape, span) => {
                let path = match &shape.geometry {
                    typst::visualize::Geometry::Line(end) => {
                        let mut path = Path2D::new();
                        path.try_move_to(Point::ORIGIN)?;
                        path.try_line_to(point(*end))?;
                        path
                    }
                    typst::visualize::Geometry::Rect(size) => {
                        Path2D::from_rect(Rect::new(0.0, 0.0, size.x.to_pt(), size.y.to_pt()))?
                    }
                    typst::visualize::Geometry::Curve(source) => curve(source)?,
                };
                let id: Arc<str> = format!("symbol-{}", symbols.len()).into();
                let source = input.range(*span);
                symbols.push(FormulaSymbol {
                    id: id.clone(),
                    source: source.clone(),
                    text: "".into(),
                    character: "".into(),
                    structural: true,
                    family: "".into(),
                });
                let path = VectorPath {
                    id: None,
                    path: Arc::new(path),
                    fill: shape
                        .fill
                        .as_ref()
                        .map(|fill| {
                            paint(fill).map(|fill| {
                                (
                                    fill,
                                    match shape.fill_rule {
                                        typst::visualize::FillRule::NonZero => FillRule::NonZero,
                                        typst::visualize::FillRule::EvenOdd => FillRule::EvenOdd,
                                    },
                                )
                            })
                        })
                        .transpose()?,
                    stroke: shape.stroke.as_ref().map(stroke).transpose()?,
                    source: None,
                };
                VectorItem::Group(VectorGroup {
                    id: Some(id),
                    source,
                    transform: translation,
                    children: vec![VectorItem::Path(path)],
                    ..Default::default()
                })
            }
            FrameItem::Tag(_) | FrameItem::Link(..) => continue,
            FrameItem::Image(..) => {
                return Err(FormulaCompileError::new(
                    "Images are outside mathematical formula input",
                ));
            }
        };
        children.push(content);
    }
    Ok(VectorGroup {
        children,
        ..Default::default()
    })
}

fn convert_text(
    text: &typst::text::TextItem,
    input: &Input,
    symbols: &mut Vec<FormulaSymbol>,
    translation: Transform,
) -> Result<VectorItem, FormulaCompileError> {
    let mut glyphs = Vec::new();
    let mut cursor = [0.0; 2];
    let scale = text.size.to_pt() / text.font.units_per_em();
    for glyph in &text.glyphs {
        if glyph.id == 0 {
            return Err(FormulaCompileError::new("Math font has a missing glyph"));
        }
        let mut pen = Pen {
            path: Path2D::new(),
            error: None,
        };
        let outlined = text
            .font
            .ttf()
            .outline_glyph(GlyphId(glyph.id), &mut pen)
            .is_some();
        if let Some(error) = pen.error {
            return Err(error.into());
        }
        let rendered = text.text.get(glyph.range()).unwrap_or("");
        if !outlined && !rendered.chars().all(char::is_whitespace) {
            return Err(FormulaCompileError::new(
                "Math color/bitmap or missing-outline glyph is unsupported",
            ));
        }

        use unicode_normalization::UnicodeNormalization;
        let id: Arc<str> = format!("symbol-{}", symbols.len()).into();
        let character: Arc<str> = rendered.nfkc().collect::<String>().into();
        let source = input.range(glyph.span.0).map(|range| {
            let start = range.start + usize::from(glyph.span.1);
            let end = start + character.len();
            if !character.is_empty()
                && end <= range.end
                && input
                    .source
                    .text()
                    .get(input.body.start + start..input.body.start + end)
                    == Some(character.as_ref())
            {
                start..end
            } else {
                range
            }
        });
        symbols.push(FormulaSymbol {
            id: id.clone(),
            source: source.clone(),
            text: rendered.into(),
            character,
            structural: false,
            family: text.font.font().info().family.clone().into(),
        });
        let transform = Transform::scale_non_uniform(scale, -scale).then(Transform::translate(
            cursor[0] + glyph.x_offset.at(text.size).to_pt(),
            -(cursor[1] + glyph.y_offset.at(text.size).to_pt()),
        ));
        let mut path = VectorPath::filled(pen.path, paint(&text.fill)?);
        if let Some(style) = &text.stroke {
            let (mut style, paint) = stroke(style)?;
            style.width /= scale;
            path.stroke = Some((style, paint));
        }
        glyphs.push(VectorItem::Group(VectorGroup {
            id: Some(id),
            source,
            transform,
            children: vec![VectorItem::Path(path)],
            ..Default::default()
        }));
        cursor[0] += glyph.x_advance.at(text.size).to_pt();
        cursor[1] += glyph.y_advance.at(text.size).to_pt();
    }
    Ok(VectorItem::Group(VectorGroup {
        transform: translation,
        children: glyphs,
        ..Default::default()
    }))
}

pub(super) fn convert(
    frame: &Frame,
    input: &Input,
) -> Result<(VectorGroup, Vec<FormulaSymbol>, Option<f64>), FormulaCompileError> {
    let mut symbols = Vec::new();
    let mut count = 0;
    let mut baseline = None;
    let mut root = walk(
        frame,
        input,
        &mut symbols,
        &mut count,
        0,
        Transform::IDENTITY,
        &mut baseline,
    )?;
    root.transform = Transform::scale(4.0 / 3.0);
    symbols.sort_by_key(|symbol| {
        symbol
            .source
            .as_ref()
            .map(|source| source.start)
            .unwrap_or(usize::MAX)
    });
    Ok((root, symbols, baseline))
}
