//! Serializable spec enumerations whose JSON form differs from the core type.

use motion_core::{Pivot, Point};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

/// Local anchor for placement, rotation and scale; mirrors [`Pivot`].
#[derive(Clone, Copy, Debug, Default, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum PivotSpec {
    /// Upper-left corner.
    TopLeft,
    /// Midpoint of the upper edge.
    Top,
    /// Upper-right corner.
    TopRight,
    /// Midpoint of the left edge.
    Left,
    /// Bounds midpoint.
    #[default]
    Center,
    /// Midpoint of the right edge.
    Right,
    /// Lower-left corner.
    BottomLeft,
    /// Midpoint of the lower edge.
    Bottom,
    /// Lower-right corner.
    BottomRight,
    /// Text baseline origin, or bottom-left without baseline metrics.
    Baseline,
    /// Fractions `[x, y]` of the bounds, without clamping.
    Relative([f64; 2]),
    /// Explicit local point `[x, y]`.
    Point([f64; 2]),
}

impl From<PivotSpec> for Pivot {
    fn from(pivot: PivotSpec) -> Self {
        match pivot {
            PivotSpec::TopLeft => Self::TopLeft,
            PivotSpec::Top => Self::Top,
            PivotSpec::TopRight => Self::TopRight,
            PivotSpec::Left => Self::Left,
            PivotSpec::Center => Self::Center,
            PivotSpec::Right => Self::Right,
            PivotSpec::BottomLeft => Self::BottomLeft,
            PivotSpec::Bottom => Self::Bottom,
            PivotSpec::BottomRight => Self::BottomRight,
            PivotSpec::Baseline => Self::Baseline,
            PivotSpec::Relative([x, y]) => Self::Relative(x, y),
            PivotSpec::Point([x, y]) => Self::Point(Point::new(x, y)),
        }
    }
}

/// Stroke corner shape; the miter limit is a separate stroke field.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum LineJoinSpec {
    /// Sharp corner, beveled beyond the miter limit.
    #[default]
    Miter,
    /// Circular corner.
    Round,
    /// Straight cut corner.
    Bevel,
}

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
