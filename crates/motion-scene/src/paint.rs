//! Fill/stroke paint, gradients, stroke geometry and trim ranges.

use crate::options::LineJoinSpec;
use crate::value::{Animated, ColorValue};
use motion_core::{Color, GradientExtend, GradientSpace, GradientUnits, LineCap};
use schemars::{JsonSchema, Schema, SchemaGenerator, json_schema};
use serde::de::Error as _;
use serde::{Deserialize, Deserializer, Serialize, Serializer};
use std::borrow::Cow;

/// Gradient tags that distinguish a gradient object from an animated color.
const GRADIENT_TAGS: [&str; 3] = ["linear", "radial", "sweep"];

/// Solid (optionally animated) color or a gradient.
///
/// JSON writes a solid paint as an [`Animated`] color (`"#fff"` or
/// `{"tween": ...}`) and a gradient as `{"linear": ...}`, `{"radial": ...}` or
/// `{"sweep": ...}`.
#[derive(Clone, Debug, PartialEq)]
pub enum PaintSpec {
    /// A fixed or animated color.
    Solid(Animated<ColorValue>),
    /// A static gradient.
    Gradient(GradientSpec),
}

impl From<Color> for PaintSpec {
    fn from(color: Color) -> Self {
        Self::Solid(color.into())
    }
}

impl From<ColorValue> for PaintSpec {
    fn from(color: ColorValue) -> Self {
        Self::Solid(color.into())
    }
}

impl From<Animated<ColorValue>> for PaintSpec {
    fn from(color: Animated<ColorValue>) -> Self {
        Self::Solid(color)
    }
}

impl From<GradientSpec> for PaintSpec {
    fn from(gradient: GradientSpec) -> Self {
        Self::Gradient(gradient)
    }
}

impl Serialize for PaintSpec {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        match self {
            Self::Solid(color) => color.serialize(serializer),
            Self::Gradient(gradient) => gradient.serialize(serializer),
        }
    }
}

impl<'de> Deserialize<'de> for PaintSpec {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let value = serde_json::Value::deserialize(deserializer)?;
        let gradient = value
            .as_object()
            .is_some_and(|object| GRADIENT_TAGS.iter().any(|tag| object.contains_key(*tag)));
        if gradient {
            GradientSpec::deserialize(value)
                .map(Self::Gradient)
                .map_err(D::Error::custom)
        } else {
            Animated::deserialize(value)
                .map(Self::Solid)
                .map_err(D::Error::custom)
        }
    }
}

impl JsonSchema for PaintSpec {
    fn schema_name() -> Cow<'static, str> {
        "Paint".into()
    }

    fn json_schema(generator: &mut SchemaGenerator) -> Schema {
        let solid = generator.subschema_for::<Animated<ColorValue>>();
        let gradient = generator.subschema_for::<GradientSpec>();
        json_schema!({
            "description": "A fixed or animated color, or a gradient.",
            "anyOf": [solid, gradient]
        })
    }
}

/// A gradient with ordered color stops.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case", deny_unknown_fields)]
pub enum GradientSpec {
    /// Gradient along a line.
    Linear(LinearGradientSpec),
    /// Gradient from a center or focal point.
    Radial(RadialGradientSpec),
    /// Clockwise angular gradient.
    Sweep(SweepGradientSpec),
}

/// A linear gradient between two points.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct LinearGradientSpec {
    /// Point at offset zero, `[x, y]`.
    pub start: [f64; 2],
    /// Point at offset one, `[x, y]`.
    pub end: [f64; 2],
    /// Stops ordered by offset.
    pub stops: Vec<GradientStop>,
    /// Shared interpolation, coordinate and extension options.
    #[serde(default, skip_serializing_if = "GradientOptions::is_default")]
    pub options: GradientOptions,
}

impl LinearGradientSpec {
    /// Creates a gradient from `start` to `end`.
    pub fn new(start: [f64; 2], end: [f64; 2]) -> Self {
        Self {
            start,
            end,
            stops: Vec::new(),
            options: GradientOptions::default(),
        }
    }

    /// Appends a stop.
    pub fn stop(mut self, offset: f64, color: impl Into<ColorValue>) -> Self {
        self.stops.push(GradientStop {
            offset,
            color: color.into(),
        });
        self
    }
}

/// A radial gradient, optionally with an offset focal point.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct RadialGradientSpec {
    /// End-circle center `[x, y]`.
    pub center: [f64; 2],
    /// End-circle radius.
    pub radius: f64,
    /// Optional start point `[x, y]` inside the end circle.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub focal: Option<[f64; 2]>,
    /// Stops ordered by offset.
    pub stops: Vec<GradientStop>,
    /// Shared interpolation, coordinate and extension options.
    #[serde(default, skip_serializing_if = "GradientOptions::is_default")]
    pub options: GradientOptions,
}

impl RadialGradientSpec {
    /// Creates a centered radial gradient.
    pub fn new(center: [f64; 2], radius: f64) -> Self {
        Self {
            center,
            radius,
            focal: None,
            stops: Vec::new(),
            options: GradientOptions::default(),
        }
    }

    /// Appends a stop.
    pub fn stop(mut self, offset: f64, color: impl Into<ColorValue>) -> Self {
        self.stops.push(GradientStop {
            offset,
            color: color.into(),
        });
        self
    }
}

/// A clockwise angular gradient; angles are degrees from +X.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct SweepGradientSpec {
    /// Rotation center `[x, y]`.
    pub center: [f64; 2],
    /// Angle of offset zero, in degrees.
    pub start_angle: f64,
    /// Angle of offset one, in degrees.
    pub end_angle: f64,
    /// Stops ordered by offset.
    pub stops: Vec<GradientStop>,
    /// Shared interpolation, coordinate and extension options.
    #[serde(default, skip_serializing_if = "GradientOptions::is_default")]
    pub options: GradientOptions,
}

impl SweepGradientSpec {
    /// Creates a sweep between two angles in degrees.
    pub fn new(center: [f64; 2], start_angle: f64, end_angle: f64) -> Self {
        Self {
            center,
            start_angle,
            end_angle,
            stops: Vec::new(),
            options: GradientOptions::default(),
        }
    }

    /// Appends a stop.
    pub fn stop(mut self, offset: f64, color: impl Into<ColorValue>) -> Self {
        self.stops.push(GradientStop {
            offset,
            color: color.into(),
        });
        self
    }
}

/// One gradient color at a normalized offset.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct GradientStop {
    /// Position in `0..=1`.
    pub offset: f64,
    /// Stop color.
    pub color: ColorValue,
}

/// Optional gradient settings; omitted values keep engine defaults.
#[derive(Clone, Copy, Debug, Default, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct GradientOptions {
    /// Interpolation color space.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub space: Option<GradientSpace>,
    /// Coordinate system of the gradient geometry.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub units: Option<GradientUnits>,
    /// Behavior outside the stop range.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub extend: Option<GradientExtend>,
}

impl GradientOptions {
    /// Reports that every option keeps its default.
    pub fn is_default(&self) -> bool {
        *self == Self::default()
    }
}

/// A stroke drawn after the fill.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct StrokeSpec {
    /// Full logical width.
    pub width: Animated<f64>,
    /// Stroke paint.
    pub paint: PaintSpec,
    /// End cap; butt by default.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cap: Option<LineCap>,
    /// Corner join; miter by default.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub join: Option<LineJoinSpec>,
    /// Miter-length to half-width ratio in `1..=1024`; four by default.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub miter_limit: Option<f64>,
    /// Alternating dash and gap lengths.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub dash: Vec<f64>,
    /// Dash phase in design units; animate for marching dashes.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub dash_offset: Option<Animated<f64>>,
}

impl StrokeSpec {
    /// Creates a butt-capped, miter-joined stroke.
    pub fn new(width: impl Into<Animated<f64>>, paint: impl Into<PaintSpec>) -> Self {
        Self {
            width: width.into(),
            paint: paint.into(),
            cap: None,
            join: None,
            miter_limit: None,
            dash: Vec::new(),
            dash_offset: None,
        }
    }

    /// Selects the end cap.
    pub fn cap(mut self, cap: LineCap) -> Self {
        self.cap = Some(cap);
        self
    }

    /// Selects the corner join.
    pub fn join(mut self, join: LineJoinSpec) -> Self {
        self.join = Some(join);
        self
    }

    /// Dashes the stroke with alternating lengths and a phase.
    pub fn dash(mut self, pattern: impl Into<Vec<f64>>, offset: impl Into<Animated<f64>>) -> Self {
        self.dash = pattern.into();
        self.dash_offset = Some(offset.into());
        self
    }
}

/// Normalized arc-length range drawn from the source geometry.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct TrimSpec {
    /// Start fraction; zero by default.
    #[serde(default = "zero", skip_serializing_if = "is_zero")]
    pub start: Animated<f64>,
    /// End fraction; one by default.
    #[serde(default = "one", skip_serializing_if = "is_one")]
    pub end: Animated<f64>,
}

impl TrimSpec {
    /// Draws the range from `start` to `end`.
    pub fn new(start: impl Into<Animated<f64>>, end: impl Into<Animated<f64>>) -> Self {
        Self {
            start: start.into(),
            end: end.into(),
        }
    }
}

fn zero() -> Animated<f64> {
    Animated::Fixed(0.0)
}

fn one() -> Animated<f64> {
    Animated::Fixed(1.0)
}

fn is_zero(value: &Animated<f64>) -> bool {
    *value == zero()
}

fn is_one(value: &Animated<f64>) -> bool {
    *value == one()
}
