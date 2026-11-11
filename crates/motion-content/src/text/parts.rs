//! Text part selection and animated per-part styling.

use super::*;

impl Text {
    /// Evaluates part styles synchronously at explicit scene-local signed time,
    /// without reshaping for animation. Source bounds and line wrapping stay fixed;
    /// returned node bounds also include transformed animation ink.
    /// Measured paragraph baseline is retained for horizontal Row alignment.
    pub fn try_animate_parts_at(
        &self,
        context: &motion_core::SceneContext<'_>,
        delay: motion_core::Duration,
        style: impl Fn(&TextPart) -> PartStyle,
    ) -> Result<Node, ValidationError> {
        if delay < motion_core::Duration::ZERO {
            return Err(invalid("delay", "Text part delay must be nonnegative"));
        }
        let fonts = self
            .fonts
            .as_ref()
            .or_else(|| {
                context
                    .assets()
                    .fonts()
                    .and_then(|environment| environment.downcast_ref::<FontLibrary>())
            })
            .ok_or_else(|| {
                invalid(
                    "fonts",
                    "Text animation requires an explicit font environment",
                )
            })?;
        let drawing = DrawContext::default()
            .with_fonts(fonts)
            .with_time(context.time());
        self.animate_parts_resolved(&drawing, delay, &style)
    }

    pub(super) fn animate_parts_resolved(
        &self,
        context: &DrawContext,
        delay: motion_core::Duration,
        style: &dyn Fn(&TextPart) -> PartStyle,
    ) -> Result<Node, ValidationError> {
        if delay < motion_core::Duration::ZERO {
            return Err(invalid("delay", "Text part delay must be nonnegative"));
        }
        let layout = self.resolve(context)?;
        let parts = layout.parts(self.split);
        let mut nodes = Vec::new();
        for (index, mut part) in parts.iter().cloned().enumerate() {
            if self.visible.is_some_and(|visible| index >= visible) {
                continue;
            }
            let ticks =
                i128::from(context.time().ticks()) - i128::from(delay.ticks()) * index as i128;
            part.time = Time::from_ticks(
                i64::try_from(ticks)
                    .map_err(|_| invalid("time", "Staggered part time exceeds signed ticks"))?,
            );
            let styled = style(&part);
            styled.validate()?;
            let children = layout
                .glyphs
                .iter()
                .filter(|glyph| part.owns(glyph))
                .map(|glyph| self.glyph_node(glyph))
                .collect::<Vec<_>>();
            nodes.push(
                Node::group(children)
                    .pivot(motion_core::Pivot::Point(part.bounds.center()))
                    .scale(styled.scale)
                    .rotate(styled.rotation)
                    .translate(styled.offset.x, styled.offset.y)
                    .opacity(styled.opacity),
            );
        }
        let node = Node::group(
            std::iter::once(Node::from(ParagraphAnchor {
                bounds: layout.bounds,
                baseline: layout.baseline,
            }))
            .chain(nodes),
        );
        let bounds = node.measure(motion_core::ValidationLimits::default())?;
        Ok(node.with_text_metrics(layout.baseline, bounds))
    }
}

struct ParagraphAnchor {
    bounds: Rect,
    baseline: f64,
}

impl Drawable for ParagraphAnchor {
    fn bounds(&self, _: &DrawContext) -> Result<Rect, ValidationError> {
        Ok(self.bounds)
    }

    fn baseline(&self, _: &DrawContext) -> Option<f64> {
        Some(self.baseline)
    }

    fn draw(&self, _: &DrawContext, _: &mut Canvas<'_>) -> Result<(), ValidationError> {
        Ok(())
    }
}

/// One logical part with original source range, layout bounds and delayed time.
#[derive(Clone, Debug)]
pub struct TextPart {
    /// Reading-order part index.
    pub index: usize,
    /// Total part count before visibility filtering.
    pub count: usize,
    /// Original UTF-8 source bytes.
    pub source: Range<usize>,
    /// Stable paragraph-local bounds.
    pub bounds: Rect,
    time: Time,
    line: Option<usize>,
}

impl TextPart {
    /// Scene-local time minus index times the supplied delay.
    pub fn time(&self) -> Time {
        self.time
    }

    /// Samples any typed animation at the delayed signed time.
    pub fn sample<Value>(&self, animation: &impl motion_core::Signal<Value>) -> Value {
        animation.sample(self.time.since_origin())
    }

    pub(super) fn owns(&self, glyph: &TextGlyph) -> bool {
        self.line.map_or_else(
            || self.source.contains(&glyph.source.start),
            |line| glyph.line == line,
        )
    }
}

impl ParagraphLayout {
    /// Derives parts from an already shaped paragraph, without font/parser work.
    pub fn parts(&self, split: TextSplit) -> Vec<TextPart> {
        let ranges = match split {
            TextSplit::Characters => self
                .text
                .grapheme_indices(true)
                .map(|(start, value)| start..start + value.len())
                .collect::<Vec<_>>(),
            TextSplit::Words => {
                let mut starts = self
                    .text
                    .unicode_word_indices()
                    .map(|(start, _)| start)
                    .collect::<Vec<_>>();
                if starts.is_empty() && !self.text.is_empty() {
                    starts.push(0);
                }
                if let Some(first) = starts.first_mut() {
                    *first = 0;
                }
                starts.push(self.text.len());
                starts.windows(2).map(|pair| pair[0]..pair[1]).collect()
            }
            TextSplit::Lines => self.line_sources.to_vec(),
        };
        let count = ranges.len();
        ranges
            .into_iter()
            .enumerate()
            .map(|(index, source)| {
                let mut bounds = None;
                for glyph in self.glyphs.iter().filter(|glyph| {
                    if split == TextSplit::Lines {
                        glyph.line == index
                    } else {
                        source.contains(&glyph.source.start)
                    }
                }) {
                    bounds = Some(
                        bounds.map_or(glyph.bounds, |bounds: Rect| bounds.union(glyph.bounds)),
                    );
                }
                TextPart {
                    index,
                    count,
                    source,
                    bounds: bounds.unwrap_or_else(|| {
                        if split == TextSplit::Lines {
                            self.lines[index]
                        } else {
                            Rect::default()
                        }
                    }),
                    time: Time::ZERO,
                    line: (split == TextSplit::Lines).then_some(index),
                }
            })
            .collect()
    }
}

/// Per-part transform evaluated without changing paragraph layout.
#[derive(Clone, Copy, Debug)]
pub struct PartStyle {
    offset: motion_core::Vector2,
    scale: f64,
    rotation: f64,
    opacity: f64,
}

impl Default for PartStyle {
    fn default() -> Self {
        Self::new()
    }
}

impl PartStyle {
    /// Identity style.
    pub fn new() -> Self {
        Self {
            offset: motion_core::Vector2::ZERO,
            scale: 1.0,
            rotation: 0.0,
            opacity: 1.0,
        }
    }

    /// Adds a logical displacement.
    pub fn offset(mut self, horizontal: f64, vertical: f64) -> Self {
        self.offset = motion_core::Vector2::new(horizontal, vertical);
        self
    }

    /// Sets uniform scale around the part's measured center.
    pub fn scale(mut self, scale: f64) -> Self {
        self.scale = scale;
        self
    }

    /// Sets clockwise radians.
    pub fn rotate(mut self, rotation: f64) -> Self {
        self.rotation = rotation;
        self
    }

    /// Sets opacity in 0..=1.
    pub fn opacity(mut self, opacity: f64) -> Self {
        self.opacity = opacity;
        self
    }

    pub(super) fn validate(self) -> Result<(), ValidationError> {
        if ![
            self.offset.x,
            self.offset.y,
            self.scale,
            self.rotation,
            self.opacity,
        ]
        .iter()
        .all(|value| value.is_finite())
            || !(0.0..=1.0).contains(&self.opacity)
        {
            Err(invalid(
                "part",
                "Part transforms must be finite and opacity in 0..=1",
            ))
        } else {
            Ok(())
        }
    }
}
