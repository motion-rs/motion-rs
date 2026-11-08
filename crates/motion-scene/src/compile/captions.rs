//! Compilation of caption tracks into laid-out, time-selected nodes.

use super::Compiler;
use super::element::Built;
use crate::captions::CaptionsSpec;

#[cfg(feature = "captions")]
use motion_content::captions::{CaptionCue, CaptionStyle, CaptionTrack};
#[cfg(feature = "captions")]
use motion_core::{Rect, Size};
#[cfg(feature = "captions")]
use std::sync::Arc;

/// Largest accepted SubRip file in bytes.
#[cfg(feature = "captions")]
const MAX_SRT_BYTES: u64 = 8 * 1024 * 1024;

impl Compiler<'_> {
    /// Lays out every cue against the document fonts and selects them by scene time.
    #[cfg(feature = "captions")]
    pub(crate) fn captions(&mut self, spec: &CaptionsSpec, path: &str) -> Option<Built> {
        let Some(fonts) = self.fonts.clone() else {
            self.error(
                path,
                "fonts",
                "Captions require the document to declare `fonts`",
            );
            return None;
        };
        let track = self.caption_track(spec, path)?;
        let [left, top, right, bottom] =
            spec.safe
                .unwrap_or([0.0, 0.0, self.canvas.width, self.canvas.height]);
        let mut style = CaptionStyle::new(Rect::new(left, top, right, bottom));
        if let Some(size) = spec.font_size {
            style = style.font_size(size);
        }
        if let Some(color) = &spec.color {
            style = style.color(color.color());
        }
        if let Some(color) = &spec.background {
            style = style.background(color.color());
        }
        if let Some([horizontal, vertical]) = spec.padding {
            style = style.padding(horizontal, vertical);
        }
        if let Some(gap) = spec.gap {
            style = style.gap(gap);
        }
        if let Some(lines) = spec.max_lines {
            style = style.max_lines(lines as usize);
        }
        for keyword in &spec.keywords {
            style = style.keyword(keyword.word.as_str(), keyword.color.color());
        }
        let canvas = Size::new(self.canvas.width, self.canvas.height);
        let layout = style
            .layout(&track, canvas, &fonts)
            .map_err(|error| self.error(path, "captions", error.to_string()))
            .ok()?;
        let layout = Arc::new(layout);
        Some(Built::new(true, move |time| layout.node(time)))
    }

    #[cfg(feature = "captions")]
    fn caption_track(&mut self, spec: &CaptionsSpec, path: &str) -> Option<CaptionTrack> {
        if spec.srt.is_some() == !spec.cues.is_empty() {
            self.error(path, "cues", "Give either inline `cues` or an `srt` file");
            return None;
        }
        if let Some(source) = &spec.srt {
            let file = self.resolve(source, &format!("{path}.srt"))?;
            let length = std::fs::metadata(&file)
                .map_err(|error| self.error(path, "srt", error.to_string()))
                .ok()?
                .len();
            if length > MAX_SRT_BYTES {
                self.limit(path, "srt", "SubRip files are limited to 8 MiB");
                return None;
            }
            let text = std::fs::read_to_string(&file)
                .map_err(|error| self.error(path, "srt", error.to_string()))
                .ok()?;
            return CaptionTrack::from_srt(&text)
                .map_err(|error| self.error(path, "srt", error.message))
                .ok();
        }
        let mut cues = Vec::with_capacity(spec.cues.len());
        for (index, cue) in spec.cues.iter().enumerate() {
            let here = format!("{path}.cues[{index}]");
            let start = self.seconds(cue.start, &here, "start");
            let end = self.seconds(cue.end, &here, "end");
            if let (Some(start), Some(end)) = (start, end) {
                let (start, end) = (start.as_seconds(), end.as_seconds());
                let (start, end) = (
                    std::time::Duration::from_secs_f64(start),
                    std::time::Duration::from_secs_f64(end),
                );
                match CaptionCue::new(start, end, cue.text.as_str()) {
                    Ok(cue) => cues.push(cue),
                    Err(error) => self.error(&here, "cue", error.message),
                }
            }
        }
        if cues.len() != spec.cues.len() {
            return None;
        }
        CaptionTrack::new(cues)
            .map_err(|error| self.error(path, "cues", error.message))
            .ok()
    }

    #[cfg(not(feature = "captions"))]
    pub(crate) fn captions(&mut self, _: &CaptionsSpec, path: &str) -> Option<Built> {
        self.unsupported(path, "Captions require feature `captions`");
        None
    }
}
