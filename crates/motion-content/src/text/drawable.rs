//! Glyph drawables and `Drawable` rendering for shaped text.

use super::*;

impl Text {
    pub(super) fn glyph_node(&self, glyph: &TextGlyph) -> Node {
        let paint = self.spans[glyph.span]
            .fill
            .as_ref()
            .unwrap_or(&self.fill)
            .clone();
        let mut node = Node::from(GlyphDrawable {
            artwork: glyph.artwork.clone(),
            transform: glyph.transform,
            bounds: glyph.bounds,
            fill: paint,
            stroke: self.stroke.clone(),
        });
        if glyph.artwork.path.verbs().is_empty()
            && glyph.artwork.layers.is_empty()
            && glyph.artwork.bitmap.is_none()
        {
            node = Node::empty();
        }
        node
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
    /// Local ink bounds of the resolved text, including animation effects.
    pub(crate) fn local_bounds(&self, context: &DrawContext) -> Result<Rect, ValidationError> {
        if self.is_deferred() {
            return DeferredContent::resolve(self, context)?
                .measure_with(context, motion_core::ValidationLimits::default());
        }
        TextRun(self.clone()).bounds(context)
    }

    pub(crate) fn is_deferred(&self) -> bool {
        self.write_progress.is_some() || self.part_animation.is_some()
    }
}

impl DeferredContent for Text {
    fn resolve(&self, context: &DrawContext) -> Result<Node, ValidationError> {
        if !self.is_deferred() {
            return Ok(Node::from(TextRun(self.clone())));
        }
        let mut base = self.clone();
        base.write_progress = None;
        base.part_animation = None;
        if let Some(animation) = &self.part_animation {
            if self.write_progress.is_some() {
                return Err(invalid(
                    "effects",
                    "Use separate Text nodes for simultaneous handwriting and part transforms",
                ));
            }
            return base.animate_parts_resolved(context, animation.delay, animation.style.as_ref());
        }
        let Some(progress) = self.write_progress else {
            return Ok(Node::from(TextRun(base)));
        };
        if !progress.is_finite() {
            return Err(invalid("progress", "Text write progress must be finite"));
        }
        base.write_with_context(progress, context)
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
        let margin = self.0.stroke.as_ref().map_or(0.0, |(width, _)| width * 2.0);
        Ok(Rect::new(
            layout.bounds.left - margin,
            layout.bounds.top - margin,
            layout.bounds.right + margin,
            layout.bounds.bottom + margin,
        ))
    }

    fn baseline(&self, context: &DrawContext) -> Option<f64> {
        self.0.resolve(context).ok().map(|layout| layout.baseline)
    }

    fn draw(&self, context: &DrawContext, canvas: &mut Canvas<'_>) -> Result<(), ValidationError> {
        let layout = self.0.resolve(context)?;
        let parts = layout.parts(self.0.split);
        for glyph in layout.glyphs.iter() {
            let owner = parts.iter().position(|part| part.owns(glyph)).unwrap_or(0);
            if self.0.visible.is_some_and(|visible| owner >= visible) {
                continue;
            }
            GlyphDrawable {
                artwork: glyph.artwork.clone(),
                transform: glyph.transform,
                bounds: glyph.bounds,
                fill: self.0.spans[glyph.span]
                    .fill
                    .as_ref()
                    .unwrap_or(&self.0.fill)
                    .clone(),
                stroke: self.0.stroke.clone(),
            }
            .draw(context, canvas)?;
        }
        Ok(())
    }
}
