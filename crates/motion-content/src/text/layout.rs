//! Line fitting with ellipsis truncation and paragraph layout of rich text.

use super::*;

impl Text {
    /// Precomputes a paragraph limited to 1..=512 lines, adding an ellipsis only
    /// when truncated. Returns the text and whether source content was removed.
    /// Rich styles and extended grapheme boundaries are retained. The supplied
    /// font snapshot is frozen on the returned text; no system discovery occurs.
    /// Preparation may reshape bounded suffix candidates and is not a frame-loop API.
    pub fn fit_lines(
        &self,
        max_lines: usize,
        fonts: &FontLibrary,
    ) -> Result<(Self, bool), ValidationError> {
        use unicode_segmentation::UnicodeSegmentation;
        if !(1..=512).contains(&max_lines) {
            return Err(invalid("max_lines", "Text line limit requires 1..=512"));
        }
        let full = self.layout(fonts)?;
        let fits = |layout: &ParagraphLayout| {
            layout.lines.len() <= max_lines
                && self.width.is_none_or(|width| {
                    layout.bounds.width() <= width
                        && layout.lines.iter().all(|line| line.width() <= width)
                })
        };
        if fits(&full) {
            return Ok((self.clone().fonts(fonts), false));
        }
        let end = full
            .line_sources
            .iter()
            .take(max_lines)
            .map(|range| range.end)
            .max()
            .unwrap_or(0);
        let mut cuts = full
            .text
            .grapheme_indices(true)
            .map(|(index, _)| index)
            .filter(|index| *index <= end)
            .collect::<Vec<_>>();
        if end == full.text.len() && !cuts.contains(&end) {
            cuts.push(end);
        }
        cuts.sort_unstable();
        for end in cuts.into_iter().rev() {
            let mut spans = Vec::new();
            let mut consumed = 0;
            for source in self.spans.iter() {
                let count = end.saturating_sub(consumed).min(source.text.len());
                if count > 0 {
                    let mut span = source.clone();
                    span.text = Arc::from(&source.text[..count]);
                    spans.push(span);
                }
                consumed += source.text.len();
                if consumed >= end {
                    break;
                }
            }
            if let Some(last) = spans.last_mut() {
                last.text = Arc::from(format!("{}\u{2026}", last.text));
            } else {
                let mut span = self
                    .spans
                    .first()
                    .cloned()
                    .unwrap_or_else(|| TextSpan::new(""));
                span.text = Arc::from("\u{2026}");
                spans.push(span);
            }
            let mut candidate = self.clone();
            candidate.spans = spans.into();
            candidate.fonts = Some(fonts.clone());
            candidate.invalidate_layout_cache();
            let layout = candidate.layout(fonts)?;
            if fits(&layout) {
                return Ok((candidate, true));
            }
        }
        Err(invalid(
            "max_width",
            "No grapheme-safe ellipsis fits the requested paragraph",
        ))
    }

    /// Shapes once per text-style/font snapshot, returning actual line/ink metrics.
    pub fn layout(&self, fonts: &FontLibrary) -> Result<Arc<ParagraphLayout>, ValidationError> {
        self.validate_layout()?;
        if let Some(result) = self
            .layout_cache
            .lock()
            .map_err(|_| invalid("cache", "Layout cache is poisoned"))?
            .get(&fonts.snapshot_id())
        {
            return result.clone();
        }
        let key = self.layout_key();
        let result = if let Some(layout) = fonts
            .layout_cache
            .lock()
            .map_err(|_| invalid("cache", "Shared paragraph cache is poisoned"))?
            .get(&key)
            .cloned()
        {
            Ok(layout)
        } else {
            let result = self.shape(fonts).map(Arc::new);
            if let Ok(layout) = &result {
                let mut shared = fonts
                    .layout_cache
                    .lock()
                    .map_err(|_| invalid("cache", "Shared paragraph cache is poisoned"))?;
                if shared.len() >= 64 {
                    shared.clear();
                }
                shared.insert(key, layout.clone());
            }
            result
        };
        let mut cache = self
            .layout_cache
            .lock()
            .map_err(|_| invalid("cache", "Layout cache is poisoned"))?;
        if cache.len() >= 8 {
            cache.clear();
        }
        cache.insert(fonts.snapshot_id(), result.clone());
        result
    }

    fn validate_layout(&self) -> Result<(), ValidationError> {
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
        for span in self.spans.iter() {
            if let Some(paint) = &span.fill {
                paint.validate()?;
            }
        }
        Ok(())
    }

    fn layout_key(&self) -> LayoutKey {
        LayoutKey {
            spans: self
                .spans
                .iter()
                .map(|span| {
                    (
                        span.text.clone(),
                        span.family.clone(),
                        span.size.map(f64::to_bits),
                        span.weight.map(|weight| weight.0),
                        span.italic,
                    )
                })
                .collect(),
            family: self.family.clone(),
            size: self.size.to_bits(),
            weight: self.weight.0,
            line_height: self.line_height.to_bits(),
            spacing: self.spacing.to_bits(),
            width: self.width.map(f64::to_bits),
            alignment: self.alignment,
        }
    }

    pub(super) fn resolve(
        &self,
        context: &DrawContext,
    ) -> Result<Arc<ParagraphLayout>, ValidationError> {
        let fonts = self
            .fonts
            .as_ref()
            .or(context
                .fonts()
                .and_then(|environment| environment.downcast_ref::<FontLibrary>()))
            .ok_or_else(|| invalid("fonts", "paragraphs require an explicit FontLibrary"))?;
        if context.layout_cache().is_none() {
            return self.layout(fonts);
        }
        let fonts = context
            .render_cache()
            .map_or_else(|| fonts.clone(), |cache| fonts.with_render_cache(cache));
        self.validate_layout()?;
        let key = self.layout_key();
        let key_bytes = std::mem::size_of::<LayoutKey>()
            + std::mem::size_of_val(key.spans.as_slice())
            + key.family.len()
            + key
                .spans
                .iter()
                .map(|span| span.0.len() + span.1.as_ref().map_or(0, |family| family.len()))
                .sum::<usize>();
        context
            .cache_layout(
                (fonts.snapshot_id(), key),
                || self.shape(&fonts).map(Arc::new),
                |result| {
                    key_bytes.saturating_add(result.as_ref().map_or_else(
                        |error| error.message.len(),
                        |layout| {
                            std::mem::size_of::<ParagraphLayout>()
                                + std::mem::size_of_val(layout.glyphs.as_ref())
                                + std::mem::size_of_val(layout.lines.as_ref())
                                + std::mem::size_of_val(layout.line_sources.as_ref())
                                + layout.text.len()
                        },
                    ))
                },
            )
            .as_ref()
            .clone()
    }

    fn paragraph_text(&self) -> Result<String, ValidationError> {
        if self.spans.len() > 256 {
            return Err(limited("Text exceeds 256 rich spans"));
        }
        if !(1..=1000).contains(&self.weight.0)
            || self.spans.iter().any(|span| {
                span.weight
                    .is_some_and(|weight| !(1..=1000).contains(&weight.0))
            })
        {
            return Err(invalid("weight", "Font weight must be in 1..=1000"));
        }
        let length = self
            .spans
            .iter()
            .try_fold(0usize, |length, span| length.checked_add(span.text.len()))
            .ok_or_else(|| limited("Paragraph size overflow"))?;
        if length > 16_384 {
            return Err(limited("Paragraph exceeds 16384 UTF-8 bytes"));
        }
        let text = self
            .spans
            .iter()
            .map(|span| span.text.as_ref())
            .collect::<String>();
        if text.len() > 16_384
            || text
                .chars()
                .filter(|value| matches!(value, '\n' | '\r'))
                .count()
                > 4096
        {
            return Err(limited("Paragraph exceeds its text/line allowance"));
        }
        if !self.size.is_finite()
            || self.size <= 0.0
            || self.size > 4096.0
            || self.size as f32 == 0.0
            || !self.line_height.is_finite()
            || !(0.1..=16.0).contains(&self.line_height)
            || !self.spacing.is_finite()
            || self.spacing.abs() > 4096.0
            || !(self.spacing / self.size).is_finite()
            || !((self.spacing / self.size) as f32).is_finite()
            || self.width.is_some_and(|width| {
                !width.is_finite() || width <= 0.0 || width > 1e7 || width as f32 == 0.0
            })
        {
            return Err(invalid(
                "style",
                "Invalid paragraph size, line-height, tracking or width",
            ));
        }
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
        Ok(text)
    }

    fn shape(&self, fonts: &FontLibrary) -> Result<ParagraphLayout, ValidationError> {
        let text = self.paragraph_text()?;
        let mut features = cosmic_text::FontFeatures::new();
        features
            .disable(cosmic_text::FeatureTag::STANDARD_LIGATURES)
            .disable(cosmic_text::FeatureTag::CONTEXTUAL_LIGATURES);

        fn family(name: &str) -> cosmic_text::Family<'_> {
            if name.is_empty() {
                cosmic_text::Family::SansSerif
            } else {
                cosmic_text::Family::Name(name)
            }
        }
        let defaults = cosmic_text::Attrs::new()
            .family(family(&self.family))
            .weight(self.weight)
            .letter_spacing((self.spacing / self.size) as f32)
            .font_features(features.clone());
        let mut styled = Vec::new();
        for (index, span) in self.spans.iter().enumerate() {
            let size = span.size.unwrap_or(self.size);
            if !size.is_finite()
                || size <= 0.0
                || size > 4096.0
                || size as f32 == 0.0
                || !((self.spacing / size) as f32).is_finite()
                || (size * self.line_height) as f32 == 0.0
            {
                return Err(invalid("span", "Invalid rich-span font size or tracking"));
            }
            if let Some(paint) = &span.fill {
                paint.validate()?;
            }
            styled.push((
                span.text.as_ref(),
                cosmic_text::Attrs::new()
                    .family(family(span.family.as_deref().unwrap_or(&self.family)))
                    .weight(span.weight.unwrap_or(self.weight))
                    .style(if span.italic {
                        cosmic_text::Style::Italic
                    } else {
                        cosmic_text::Style::Normal
                    })
                    .metadata(index)
                    .metrics(cosmic_text::Metrics::new(
                        size as f32,
                        (size * self.line_height) as f32,
                    ))
                    .letter_spacing((self.spacing / size) as f32)
                    .font_features(features.clone()),
            ));
        }
        if text.is_empty() {
            return Ok(ParagraphLayout {
                glyphs: Arc::from([]),
                bounds: Rect::default(),
                baseline: 0.0,
                lines: Arc::from([]),
                line_sources: Arc::from([]),
                text: text.into(),
            });
        }
        if fonts.is_empty() {
            return Err(invalid("fonts", "The font library is empty"));
        }
        let mut system = fonts.lock_font_system()?;
        let mut buffer = cosmic_text::Buffer::new(
            &mut system,
            cosmic_text::Metrics::new(self.size as f32, (self.size * self.line_height) as f32),
        );
        buffer.set_size(self.width.map(|width| width as f32), None);
        buffer.set_wrap(cosmic_text::Wrap::WordOrGlyph);
        let alignment = match self.alignment {
            TextAlignment::Start => None,
            TextAlignment::End => Some(cosmic_text::Align::End),
            TextAlignment::Center => Some(cosmic_text::Align::Center),
            TextAlignment::Justified => Some(cosmic_text::Align::Justified),
        };
        buffer.set_rich_text(styled, &defaults, cosmic_text::Shaping::Advanced, alignment);
        buffer.shape_until_scroll(&mut system, false);
        let mut offset = 0;
        let mut offsets = Vec::with_capacity(buffer.lines.len());
        for line in &buffer.lines {
            if !text[offset..].starts_with(line.text()) {
                return Err(invalid(
                    "source",
                    "Shaped line no longer matches original UTF-8 text",
                ));
            }
            offsets.push(offset);
            offset += line.text().len();
            let remaining = &text[offset..];
            if remaining.starts_with("\r\n") {
                offset += 2;
            } else if let Some(separator) = remaining
                .chars()
                .next()
                .filter(|value| matches!(value, '\r' | '\n' | '\u{0085}' | '\u{2028}' | '\u{2029}'))
            {
                offset += separator.len_utf8();
            }
        }
        self.collect_layout(fonts, &mut system, &buffer, text, offsets)
    }

    fn collect_layout(
        &self,
        fonts: &FontLibrary,
        system: &mut cosmic_text::FontSystem,
        buffer: &cosmic_text::Buffer,
        text: String,
        offsets: Vec<usize>,
    ) -> Result<ParagraphLayout, ValidationError> {
        let mut glyphs = Vec::new();
        let mut lines = Vec::new();
        let mut line_sources = Vec::new();
        let mut bounds = Rect::default();
        let mut baseline = 0.0;
        for run in buffer.layout_runs() {
            if lines.len() >= 4096 {
                return Err(limited("Paragraph exceeds 4096 visual lines"));
            }
            if lines.is_empty() {
                baseline = f64::from(run.line_y);
            }
            let line = Rect::new(
                0.0,
                f64::from(run.line_top),
                self.width.unwrap_or(f64::from(run.line_w)),
                f64::from(run.line_top + run.line_height),
            );
            bounds = bounds.union(line);
            let line_index = lines.len();
            lines.push(line);
            let start = run
                .glyphs
                .iter()
                .map(|glyph| glyph.start)
                .min()
                .unwrap_or(0);
            let end = run
                .glyphs
                .iter()
                .map(|glyph| glyph.end)
                .max()
                .unwrap_or(start);
            line_sources.push(offsets[run.line_i] + start..offsets[run.line_i] + end);
            for glyph in run.glyphs {
                if glyphs.len() >= 4096 {
                    return Err(limited("Paragraph exceeds 4096 shaped glyphs"));
                }
                if glyph.glyph_id == 0 {
                    return Err(ValidationError::unsupported(
                        "text",
                        "missing_glyph",
                        "Registered fonts do not cover a shaped text cluster",
                    ));
                }
                let artwork = glyph::Artwork::load(
                    fonts,
                    system,
                    glyph.font_id,
                    glyph.font_weight,
                    glyph.glyph_id,
                )?;
                let scale = f64::from(glyph.font_size) / artwork.units;
                let horizontal =
                    f64::from(glyph.x) + f64::from(glyph.font_size) * f64::from(glyph.x_offset);
                let vertical = f64::from(run.line_y + glyph.y)
                    - f64::from(glyph.font_size) * f64::from(glyph.y_offset);
                let transform = Transform::scale_non_uniform(scale, -scale)
                    .then(Transform::translate(horizontal, vertical));
                let hit = Rect::new(
                    f64::from(glyph.x),
                    f64::from(run.line_top),
                    f64::from(glyph.x + glyph.w),
                    f64::from(run.line_top + run.line_height),
                );
                let ink = artwork
                    .bounds(transform)?
                    .map_or(hit, |bounds| hit.union(bounds));
                bounds = bounds.union(ink);
                let family = system
                    .db()
                    .face(glyph.font_id)
                    .and_then(|face| face.families.first())
                    .map(|family| Arc::from(family.0.as_str()))
                    .unwrap_or_else(|| Arc::from("<font>"));
                glyphs.push(TextGlyph {
                    source: offsets[run.line_i] + glyph.start..offsets[run.line_i] + glyph.end,
                    line: line_index,
                    family,
                    bounds: ink,
                    artwork,
                    transform,
                    span: glyph.metadata,
                });
            }
        }
        Ok(ParagraphLayout {
            glyphs: glyphs.into(),
            bounds,
            baseline,
            lines: lines.into(),
            line_sources: line_sources.into(),
            text: text.into(),
        })
    }
}
