//! Caption styling and frozen per-cue paragraph layout for subtitle tracks.

use crate::{
    CaptionCue, CaptionTrack, FontLibrary, ParagraphLayout, Text, TextAlignment, TextSpan,
};
use motion_core::{Color, DrawContext, Node, NodeModifiers, Rect, Size, Time, ValidationError};
use std::sync::Arc;
use unicode_segmentation::UnicodeSegmentation;

fn invalid(message: &str) -> ValidationError {
    ValidationError::invalid("captions", "input", message)
}

/// production subtitle style with an explicit safe area and frozen font environment.
#[derive(Clone, Debug)]
pub struct CaptionStyle {
    safe: Rect,
    size: f64,
    color: Color,
    background: Color,
    padding: (f64, f64),
    gap: f64,
    lines: usize,
    keywords: Vec<(String, Color)>,
}

impl CaptionStyle {
    /// Defaults to 30-unit white text, translucent black, padding 12/8 and two lines.
    pub fn new(safe: Rect) -> Self {
        Self {
            safe,
            size: 30.0,
            color: Color::WHITE,
            background: Color::rgba(0, 0, 0, 210),
            padding: (12.0, 8.0),
            gap: 8.0,
            lines: 2,
            keywords: Vec::new(),
        }
    }

    /// Replaces finite font size in (0,4096].
    pub fn font_size(mut self, size: f64) -> Self {
        self.size = size;
        self
    }

    /// Replaces the base text color.
    pub fn color(mut self, color: Color) -> Self {
        self.color = color;
        self
    }

    /// Replaces the shared backplate color.
    pub fn background(mut self, color: Color) -> Self {
        self.background = color;
        self
    }

    /// Replaces nonnegative horizontal/vertical padding, each at most 4096.
    pub fn padding(mut self, horizontal: f64, vertical: f64) -> Self {
        self.padding = (horizontal, vertical);
        self
    }

    /// Replaces the nonnegative gap between simultaneous cues, at most 4096.
    pub fn gap(mut self, gap: f64) -> Self {
        self.gap = gap;
        self
    }

    /// Replaces the per-cue line limit in 1..=8; excess text is rejected.
    pub fn max_lines(mut self, lines: usize) -> Self {
        self.lines = lines;
        self
    }

    /// Adds a grapheme-safe literal keyword; longest match and then insertion order win.
    pub fn keyword(mut self, word: impl Into<String>, color: Color) -> Self {
        self.keywords.push((word.into(), color));
        self
    }

    fn runs(&self, source: &str) -> Result<Vec<(String, Color)>, ValidationError> {
        let text = source.replace('\t', "    ");
        if text.len() > 32768 {
            return Err(invalid("Subtitle exceeds expanded UTF-8 byte limit"));
        }
        let mut boundaries = text
            .grapheme_indices(true)
            .map(|(index, _)| index)
            .collect::<Vec<_>>();
        boundaries.push(text.len());
        let mut runs = Vec::new();
        let mut cursor = 0;
        let mut plain = 0;
        for &offset in &boundaries {
            if offset < cursor || offset == text.len() {
                continue;
            }
            if let Some((word, color)) = self
                .keywords
                .iter()
                .filter(|(word, _)| {
                    text[offset..].starts_with(word)
                        && boundaries.binary_search(&(offset + word.len())).is_ok()
                })
                .min_by_key(|(word, _)| std::cmp::Reverse(word.len()))
            {
                if offset > plain {
                    runs.push((text[plain..offset].to_owned(), self.color));
                }
                runs.push((word.clone(), *color));
                cursor = offset + word.len();
                plain = cursor;
                if runs.len() > 256 {
                    return Err(invalid("Subtitle keyword styling exceeds 256 runs"));
                }
            }
        }
        if plain < text.len() {
            runs.push((text[plain..].to_owned(), self.color));
        }
        if runs.len() > 256 {
            return Err(invalid("Subtitle keyword styling exceeds 256 runs"));
        }
        Ok(runs)
    }

    /// Measures with supplied fonts and freezes vector artwork before frame evaluation.
    /// At most 1024 cues/128 KiB source and explicit per-cue line/safe-area limits apply.
    pub fn layout(
        &self,
        track: &CaptionTrack,
        canvas: Size,
        fonts: &FontLibrary,
    ) -> motion_core::Result<CaptionLayout> {
        let safe = self.safe;
        if ![
            canvas.width,
            canvas.height,
            safe.left,
            safe.top,
            safe.right,
            safe.bottom,
            self.size,
            self.padding.0,
            self.padding.1,
            self.gap,
        ]
        .iter()
        .all(|value| value.is_finite())
            || canvas.width <= 0.0
            || canvas.height <= 0.0
            || canvas.width > 65536.0
            || canvas.height > 65536.0
            || safe.is_empty()
            || safe.left < 0.0
            || safe.top < 0.0
            || safe.right > canvas.width
            || safe.bottom > canvas.height
            || self.size <= 0.0
            || self.size > 4096.0
            || !(1..=8).contains(&self.lines)
            || [self.padding.0, self.padding.1, self.gap]
                .iter()
                .any(|value| !(0.0..=4096.0).contains(value))
            || safe.width() <= 2.0 * self.padding.0
            || safe.height() <= 2.0 * self.padding.1
            || self.keywords.len() > 16
            || self.keywords.iter().any(|(word, _)| {
                word.is_empty() || word.len() > 128 || word.chars().any(char::is_control)
            })
            || track.cues().len() > 1024
            || track
                .cues()
                .iter()
                .map(|cue| cue.text().len())
                .sum::<usize>()
                > 131072
        {
            return Err(invalid("Invalid subtitle style, safe area or layout budget").into());
        }
        let context = DrawContext::default().with_fonts(fonts.clone());
        let mut entries = Vec::new();
        let mut events = Vec::new();
        for (index, cue) in track.cues().iter().enumerate() {
            let spans = self
                .runs(cue.text())?
                .into_iter()
                .map(|(text, color)| TextSpan::new(text).size(self.size).fill(color));
            let text = Text::rich(spans)
                .size(self.size)
                .line_height(1.0 + 6.0 / self.size)
                .alignment(TextAlignment::Center)
                .max_width(safe.width() - 2.0 * self.padding.0)
                .fonts(fonts);
            let layout = text.layout(fonts)?;
            if layout.lines().len() > self.lines {
                return Err(
                    invalid("Subtitle exceeds its line limit; edit or retime the cue").into(),
                );
            }
            let node = text.vector_tree(&context)?.node();
            entries.push((cue.clone(), layout, node));
            events.push((cue.start(), true, index));
            events.push((cue.end(), false, index));
        }
        events.sort_unstable();
        let mut active = Vec::new();
        for (_, entering, index) in events {
            if entering {
                active.push(index);
            } else {
                active.retain(|other| *other != index);
            }
            let height = active
                .iter()
                .map(|index| entries[*index].1.bounds().height())
                .sum::<f64>()
                + self.gap * active.len().saturating_sub(1) as f64
                + 2.0 * self.padding.1;
            if height > safe.height() {
                return Err(invalid("Simultaneous subtitles exceed safe-area height").into());
            }
        }
        Ok(CaptionLayout {
            style: self.clone(),
            entries,
        })
    }
}

/// Cached subtitle vectors; frame sampling never reshapes or discovers fonts.
#[derive(Clone, Debug)]
pub struct CaptionLayout {
    style: CaptionStyle,
    entries: Vec<(CaptionCue, Arc<ParagraphLayout>, Node)>,
}

impl CaptionLayout {
    /// Returns immutable cue/paragraph pairs in input order.
    pub fn cues(&self) -> impl Iterator<Item = (&CaptionCue, &ParagraphLayout)> {
        self.entries
            .iter()
            .map(|(cue, layout, _)| (cue, layout.as_ref()))
    }

    /// Returns the validated safe rectangle.
    pub fn safe_area(&self) -> Rect {
        self.style.safe
    }

    fn active(
        &self,
        time: Time,
    ) -> impl Iterator<Item = &(CaptionCue, Arc<ParagraphLayout>, Node)> {
        let ticks = u128::try_from(time.ticks()).ok();
        self.entries.iter().filter(move |(cue, _, _)| {
            ticks.is_some_and(|ticks| {
                let position = ticks * 1_000_000_000;
                let rate = motion_core::TICKS_PER_SECOND as u128;
                cue.start().as_nanos() * rate <= position && position < cue.end().as_nanos() * rate
            })
        })
    }

    /// Samples half-open cues and stacks overlaps bottom-up in input order.
    pub fn node(&self, time: Time) -> Node {
        let mut bottom = self.style.safe.bottom - self.style.padding.1;
        let mut nodes = Vec::new();
        for (_, layout, node) in self.active(time) {
            if !nodes.is_empty() {
                bottom -= self.style.gap;
            }
            bottom -= layout.bounds().height();
            nodes.push(
                node.clone()
                    .translate(self.style.safe.left + self.style.padding.0, bottom),
            );
        }
        if nodes.is_empty() {
            return Node::empty();
        }
        let background = Node::from(
            Rect::new(
                self.style.safe.left,
                bottom - self.style.padding.1,
                self.style.safe.right,
                self.style.safe.bottom,
            )
            .fill(self.style.background),
        );
        Node::group([background, Node::group(nodes)])
            .clip_with(self.style.safe, motion_core::ClipMode::Geometric)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use motion_core::{FrameRate, PixelSize, TimeLiteral, ValidationLimits};
    use std::time::Duration;

    #[test]
    fn keyword_runs_remain_grapheme_safe_longest_first_and_tab_expanded() -> motion_core::Result {
        let style = CaptionStyle::new(Rect::new(0.0, 0.0, 600.0, 200.0))
            .keyword("Moon", Color::BLACK)
            .keyword("Moonlight", Color::WHITE);
        let runs = style.runs("Earth\tMoonlight Moon")?;
        assert_eq!(
            runs.iter()
                .map(|(text, _)| text.as_str())
                .collect::<String>(),
            "Earth    Moonlight Moon"
        );
        assert_eq!(runs[1], ("Moonlight".into(), Color::WHITE));
        let style = CaptionStyle::new(Rect::new(0.0, 0.0, 600.0, 200.0)).keyword("e", Color::BLACK);
        assert_eq!(style.runs("e\u{301}")?.len(), 1);
        Ok(())
    }

    #[test]
    #[ignore = "requires explicit MOTION_TEST_FONT; no system discovery"]
    fn subtitles_keep_safe_area_half_open_overlaps_and_overflow_errors() -> motion_core::Result {
        let mut fonts = FontLibrary::new();
        fonts.load_file(
            std::env::var_os("MOTION_TEST_FONT").ok_or_else(|| invalid("Set MOTION_TEST_FONT"))?,
        )?;
        let track = CaptionTrack::new([
            CaptionCue::new(Duration::ZERO, Duration::from_secs(2), "Earth\tMoon")?,
            CaptionCue::new(Duration::from_secs(1), Duration::from_secs(3), "Sun")?,
        ])?;
        let style = CaptionStyle::new(Rect::new(20.0, 100.0, 620.0, 340.0))
            .keyword("Moon", Color::from_hex(0x118677));
        let layout = style.layout(&track, Size::new(640.0, 360.0), &fonts)?;
        for (index, count) in [(0, 1), (29, 1), (30, 2), (59, 2), (60, 1), (89, 1), (90, 0)] {
            let time = Time::from_frame(index, FrameRate::from(30));
            assert_eq!(layout.active(time).count(), count);
            assert_eq!(track.active_at_time(time).count(), count);
            layout.node(time).compile(
                Size::new(640.0, 360.0),
                PixelSize::new(320, 180),
                ValidationLimits::default(),
            )?;
        }
        let height = layout.entries[0].1.bounds().height();
        assert!(
            CaptionStyle::new(Rect::new(0.0, 0.0, 600.0, height + 16.0))
                .layout(&track, Size::new(640.0, 360.0), &fonts)
                .is_err()
        );
        let multiline = CaptionTrack::new([CaptionCue::new(
            Duration::ZERO,
            Duration::from_secs(1),
            "one\ntwo",
        )?])?;
        assert!(
            style
                .clone()
                .max_lines(1)
                .layout(&multiline, Size::new(640.0, 360.0), &fonts)
                .is_err()
        );
        assert!(
            style
                .layout(&track, Size::new(320.0, 180.0), &fonts)
                .is_err()
        );
        assert!(
            layout
                .node(500.milliseconds())
                .measure(ValidationLimits::default())
                .is_ok()
        );
        Ok(())
    }
}
