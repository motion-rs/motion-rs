//! Text paragraphs, rich styles and grapheme-aware stateless part animation.
//!
//! ```no_run
//! use motion_content::prelude::*;
//! use motion_core::prelude::*;
//! use motion_core::{
//!     DisplayList, RasterLimits, ResourceStore, ValidatedDisplayList, MAX_PATH_VERBS,
//!     MAX_STROKE_VERBS, MAX_SVG_PATH_BYTES, sample_offsets,
//! };
//! use motion_render::{Composition, RenderPrepare, RenderOptions, BackendKind, render_still};
//! let mut fonts = FontLibrary::new();
//! fonts.load_file("assets/project.ttf")?;
//! let resources = Assets::new(".")?.with_fonts(fonts);
//! let text = Text::rich([
//!     TextSpan::new("Hello, ").fill(Color::WHITE),
//!     TextSpan::new("motion-rs").weight(FontWeight::BOLD),
//! ])
//! .size(28.0)
//! .max_width(300.0);
//! let frame = Composition::new(320, 120, motion_render::FrameClock::new(fps!(30), 150))
//!     .configure(|options| options.resources(resources))
//!     .render_frame(0, Size::new(320.0, 120.0), |_| text.clone().into())?;
//! assert_eq!(frame.size().width, 320);
//! # Ok::<(), motion_core::Error>(())
//! ```

use crate::FontLibrary;

use motion_core::{
    Canvas, Color, DeferredContent, DrawContext, Drawable, FillRule, Node, NodeModifiers, Paint,
    Rect, Time, Transform, ValidationError,
};
use std::{
    collections::BTreeMap,
    ops::Range,
    sync::{Arc, Mutex},
};
use unicode_segmentation::UnicodeSegmentation;

mod drawable;
pub(crate) mod glyph;
mod layout;
mod paragraph;
mod parts;
#[cfg(test)]
mod tests;

pub use cosmic_text::Weight as FontWeight;
pub use paragraph::ParagraphStyle;
pub(crate) use paragraph::{ParagraphSpec, SharedLayouts, SpanSpec};
pub use parts::{PartStyle, TextPart};

/// Paragraph alignment, with start/end resolved by the paragraph direction.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum TextAlignment {
    /// Follow paragraph start.
    #[default]
    Start,
    /// Follow paragraph end.
    End,
    /// Center each line within the requested width.
    Center,
    /// Justify nonfinal lines.
    Justified,
}

/// Units used for visibility and per-part animation.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum TextSplit {
    /// Extended Unicode grapheme clusters, not scalar chars.
    #[default]
    Characters,
    /// Unicode words, with separators attached to the preceding word.
    Words,
    /// Wrapped visual lines.
    Lines,
}

/// span style; unset properties inherit paragraph defaults.
#[derive(Clone, Debug)]
pub struct TextSpan {
    text: Arc<str>,
    family: Option<Arc<str>>,
    size: Option<f64>,
    weight: Option<FontWeight>,
    italic: bool,
    fill: Option<Paint>,
}

impl TextSpan {
    /// Creates an inheriting UTF-8 span.
    pub fn new(text: impl Into<Arc<str>>) -> Self {
        Self {
            text: text.into(),
            family: None,
            size: None,
            weight: None,
            italic: false,
            fill: None,
        }
    }

    /// Overrides family name.
    pub fn family(mut self, family: impl Into<Arc<str>>) -> Self {
        self.family = Some(family.into());
        self
    }

    /// Overrides logical font size.
    pub fn size(mut self, size: f64) -> Self {
        self.size = Some(size);
        self
    }

    /// Overrides OpenType weight.
    pub fn weight(mut self, weight: FontWeight) -> Self {
        self.weight = Some(weight);
        self
    }

    /// Requests an italic face.
    pub fn italic(mut self, italic: bool) -> Self {
        self.italic = italic;
        self
    }

    /// Overrides fill paint.
    pub fn fill(mut self, paint: impl Into<Paint>) -> Self {
        self.fill = Some(paint.into());
        self
    }
}

/// Public glyph provenance and logical ink/hit bounds from one paragraph layout.
#[derive(Clone, Debug)]
pub struct TextGlyph {
    /// Original UTF-8 cluster byte range.
    pub source: Range<usize>,
    /// Wrapped visual line number.
    pub line: usize,
    /// Explicit/fallback font family selected by the shaper.
    pub family: Arc<str>,
    /// Local glyph hit/ink bounds, including baseline placement.
    pub bounds: Rect,
    artwork: Arc<glyph::Artwork>,
    transform: Transform,
    span: usize,
}

/// Immutable paragraph geometry; color/part transforms do not reshape it.
#[derive(Clone, Debug)]
pub struct ParagraphLayout {
    glyphs: Arc<[TextGlyph]>,
    bounds: Rect,
    baseline: f64,
    lines: Arc<[Rect]>,
    line_sources: Arc<[Range<usize>]>,
    text: Arc<str>,
}

impl ParagraphLayout {
    /// Logical bounds enclosing line boxes and glyph ink.
    pub fn bounds(&self) -> Rect {
        self.bounds
    }

    /// Real first-line baseline in local coordinates.
    pub fn baseline(&self) -> f64 {
        self.baseline
    }

    /// Shaped glyphs and their original cluster ranges.
    pub fn glyphs(&self) -> &[TextGlyph] {
        &self.glyphs
    }

    /// Wrapped visual line boxes.
    pub fn lines(&self) -> &[Rect] {
        &self.lines
    }
}

/// Multi-line Unicode text: a [`ParagraphStyle`] shaped into a [`ParagraphLayout`],
/// plus ink and one optional reveal effect. cosmic-text is the sole paragraph
/// shaper, and unhinted skrifa outlines enter the ordinary shared fill pipeline.
/// Static drawing, handwriting and part animation all read the same layout.
#[derive(Clone, Debug)]
pub struct Text {
    paragraph: ParagraphStyle,
    fill: Paint,
    stroke: Option<(f64, Paint)>,
    reveal: Reveal,
    split: TextSplit,
    visible: Option<usize>,
}

/// The effect applied to the laid-out glyphs; the last one requested wins.
#[derive(Clone, Debug, Default)]
enum Reveal {
    #[default]
    None,
    Write(f64),
    Parts(PartAnimation),
}

#[derive(Clone)]
struct PartAnimation {
    delay: motion_core::Duration,
    style: Arc<dyn Fn(&TextPart) -> PartStyle + Send + Sync>,
}

impl std::fmt::Debug for PartAnimation {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("PartAnimation")
            .field("delay", &self.delay)
            .finish_non_exhaustive()
    }
}

impl Text {
    /// Builds and validates text using one explicit immutable font face.
    pub fn from_font(
        text: &str,
        font: &crate::FontFace,
        size: f64,
        paint: impl Into<Paint>,
    ) -> Result<Self, ValidationError> {
        if !size.is_finite() || size <= 0.0 || size > 4096.0 {
            return Err(invalid(
                "font_size",
                "Require a finite font size in (0, 4096]",
            ));
        }
        let fonts = font.library()?;
        let face = fonts
            .database()
            .faces()
            .find(|face| face.index == font.index())
            .ok_or_else(|| invalid("font", "Explicit font face is unavailable"))?;
        let family = face
            .families
            .first()
            .ok_or_else(|| invalid("font", "Explicit font face has no family"))?
            .0
            .clone();
        let span = TextSpan::new(text)
            .family(family)
            .weight(FontWeight(face.weight.0))
            .italic(face.style == fontdb::Style::Italic);
        let value = Self::rich([span]).fonts(&fonts).size(size).fill(paint);
        value.layout(&fonts)?;
        Ok(value)
    }

    /// Creates a paragraph with size 24, line height 1.3 and white fill.
    pub fn new(text: impl Into<Arc<str>>) -> Self {
        Self::from_paragraph(ParagraphStyle::new(text))
    }

    /// Creates rich spans, whose unset styles inherit paragraph defaults.
    pub fn rich(spans: impl IntoIterator<Item = TextSpan>) -> Self {
        Self::from_paragraph(ParagraphStyle::rich(spans))
    }

    /// Wraps a paragraph description with white fill and no effect.
    pub fn from_paragraph(paragraph: ParagraphStyle) -> Self {
        Self {
            paragraph,
            fill: Color::WHITE.into(),
            stroke: None,
            reveal: Reveal::None,
            split: TextSplit::Characters,
            visible: None,
        }
    }

    /// The paragraph description this text shapes.
    pub fn paragraph(&self) -> &ParagraphStyle {
        &self.paragraph
    }

    /// Selects an explicit family; empty uses the library's generic default.
    pub fn family(mut self, family: impl Into<Arc<str>>) -> Self {
        self.paragraph = self.paragraph.family(family);
        self
    }

    /// Sets finite logical size in (0,4096].
    pub fn size(mut self, size: f64) -> Self {
        self.paragraph = self.paragraph.size(size);
        self
    }

    /// Selects OpenType weight; variable weight participates in glyph identity.
    pub fn weight(mut self, weight: FontWeight) -> Self {
        self.paragraph = self.paragraph.weight(weight);
        self
    }

    /// Sets paint without invalidating shaped geometry.
    pub fn fill(mut self, paint: impl Into<Paint>) -> Self {
        self.fill = paint.into();
        self
    }

    /// Adds stroke after filling; existing bounded stroke rules apply.
    pub fn stroke(mut self, width: f64, paint: impl Into<Paint>) -> Self {
        self.stroke = Some((width, paint.into()));
        self
    }

    /// Sets a positive relative line-height multiplier.
    pub fn line_height(mut self, height: f64) -> Self {
        self.paragraph = self.paragraph.line_height(height);
        self
    }

    /// Sets logical tracking, converted to em internally for shaping.
    pub fn letter_spacing(mut self, spacing: f64) -> Self {
        self.paragraph = self.paragraph.letter_spacing(spacing);
        self
    }

    /// Wraps to finite positive logical width.
    pub fn max_width(mut self, width: f64) -> Self {
        self.paragraph = self.paragraph.max_width(width);
        self
    }

    /// Selects bidirectional-aware paragraph alignment.
    pub fn alignment(mut self, alignment: TextAlignment) -> Self {
        self.paragraph = self.paragraph.alignment(alignment);
        self
    }

    /// Binds an explicit font snapshot, overriding a DrawContext environment.
    pub fn fonts(mut self, fonts: &FontLibrary) -> Self {
        self.paragraph = self.paragraph.fonts(fonts);
        self
    }

    /// Selects grapheme, word-boundary or wrapped-line parts.
    pub fn split(mut self, split: TextSplit) -> Self {
        self.split = split;
        self
    }

    /// Keeps original layout while drawing only the first logical parts.
    pub fn visible_parts(mut self, count: usize) -> Self {
        self.visible = Some(count);
        self
    }

    /// Defers vector handwriting to compilation with the current font environment.
    /// It replaces any earlier part animation.
    pub fn write(mut self, progress: f64) -> Self {
        self.reveal = Reveal::Write(progress);
        self
    }

    /// Defers per-part style evaluation to scene-local drawing time.
    /// It replaces any earlier handwriting.
    pub fn animate_parts(
        mut self,
        delay: motion_core::Duration,
        style: impl Fn(&TextPart) -> PartStyle + Send + Sync + 'static,
    ) -> Self {
        self.reveal = Reveal::Parts(PartAnimation {
            delay,
            style: Arc::new(style),
        });
        self
    }

    /// Precomputes a paragraph limited to 1..=512 lines; see
    /// [`ParagraphStyle::fit_lines`]. Ink and effects are retained.
    pub fn fit_lines(
        &self,
        max_lines: usize,
        fonts: &FontLibrary,
    ) -> Result<(Self, bool), ValidationError> {
        self.validate_ink()?;
        let (paragraph, truncated) = self.paragraph.fit_lines(max_lines, fonts)?;
        Ok((
            Self {
                paragraph,
                ..self.clone()
            },
            truncated,
        ))
    }

    /// Shapes once per text-style/font snapshot, returning actual line/ink metrics.
    pub fn layout(&self, fonts: &FontLibrary) -> Result<Arc<ParagraphLayout>, ValidationError> {
        self.validate_ink()?;
        self.paragraph.layout(fonts)
    }

    pub(super) fn resolve(
        &self,
        context: &DrawContext,
    ) -> Result<Arc<ParagraphLayout>, ValidationError> {
        self.validate_ink()?;
        self.paragraph.resolve(context)
    }

    fn validate_ink(&self) -> Result<(), ValidationError> {
        self.fill.validate()?;
        if let Some((width, paint)) = &self.stroke {
            if !width.is_finite() || *width < 0.0 {
                return Err(invalid(
                    "stroke",
                    "Text stroke requires finite nonnegative width",
                ));
            }
            paint.validate()?;
        }
        Ok(())
    }

    /// Applies the common vector handwriting effect with default contour/fill
    /// handoff. Bind fonts on this Text or supply them in the DrawContext.
    pub fn write_with_context(
        &self,
        progress: f64,
        context: &DrawContext,
    ) -> Result<Node, ValidationError> {
        self.vector_tree(context)?.reveal(&[], Some(progress))
    }

    /// Exports original shared glyph curves, color layers, bitmap strikes and
    /// source ranges into the common vector model. No second shaping pass occurs.
    pub fn vector_tree(
        &self,
        context: &DrawContext,
    ) -> Result<Arc<crate::vector::VectorTree>, ValidationError> {
        let layout = self.resolve(context)?;
        self.vector_tree_of(&layout)
    }

    /// Exports the glyphs of an already shaped layout, honoring visibility,
    /// fills and stroke of this text.
    pub fn vector_tree_of(
        &self,
        layout: &ParagraphLayout,
    ) -> Result<Arc<crate::vector::VectorTree>, ValidationError> {
        use crate::vector::{VectorGroup, VectorImage, VectorItem, VectorPath, VectorTree};
        let mut children = Vec::new();
        for (index, glyph) in self.visible_glyphs(layout) {
            let fill = self.glyph_fill(glyph);
            let mut ink = Vec::new();
            if !glyph.artwork.path.verbs().is_empty() {
                ink.push(VectorItem::Path(VectorPath {
                    id: None,
                    path: glyph.artwork.path.clone(),
                    fill: Some((fill.clone(), FillRule::NonZero)),
                    stroke: None,
                    source: None,
                }));
            }
            for (path, color) in &glyph.artwork.layers {
                ink.push(VectorItem::Path(VectorPath {
                    id: None,
                    path: path.clone(),
                    fill: Some((
                        color.map_or_else(|| fill.clone(), Paint::from),
                        FillRule::NonZero,
                    )),
                    stroke: None,
                    source: None,
                }));
            }
            let mut glyph_children = vec![VectorItem::Group(VectorGroup {
                transform: glyph.transform,
                children: ink,
                ..Default::default()
            })];
            if let Some((image, local)) = &glyph.artwork.bitmap {
                let pattern =
                    motion_core::ImagePattern::from_asset(image.clone()).transform(*local);
                let rect = motion_core::display_list::mapped_bounds(
                    Rect::new(
                        0.0,
                        0.0,
                        f64::from(image.width()),
                        f64::from(image.height()),
                    ),
                    *local,
                    "text.bitmap",
                )?;
                let mut image = VectorImage::new(image.clone(), rect);
                image.pixel_transform = *local;
                image.sampling = pattern.sampling;
                image.extend = pattern.extend;
                image.decal = pattern.decal;
                glyph_children.push(VectorItem::Group(VectorGroup {
                    transform: glyph.transform,
                    children: vec![VectorItem::Image(image)],
                    ..Default::default()
                }));
            }
            if let Some((width, paint)) = &self.stroke {
                for path in std::iter::once(&glyph.artwork.path)
                    .chain(glyph.artwork.layers.iter().map(|(path, _)| path))
                    .filter(|path| !path.verbs().is_empty())
                {
                    let outline = motion_core::StrokeStyle {
                        width: *width,
                        ..Default::default()
                    }
                    .outline(&path.transformed(&glyph.transform)?, "text.vector.stroke")?;
                    glyph_children
                        .push(VectorItem::Path(VectorPath::filled(outline, paint.clone())));
                }
            }
            children.push(VectorItem::Group(VectorGroup {
                id: Some(format!("glyph-{index}").into()),
                source: Some(glyph.source.clone()),
                children: glyph_children,
                ..Default::default()
            }));
        }
        Ok(Arc::new(
            VectorTree::new(
                self.ink_bounds(layout),
                VectorGroup {
                    children,
                    ..Default::default()
                },
            )?
            .with_baseline(layout.baseline),
        ))
    }
}

pub(crate) fn invalid(field: &str, message: &str) -> ValidationError {
    ValidationError::invalid("text", field, message)
}

pub(crate) fn limited(message: &str) -> ValidationError {
    ValidationError::limit("text", "work", message)
}
