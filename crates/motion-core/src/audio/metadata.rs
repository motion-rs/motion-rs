//! Bounded container tags and chapters attached to exported audio.

use super::invalid;
use crate::{Time, ValidationError};

/// Maximum container tags in one metadata request.
const MAX_TAGS: usize = 64;

/// Maximum chapters in one metadata request.
const MAX_CHAPTERS: usize = 4096;

/// Maximum UTF-8 bytes in a tag key.
const MAX_TAG_KEY_BYTES: usize = 128;

/// Maximum UTF-8 bytes in a tag value or chapter title.
const MAX_TEXT_BYTES: usize = 4096;

/// Latest chapter end, in seconds (one day).
const MAX_CHAPTER_END_SECONDS: f64 = 86400.0;

/// Container chapter in the same integer tick clock as the timeline.
#[derive(Clone, Debug)]
pub struct AudioChapter {
    /// Inclusive chapter start.
    pub start: Time,
    /// Exclusive chapter end.
    pub end: Time,
    /// Chapter title, at most 4096 bytes.
    pub title: String,
}

/// Optional media tags and chapters; empty metadata writes none.
#[derive(Clone, Debug, Default)]
pub struct AudioMetadata {
    /// Container tags, including title, language, R128_TRACK_GAIN or ReplayGain.
    pub tags: std::collections::BTreeMap<String, String>,
    /// Ordered, nonoverlapping chapters in timeline coordinates.
    pub chapters: Vec<AudioChapter>,
}

impl AudioMetadata {
    /// Validates finite tick ranges, byte limits and NUL-free tags before opening output.
    pub fn validate(&self) -> Result<(), ValidationError> {
        if self.tags.len() > MAX_TAGS
            || self.chapters.len() > MAX_CHAPTERS
            || self.tags.iter().any(|(key, value)| {
                key.is_empty()
                    || key.len() > MAX_TAG_KEY_BYTES
                    || value.len() > MAX_TEXT_BYTES
                    || key.contains('\0')
                    || value.contains('\0')
            })
            || self.chapters.iter().any(|chapter| {
                chapter.start < Time::ZERO
                    || chapter.end <= chapter.start
                    || chapter.end.as_seconds() > MAX_CHAPTER_END_SECONDS
                    || chapter.title.len() > MAX_TEXT_BYTES
                    || chapter.title.contains('\0')
            })
            || self
                .chapters
                .windows(2)
                .any(|pair| pair[0].end > pair[1].start)
        {
            return Err(invalid(
                "metadata",
                "Invalid bounded container tags or ordered chapters",
            ));
        }
        Ok(())
    }

    /// Whether the request leaves container metadata unchanged.
    pub fn is_empty(&self) -> bool {
        self.tags.is_empty() && self.chapters.is_empty()
    }
}
