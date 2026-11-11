//! Rulers, callouts and line charts described as plain data.

use motion_core::{Color, LineCap, LineJoin, Point, Rect};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

/// Value-to-coordinate mapping type.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum ScaleKind {
    /// Linear interpolation.
    #[default]
    Linear,
    /// Base-ten logarithm; the domain must be positive.
    Log10,
}

/// A numeric scale from a data domain to a coordinate range.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ScaleSpec {
    /// Data endpoints `[start, end]`, distinct and finite.
    pub domain: [f64; 2],
    /// Coordinate endpoints `[start, end]`; required by rulers, derived by charts.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub range: Option<[f64; 2]>,
    /// Scale type.
    #[serde(default, skip_serializing_if = "is_linear")]
    pub kind: ScaleKind,
    /// Clamp mapped values to the range.
    #[serde(default, skip_serializing_if = "crate::value::is_false")]
    pub clamp: bool,
}

impl ScaleSpec {
    /// Creates a linear scale over `domain`.
    pub fn linear(domain: [f64; 2]) -> Self {
        Self {
            domain,
            range: None,
            kind: ScaleKind::Linear,
            clamp: false,
        }
    }

    /// Sets the coordinate range.
    pub fn range(mut self, range: [f64; 2]) -> Self {
        self.range = Some(range);
        self
    }
}

fn is_linear(kind: &ScaleKind) -> bool {
    *kind == ScaleKind::Linear
}

/// Ruler direction.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum OrientationSpec {
    /// Ticks along X at a fixed Y baseline.
    #[default]
    Horizontal,
    /// Ticks along Y at a fixed X baseline.
    Vertical,
}

/// A labeled axis ruler. Labels are text, so they use the render font environment;
/// documents that declare `fonts` supply it through `CompiledDocument::render_options`.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct RulerSpec {
    /// Axis direction.
    #[serde(default, skip_serializing_if = "is_horizontal")]
    pub orientation: OrientationSpec,
    /// Fixed coordinate of the axis line.
    pub baseline: f64,
    /// Value scale with an explicit `range`.
    pub scale: ScaleSpec,
    /// Tick count in `2..=256`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub ticks: Option<u32>,
    /// Round tick values to nice numbers.
    #[serde(default, skip_serializing_if = "crate::value::is_false")]
    pub nice: bool,
    /// Tick mark length.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tick_length: Option<f64>,
    /// Axis line width.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub line_width: Option<f64>,
    /// Label font size in `(0, 4096]`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub font_size: Option<f64>,
    /// Reserved label box `[width, height]`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub label_size: Option<[f64; 2]>,
    /// Line and label color.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub color: Option<Color>,
    /// Unit suffix appended to labels.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub unit: Option<String>,
}

fn is_horizontal(orientation: &OrientationSpec) -> bool {
    *orientation == OrientationSpec::Horizontal
}

impl RulerSpec {
    /// Creates a horizontal ruler with default styling.
    pub fn horizontal(baseline: f64, scale: ScaleSpec) -> Self {
        Self {
            orientation: OrientationSpec::Horizontal,
            baseline,
            scale,
            ticks: None,
            nice: false,
            tick_length: None,
            line_width: None,
            font_size: None,
            label_size: None,
            color: None,
            unit: None,
        }
    }
}

/// A text label with a leader line to a target point.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct CalloutSpec {
    /// Label text.
    pub text: String,
    /// Annotated point `[x, y]`.
    pub target: Point,
    /// Label box `[left, top, right, bottom]`.
    pub bounds: Rect,
    /// Text and leader color.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub color: Option<Color>,
    /// Font size in `(0, 4096]`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub font_size: Option<f64>,
    /// Positive leader width.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub leader_width: Option<f64>,
}

/// A multi-series line chart with grid, axes and non-overlapping labels. Axis ranges
/// are derived from `bounds`; labels use the render font environment.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct LineChartSpec {
    /// Plot rectangle `[left, top, right, bottom]`.
    pub bounds: Rect,
    /// Horizontal data scale; its `range` is derived.
    pub horizontal: ScaleSpec,
    /// Vertical data scale; its `range` is derived.
    pub vertical: ScaleSpec,
    /// Grid and label tick count in `2..=32`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub ticks: Option<u32>,
    /// Up to 32 data series.
    pub series: Vec<SeriesSpec>,
}

/// One chart series.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct SeriesSpec {
    /// Points `[[x, y], ...]`; `null` entries break the curve.
    pub points: Vec<Option<Point>>,
    /// Curve color.
    pub color: Color,
    /// Curve width; 2 by default.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub width: Option<f64>,
    /// End cap; round by default.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cap: Option<LineCap>,
    /// Corner join; round by default.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub join: Option<LineJoin>,
}
