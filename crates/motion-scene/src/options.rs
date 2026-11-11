//! Serializable document enumerations for image fit and text alignment.

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

/// Image placement inside its viewport.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum FitSpec {
    /// Stretch each axis.
    Fill,
    /// Preserve aspect ratio inside the viewport.
    #[default]
    Contain,
    /// Preserve aspect ratio and crop.
    Cover,
    /// Keep source pixel dimensions.
    None,
}

/// Paragraph alignment.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum TextAlignmentSpec {
    /// Paragraph start.
    #[default]
    Start,
    /// Paragraph end.
    End,
    /// Centered lines.
    Center,
    /// Justified nonfinal lines.
    Justified,
}
