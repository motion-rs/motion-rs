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
    fn bounds(&self, _: &DrawContext) -> Rect {
        let margin = self.stroke.as_ref().map_or(0.0, |(width, _)| width * 2.0);
        Rect::new(
            self.bounds.left - margin,
            self.bounds.top - margin,
            self.bounds.right + margin,
            self.bounds.bottom + margin,
        )
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

impl Drawable for Text {
    fn resolve_node(&self, context: &DrawContext) -> Result<Option<Node>, ValidationError> {
        if self.write_progress.is_none() && self.part_animation.is_none() {
            return Ok(None);
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
            return base
                .animate_parts_resolved(context, animation.delay, animation.style.as_ref())
                .map(Some);
        }
        let Some(progress) = self.write_progress else {
            return Ok(None);
        };
        if !progress.is_finite() {
            return Err(invalid("progress", "Text write progress must be finite"));
        }
        base.write_with_context(progress, context).map(Some)
    }

    fn try_bounds(&self, context: &DrawContext) -> Result<Rect, ValidationError> {
        if let Some(node) = self.resolve_node(context)? {
            return node.measure_with(context, motion_core::ValidationLimits::default());
        }
        let layout = self.resolve(context)?;
        let margin = self.stroke.as_ref().map_or(0.0, |(width, _)| width * 2.0);
        Ok(Rect::new(
            layout.bounds.left - margin,
            layout.bounds.top - margin,
            layout.bounds.right + margin,
            layout.bounds.bottom + margin,
        ))
    }

    fn bounds(&self, context: &DrawContext) -> Rect {
        self.try_bounds(context)
            .unwrap_or(Rect::new(f64::NAN, 0.0, 0.0, 0.0))
    }

    fn baseline(&self, context: &DrawContext) -> Option<f64> {
        self.resolve(context).ok().map(|layout| layout.baseline)
    }

    fn draw(&self, context: &DrawContext, canvas: &mut Canvas<'_>) -> Result<(), ValidationError> {
        let layout = self.resolve(context)?;
        let parts = layout.parts(self.split);
        for glyph in layout.glyphs.iter() {
            let owner = parts.iter().position(|part| part.owns(glyph)).unwrap_or(0);
            if self.visible.is_some_and(|visible| owner >= visible) {
                continue;
            }
            GlyphDrawable {
                artwork: glyph.artwork.clone(),
                transform: glyph.transform,
                bounds: glyph.bounds,
                fill: self.spans[glyph.span]
                    .fill
                    .as_ref()
                    .unwrap_or(&self.fill)
                    .clone(),
                stroke: self.stroke.clone(),
            }
            .draw(context, canvas)?;
        }
        Ok(())
    }
}
