//! Timed caption tracks described as plain data.

use crate::value::ColorValue;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

/// Timed captions laid out inside a safe area. Layout uses the document `fonts`.
///
/// Give `cues` inline or `srt` as a file path, not both. Cue intervals are exact
/// half-open `[start, end)` in scene-local seconds; simultaneous cues stack upward.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct CaptionsSpec {
    /// Inline cues, at most 1024.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub cues: Vec<CueSpec>,
    /// SubRip file, relative to the document directory.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub srt: Option<String>,
    /// Safe area `[left, top, right, bottom]`; the whole canvas by default.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub safe: Option<[f64; 4]>,
    /// Font size in `(0, 4096]`; 30 by default.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub font_size: Option<f64>,
    /// Text color; white by default.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub color: Option<ColorValue>,
    /// Box color; translucent black by default.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub background: Option<ColorValue>,
    /// Box padding `[horizontal, vertical]`; `[12, 8]` by default.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub padding: Option<[f64; 2]>,
    /// Gap between stacked cues; 8 by default.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub gap: Option<f64>,
    /// Most lines per cue in `1..=8`; 2 by default.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max_lines: Option<u32>,
    /// Words drawn in their own color, at most 16.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub keywords: Vec<KeywordSpec>,
}

impl CaptionsSpec {
    /// Creates captions from inline cues.
    pub fn new(cues: impl IntoIterator<Item = CueSpec>) -> Self {
        Self {
            cues: cues.into_iter().collect(),
            ..Self::default()
        }
    }
}

/// One caption cue.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct CueSpec {
    /// Start in seconds.
    pub start: f64,
    /// End in seconds, after `start`.
    pub end: f64,
    /// Literal cue text.
    pub text: String,
}

impl CueSpec {
    /// Creates a cue over `[start, end)`.
    pub fn new(start: f64, end: f64, text: impl Into<String>) -> Self {
        Self {
            start,
            end,
            text: text.into(),
        }
    }
}

/// A literal word and the color it is drawn in.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct KeywordSpec {
    /// Word to match, longest first.
    pub word: String,
    /// Replacement color.
    pub color: ColorValue,
}
