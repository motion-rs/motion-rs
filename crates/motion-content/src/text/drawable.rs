//! Glyph drawables and `Drawable` rendering for shaped text.

use super::*;

impl Text {
    pub(super) fn glyph_fill(&self, glyph: &TextGlyph) -> Paint {
        self.paragraph
            .fills
            .get(glyph.span)
            .and_then(Option::as_ref)
            .unwrap_or(&self.fill)
            .clone()
    }

    /// Glyphs whose owning part is within the visible count, with source indices.
    pub(super) fn visible_glyphs<'a>(
        &self,
        layout: &'a ParagraphLayout,
    ) -> Vec<(usize, &'a TextGlyph)> {
        let parts = layout.parts(self.split);
        layout
            .glyphs
            .iter()
            .enumerate()
            .filter(|(_, glyph)| {
                let owner = parts.iter().position(|part| part.owns(glyph)).unwrap_or(0);
                self.visible.is_none_or(|visible| owner < visible)
            })
            .collect()
    }

    pub(super) fn ink_bounds(&self, layout: &ParagraphLayout) -> Rect {
        let margin = self.stroke.as_ref().map_or(0.0, |(width, _)| width * 2.0);
        Rect::new(
            layout.bounds.left - margin,
            layout.bounds.top - margin,
            layout.bounds.right + margin,
            layout.bounds.bottom + margin,
        )
    }

    fn glyph_drawable(&self, glyph: &TextGlyph) -> GlyphDrawable {
        GlyphDrawable {
            artwork: glyph.artwork.clone(),
            transform: glyph.transform,
            bounds: glyph.bounds,
            fill: self.glyph_fill(glyph),
            stroke: self.stroke.clone(),
        }
    }

    pub(super) fn glyph_node(&self, glyph: &TextGlyph) -> Node {
        if glyph.artwork.path.verbs().is_empty()
            && glyph.artwork.layers.is_empty()
            && glyph.artwork.bitmap.is_none()
        {
            return Node::empty();
        }
        Node::from(self.glyph_drawable(glyph))
    }
}

#[derive(Clone)]
struct GlyphDrawable {
    artwork: Arc<glyph::Artwork>,
    transform: Transform,
    bounds: Rect,
    fill: Paint,
    stroke: Option<(f64, Paint)>,
}

impl Drawable for GlyphDrawable {
    fn bounds(&self, _: &DrawContext) -> Result<Rect, ValidationError> {
        let margin = self.stroke.as_ref().map_or(0.0, |(width, _)| width * 2.0);
        Ok(Rect::new(
            self.bounds.left - margin,
            self.bounds.top - margin,
            self.bounds.right + margin,
            self.bounds.bottom + margin,
        ))
    }

    fn draw(&self, context: &DrawContext, canvas: &mut Canvas<'_>) -> Result<(), ValidationError> {
        context.font_usage().record(self.artwork.font.clone());
        if !self.artwork.path.verbs().is_empty() {
            canvas.fill_path(
                self.artwork.path.clone(),
                self.transform,
                self.fill.clone(),
                FillRule::NonZero,
            )?;
        }
        for (path, color) in &self.artwork.layers {
            canvas.fill_path(
                path.clone(),
                self.transform,
                color.map_or_else(|| self.fill.clone(), Paint::from),
                FillRule::NonZero,
            )?;
        }
        if let Some((image, local)) = &self.artwork.bitmap {
            let pattern = motion_core::ImagePattern::from_asset(image.clone()).transform(*local);
            let bounds = motion_core::display_list::mapped_bounds(
                Rect::new(
                    0.0,
                    0.0,
                    f64::from(image.width()),
                    f64::from(image.height()),
                ),
                *local,
                "text.bitmap",
            )?;
            canvas.fill_path(
                Arc::new(motion_core::Path2D::from_rect(bounds)?),
                self.transform,
                pattern,
                FillRule::NonZero,
            )?;
        }
        if let Some((width, paint)) = &self.stroke {
            for path in std::iter::once(&self.artwork.path)
                .chain(self.artwork.layers.iter().map(|(path, _)| path))
                .filter(|path| !path.verbs().is_empty())
            {
                canvas.stroke(
                    &path.transformed(&self.transform)?,
                    &motion_core::StrokeStyle {
                        width: *width,
                        ..motion_core::StrokeStyle::default()
                    },
                    paint.clone(),
                )?;
            }
        }
        Ok(())
    }
}

impl Text {
    pub(crate) fn is_deferred(&self) -> bool {
        !matches!(self.reveal, Reveal::None)
    }
}

impl DeferredContent for Text {
    fn resolve(&self, context: &DrawContext) -> Result<Node, ValidationError> {
        match &self.reveal {
            Reveal::None => Ok(Node::from(TextRun(self.clone()))),
            Reveal::Write(progress) => {
                if !progress.is_finite() {
                    return Err(invalid("progress", "Text write progress must be finite"));
                }
                self.write_with_context(*progress, context)
            }
            Reveal::Parts(animation) => {
                self.animate_parts_resolved(context, animation.delay, animation.style.as_ref())
            }
        }
    }

    fn baseline(&self, context: &DrawContext) -> Option<f64> {
        self.resolve(context).ok().map(|layout| layout.baseline)
    }
}

impl From<Text> for Node {
    fn from(text: Text) -> Node {
        if text.is_deferred() {
            Node::deferred(text)
        } else {
            Node::from(TextRun(text))
        }
    }
}

/// Static text laid out and drawn as a leaf.
struct TextRun(Text);

impl Drawable for TextRun {
    fn bounds(&self, context: &DrawContext) -> Result<Rect, ValidationError> {
        let layout = self.0.resolve(context)?;
        Ok(self.0.ink_bounds(&layout))
    }

    fn baseline(&self, context: &DrawContext) -> Option<f64> {
        self.0.resolve(context).ok().map(|layout| layout.baseline)
    }

    fn draw(&self, context: &DrawContext, canvas: &mut Canvas<'_>) -> Result<(), ValidationError> {
        let layout = self.0.resolve(context)?;
        for (_, glyph) in self.0.visible_glyphs(&layout) {
            self.0.glyph_drawable(glyph).draw(context, canvas)?;
        }
        Ok(())
    }
}
