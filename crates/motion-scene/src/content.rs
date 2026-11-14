//! Element content kinds: geometry, text, images and SVG.

use crate::captions::CaptionsSpec;
use crate::diagram::{CalloutSpec, LineChartSpec, RulerSpec};
use crate::element::Element;
use crate::formula::{FormulaSpec, MorphSpec, SvgMorphSpec};
use crate::ink::{BrushStrokeSpec, InkFillSpec};
use crate::particles::{ParticlesSpec, TrajectorySpec};
use crate::value::Animated;
use motion_content::{Fit, TextAlignment, TextSplit, WriteOptions};
use motion_core::{ArrowHead, Color, Point, Rect, Sampling};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

/// What an element draws. Documents select it with exactly one content key.
#[derive(Clone, Debug, Default, PartialEq)]
pub enum Content {
    /// Draws nothing; transforms and effects still apply.
    #[default]
    Empty,
    /// Child elements in stable z-index and insertion order (`children`).
    Group(Vec<Element>),
    /// Axis-aligned rectangle, optionally rounded.
    Rect(RectSpec),
    /// Circle.
    Circle(CircleSpec),
    /// Axis-aligned ellipse.
    Ellipse(EllipseSpec),
    /// Straight segment.
    Line(LineSpec),
    /// Open circular arc.
    Arc(ArcSpec),
    /// Closed polygon through points.
    Polygon(PointsSpec),
    /// Open polyline through points.
    Polyline(PointsSpec),
    /// Regular polygon.
    RegularPolygon(RegularPolygonSpec),
    /// Star.
    Star(StarSpec),
    /// Annulus or annular sector.
    Ring(RingSpec),
    /// Arrow with a head.
    Arrow(ArrowSpec),
    /// Smooth curve through points.
    Spline(SplineSpec),
    /// SVG path data.
    Path(PathSpec),
    /// Paragraph text; requires feature `text`.
    Text(TextSpec),
    /// Raster image file; requires feature `image`.
    Image(ImageSpec),
    /// Static SVG file; requires feature `svg`.
    Svg(SvgSpec),
    /// Particle simulation; see [`ParticlesSpec`].
    Particles(ParticlesSpec),
    /// Analytic particle trajectory; see [`TrajectorySpec`].
    Trajectory(TrajectorySpec),
    /// Axis ruler; see [`RulerSpec`].
    Ruler(RulerSpec),
    /// Callout; see [`CalloutSpec`].
    Callout(CalloutSpec),
    /// Line chart; see [`LineChartSpec`].
    LineChart(LineChartSpec),
    /// Caption track; see [`CaptionsSpec`].
    Captions(CaptionsSpec),
    /// Math formula; see [`FormulaSpec`].
    Formula(FormulaSpec),
    /// Path morph; see [`MorphSpec`].
    Morph(MorphSpec),
    /// SVG morph; see [`SvgMorphSpec`].
    SvgMorph(SvgMorphSpec),
    /// Pressure-sensitive brush; requires feature `ink`.
    BrushStroke(Box<BrushStrokeSpec>),
    /// Deterministic even-odd ink fill; requires feature `ink`.
    InkFill(InkFillSpec),
}

impl Content {
    /// Reports whether fill and stroke paint apply to this content.
    pub fn accepts_paint(&self) -> bool {
        self.is_geometry() || matches!(self, Self::Text(_))
    }

    /// Reports whether stroke trim, dash and fill rule apply to this content.
    pub fn is_geometry(&self) -> bool {
        matches!(
            self,
            Self::Rect(_)
                | Self::Circle(_)
                | Self::Ellipse(_)
                | Self::Line(_)
                | Self::Arc(_)
                | Self::Polygon(_)
                | Self::Polyline(_)
                | Self::RegularPolygon(_)
                | Self::Star(_)
                | Self::Ring(_)
                | Self::Arrow(_)
                | Self::Spline(_)
                | Self::Path(_)
                | Self::Morph(_)
        )
    }
}

/// A numeric text readout with bounded precision and affixes.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct NumberTextSpec {
    /// Numeric animation to display; nonfinite results use Rust's numeric spelling.
    pub value: Animated<f64>,
    /// Decimal places, at most 16.
    pub precision: u8,
    /// Fixed prefix; combined affixes are limited to 4096 bytes.
    #[serde(default)]
    pub prefix: String,
    /// Fixed suffix.
    #[serde(default)]
    pub suffix: String,
}

/// An axis-aligned rectangle.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct RectSpec {
    /// Opposite corners `[left, top, right, bottom]`.
    pub bounds: Animated<Rect>,
    /// Corner radius; square corners by default.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub corner_radius: Option<Animated<f64>>,
}

impl RectSpec {
    /// Creates a square-cornered rectangle.
    pub fn new(bounds: impl Into<Animated<Rect>>) -> Self {
        Self {
            bounds: bounds.into(),
            corner_radius: None,
        }
    }

    /// Rounds every corner.
    pub fn corner_radius(mut self, radius: impl Into<Animated<f64>>) -> Self {
        self.corner_radius = Some(radius.into());
        self
    }
}

/// A circle.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct CircleSpec {
    /// Center `[x, y]`.
    pub center: Animated<Point>,
    /// Nonnegative radius.
    pub radius: Animated<f64>,
}

/// An axis-aligned ellipse.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct EllipseSpec {
    /// Center `[x, y]`.
    pub center: Animated<Point>,
    /// Radii `[horizontal, vertical]`.
    pub radii: Animated<[f64; 2]>,
}

/// A straight segment.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct LineSpec {
    /// Start point `[x, y]`.
    pub from: Animated<Point>,
    /// End point `[x, y]`.
    pub to: Animated<Point>,
}

/// An open circular arc; angles are clockwise degrees from +X.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ArcSpec {
    /// Center `[x, y]`.
    pub center: Animated<Point>,
    /// Radius.
    pub radius: Animated<f64>,
    /// Start angle in degrees.
    pub start_angle: Animated<f64>,
    /// Signed sweep in degrees.
    pub sweep_angle: Animated<f64>,
}

/// Fixed or compile-time sampled points for polygons and polylines.
///
/// Give exactly one of a nonempty `points` list or `sample`.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct PointsSpec {
    /// Points `[[x, y], ...]` in drawing order.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub points: Vec<Point>,
    /// Point animation sampled once during compilation, instead of fixed points.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub sample: Option<SampledPointsSpec>,
    /// Per-frame animated points, instead of fixed or compile-time sampled points.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub animated: Option<Vec<Animated<Point>>>,
}

impl PointsSpec {
    /// Creates a fixed point list.
    pub fn new(points: impl IntoIterator<Item = Point>) -> Self {
        Self {
            points: points.into_iter().collect(),
            sample: None,
            animated: None,
        }
    }

    /// Creates a compile-time sampled point list.
    pub fn sampled(sample: SampledPointsSpec) -> Self {
        Self {
            points: Vec::new(),
            sample: Some(sample),
            animated: None,
        }
    }
}

/// Uniform samples of an animated point, independent of the current frame time.
///
/// The document compiler resolves parameters and animations normally, then retains
/// `segments + 1` points including both range endpoints. No per-frame sampling occurs.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct SampledPointsSpec {
    /// Animated point in drawing coordinates.
    pub source: Animated<Point>,
    /// Increasing, nonnegative sample times in seconds, at most 1e6 each.
    pub range: [f64; 2],
    /// Number of line segments in `1..=65535`.
    pub segments: u32,
}

impl SampledPointsSpec {
    /// Creates a sampling description validated when the document is compiled.
    pub fn new(source: impl Into<Animated<Point>>, range: [f64; 2], segments: u32) -> Self {
        Self {
            source: source.into(),
            range,
            segments,
        }
    }
}

/// A regular polygon with one vertex pointing up.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct RegularPolygonSpec {
    /// Center `[x, y]`.
    pub center: Animated<Point>,
    /// Circumradius.
    pub radius: Animated<f64>,
    /// Side count, at least three.
    pub sides: u32,
}

/// A star with alternating outer and inner vertices.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct StarSpec {
    /// Center `[x, y]`.
    pub center: Animated<Point>,
    /// Number of outer points.
    pub points: u32,
    /// Outer radius.
    pub outer_radius: Animated<f64>,
    /// Inner radius.
    pub inner_radius: Animated<f64>,
}

/// An annulus, or an annular sector when a sweep is given.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct RingSpec {
    /// Center `[x, y]`.
    pub center: Animated<Point>,
    /// Inner radius.
    pub inner_radius: Animated<f64>,
    /// Outer radius.
    pub outer_radius: Animated<f64>,
    /// Sector start in degrees; zero by default.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub start_angle: Option<Animated<f64>>,
    /// Sector sweep in degrees; a full ring by default.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub sweep_angle: Option<Animated<f64>>,
}

/// An arrow outline from a tail to a head.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ArrowSpec {
    /// Tail point `[x, y]`.
    pub from: Animated<Point>,
    /// Head tip `[x, y]`.
    pub to: Animated<Point>,
    /// Shaft width.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub width: Option<Animated<f64>>,
    /// Head geometry; a filled triangle by default.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub head: Option<ArrowHead>,
    /// Head length and width.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub head_size: Option<Animated<f64>>,
}

/// A Catmull-Rom style curve through knots.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct SplineSpec {
    /// Knots `[[x, y], ...]`.
    pub points: Vec<Point>,
    /// Curve tension.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tension: Option<f64>,
}

/// Geometry from SVG path data.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct PathSpec {
    /// SVG `d` attribute syntax, for example `"M0 0 L10 0 Z"`.
    pub data: String,
}

/// A paragraph laid out with the document or render font environment.
///
/// Give `content` for uniform text, `spans` for rich text or `number` for a
/// numeric readout. Fill and
/// stroke come from the element, so they apply to the whole paragraph; spans may
/// override the family, size, weight, slant and fill of their own run.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct TextSpec {
    /// Text content; empty when `spans` is used.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub content: String,
    /// Styled runs of a rich paragraph, at most 256.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub spans: Vec<TextSpanSpec>,
    /// Dynamic numeric readout, instead of `content` or `spans`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub number: Option<NumberTextSpec>,
    /// Font family; the library default when omitted.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub family: Option<String>,
    /// Logical size in `(0, 4096]`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub size: Option<Animated<f64>>,
    /// OpenType weight, for example 400 or 700.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub weight: Option<u16>,
    /// Positive relative line-height multiplier.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub line_height: Option<f64>,
    /// Tracking in design units.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub letter_spacing: Option<f64>,
    /// Wrapping width in design units.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max_width: Option<f64>,
    /// Paragraph alignment.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub alignment: Option<TextAlignment>,
    /// Unit of `visible_parts` and `parts`; characters by default.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub split: Option<TextSplit>,
    /// Number of leading parts drawn, rounded down; a typewriter reveal when animated.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub visible_parts: Option<Animated<f64>>,
    /// Staggered per-part transform and opacity animation.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub parts: Option<PartsSpec>,
    /// Handwriting progress in `0..=1`; fully drawn when omitted.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub write: Option<Animated<f64>>,
}

impl TextSpec {
    /// Creates unstyled text.
    pub fn new(content: impl Into<String>) -> Self {
        Self {
            content: content.into(),
            spans: Vec::new(),
            number: None,
            family: None,
            size: None,
            weight: None,
            line_height: None,
            letter_spacing: None,
            max_width: None,
            alignment: None,
            split: None,
            visible_parts: None,
            parts: None,
            write: None,
        }
    }

    /// Creates a rich paragraph from styled runs.
    pub fn rich(spans: impl IntoIterator<Item = TextSpanSpec>) -> Self {
        Self {
            spans: spans.into_iter().collect(),
            ..Self::new("")
        }
    }

    /// Selects the font family.
    pub fn family(mut self, family: impl Into<String>) -> Self {
        self.family = Some(family.into());
        self
    }

    /// Selects the logical size.
    pub fn size(mut self, size: impl Into<Animated<f64>>) -> Self {
        self.size = Some(size.into());
        self
    }

    /// Selects the OpenType weight.
    pub fn weight(mut self, weight: u16) -> Self {
        self.weight = Some(weight);
        self
    }

    /// Selects the unit of parts.
    pub fn split(mut self, split: TextSplit) -> Self {
        self.split = Some(split);
        self
    }

    /// Draws only the leading parts.
    pub fn visible_parts(mut self, count: impl Into<Animated<f64>>) -> Self {
        self.visible_parts = Some(count.into());
        self
    }

    /// Animates every part with a staggered delay.
    pub fn parts(mut self, parts: PartsSpec) -> Self {
        self.parts = Some(parts);
        self
    }

    /// Animates handwriting progress.
    pub fn write(mut self, progress: impl Into<Animated<f64>>) -> Self {
        self.write = Some(progress.into());
        self
    }
}

/// One styled run of a rich paragraph; unset properties inherit the paragraph's.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct TextSpanSpec {
    /// Run text.
    pub text: String,
    /// Font family.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub family: Option<String>,
    /// Logical size in `(0, 4096]`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub size: Option<f64>,
    /// OpenType weight.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub weight: Option<u16>,
    /// Italic slant.
    #[serde(default, skip_serializing_if = "crate::value::is_false")]
    pub italic: bool,
    /// Fixed run color.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub fill: Option<Color>,
}

impl TextSpanSpec {
    /// Creates an unstyled run.
    pub fn new(text: impl Into<String>) -> Self {
        Self {
            text: text.into(),
            family: None,
            size: None,
            weight: None,
            italic: false,
            fill: None,
        }
    }

    /// Selects the run's size.
    pub fn size(mut self, size: f64) -> Self {
        self.size = Some(size);
        self
    }

    /// Selects the run's weight.
    pub fn weight(mut self, weight: u16) -> Self {
        self.weight = Some(weight);
        self
    }

    /// Slants the run.
    pub fn italic(mut self) -> Self {
        self.italic = true;
        self
    }

    /// Colors the run.
    pub fn fill(mut self, color: impl Into<Color>) -> Self {
        self.fill = Some(color.into());
        self
    }
}

/// Per-part animation. Part `i` is sampled at its own time, the scene time minus
/// `i * delay`, so every animated value (and `t` in expressions) is delayed by part.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct PartsSpec {
    /// Nonnegative per-part stagger in seconds.
    pub delay: f64,
    /// Displacement `[x, y]` of the part.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub offset: Option<Animated<[f64; 2]>>,
    /// Uniform scale about the part's center.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub scale: Option<Animated<f64>>,
    /// Clockwise rotation in degrees.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub rotate: Option<Animated<f64>>,
    /// Opacity in `0..=1`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub opacity: Option<Animated<f64>>,
}

impl PartsSpec {
    /// Creates an identity animation staggered by `delay` seconds per part.
    pub fn new(delay: f64) -> Self {
        Self {
            delay,
            offset: None,
            scale: None,
            rotate: None,
            opacity: None,
        }
    }

    /// Animates the displacement.
    pub fn offset(mut self, offset: impl Into<Animated<[f64; 2]>>) -> Self {
        self.offset = Some(offset.into());
        self
    }

    /// Animates the opacity.
    pub fn opacity(mut self, opacity: impl Into<Animated<f64>>) -> Self {
        self.opacity = Some(opacity.into());
        self
    }
}

/// A raster image file.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ImageSpec {
    /// File path, relative to the document directory.
    pub source: String,
    /// Viewport `[width, height]`; source pixel size by default.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub size: Option<[f64; 2]>,
    /// Placement inside the viewport.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub fit: Option<Fit>,
    /// Corner radius of the viewport.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub corner_radius: Option<f64>,
    /// Reconstruction filter.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub sampling: Option<Sampling>,
}

/// An SVG drawing from a file or embedded markup, optionally written over time.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct SvgSpec {
    /// File path relative to the document directory, or embedded SVG markup.
    pub source: SvgSourceSpec,
    /// Fit the drawing into `[width, height]`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub size: Option<[f64; 2]>,
    /// Handwriting progress in `0..=1`; fully drawn when omitted.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub write: Option<Animated<f64>>,
    /// Handwriting geometry; core defaults when omitted.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub write_options: Option<WriteOptions>,
}

/// An SVG file path or explicitly embedded SVG markup.
#[derive(Clone, Debug, PartialEq, Serialize, JsonSchema)]
#[serde(untagged, deny_unknown_fields)]
pub enum SvgSourceSpec {
    /// File path relative to the document directory.
    File(String),
    /// SVG markup parsed once during document compilation.
    Inline {
        /// SVG XML, limited to 16 MiB.
        data: String,
    },
}

impl From<String> for SvgSourceSpec {
    fn from(source: String) -> Self {
        Self::File(source)
    }
}

impl From<&str> for SvgSourceSpec {
    fn from(source: &str) -> Self {
        Self::File(source.to_owned())
    }
}

impl<'de> Deserialize<'de> for SvgSourceSpec {
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        deserializer.deserialize_any(SvgSourceVisitor)
    }
}

/// Reads an SVG source with a precise message instead of serde's untagged fallback.
struct SvgSourceVisitor;

impl<'de> serde::de::Visitor<'de> for SvgSourceVisitor {
    type Value = SvgSourceSpec;

    fn expecting(&self, formatter: &mut std::fmt::Formatter) -> std::fmt::Result {
        formatter.write_str("an SVG file path string or an object {\"data\": \"<svg ...>\"}")
    }

    fn visit_str<E: serde::de::Error>(self, value: &str) -> Result<Self::Value, E> {
        Ok(SvgSourceSpec::File(value.to_owned()))
    }

    fn visit_string<E: serde::de::Error>(self, value: String) -> Result<Self::Value, E> {
        Ok(SvgSourceSpec::File(value))
    }

    fn visit_map<A: serde::de::MapAccess<'de>>(self, mut map: A) -> Result<Self::Value, A::Error> {
        use serde::de::Error;
        let mut data = None;
        while let Some(key) = map.next_key::<String>()? {
            if key != "data" {
                return Err(A::Error::unknown_field(&key, &["data"]));
            }
            if data.is_some() {
                return Err(A::Error::duplicate_field("data"));
            }
            data = Some(map.next_value::<String>()?);
        }
        data.map(|data| SvgSourceSpec::Inline { data })
            .ok_or_else(|| A::Error::missing_field("data"))
    }
}
