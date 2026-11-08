//! Math formulas and shape morphs described as plain data.

use crate::value::{Animated, ColorValue};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

/// Typst math drawn as vector shapes; requires feature `formula`.
///
/// `source` is Typst math syntax without the surrounding `$`. With `morph_to`, the
/// formula blends into the second formula as `progress` goes from 0 to 1.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct FormulaSpec {
    /// Typst math source, 1 to 8192 bytes.
    pub source: String,
    /// Logical size in `1..=1024`; 24 by default.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub size: Option<f64>,
    /// Fill color; white by default.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub color: Option<ColorValue>,
    /// Math font family.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub family: Option<String>,
    /// Handwriting progress in `0..=1`; fully drawn when omitted.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub write: Option<Animated<f64>>,
    /// Target formula to morph into.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub morph_to: Option<String>,
    /// Morph progress in `0..=1`; required with `morph_to`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub progress: Option<Animated<f64>>,
}

impl FormulaSpec {
    /// Creates an unstyled formula.
    pub fn new(source: impl Into<String>) -> Self {
        Self {
            source: source.into(),
            size: None,
            color: None,
            family: None,
            write: None,
            morph_to: None,
            progress: None,
        }
    }

    /// Sets the logical size.
    pub fn size_of(mut self, size: f64) -> Self {
        self.size = Some(size);
        self
    }

    /// Animates handwriting progress.
    pub fn write_with(mut self, progress: impl Into<Animated<f64>>) -> Self {
        self.write = Some(progress.into());
        self
    }

    /// Morphs into `target` as `progress` advances.
    pub fn morph_to(
        mut self,
        target: impl Into<String>,
        progress: impl Into<Animated<f64>>,
    ) -> Self {
        self.morph_to = Some(target.into());
        self.progress = Some(progress.into());
        self
    }
}

/// A path that morphs into another path as `progress` goes from 0 to 1.
///
/// Fill and stroke come from the element. The contours are matched by arc length, so
/// the paths need not have the same structure; overshooting progress extrapolates.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct MorphSpec {
    /// Start path, as SVG path data.
    pub from: String,
    /// End path, as SVG path data.
    pub to: String,
    /// Morph progress.
    pub progress: Animated<f64>,
    /// Arc-length samples per contour in `4..=256`; 32 by default.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub samples: Option<u32>,
}

/// An SVG file that morphs into another SVG file; requires feature `svg`.
///
/// Painted parts are matched by id, then by position and shape; unmatched parts grow
/// or shrink around their centers. Progress is clamped to `0..=1`.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct SvgMorphSpec {
    /// Start SVG file, relative to the document directory.
    pub from: String,
    /// End SVG file, relative to the document directory.
    pub to: String,
    /// Morph progress.
    pub progress: Animated<f64>,
}
