//! Declarative scene elements and their JSON object form.

use crate::captions::CaptionsSpec;
use crate::content::{
    ArcSpec, ArrowSpec, CircleSpec, Content, EllipseSpec, ImageSpec, LineSpec, PathSpec,
    PointsSpec, RectSpec, RegularPolygonSpec, RingSpec, SplineSpec, StarSpec, SvgSpec, TextSpec,
};
use crate::diagram::{CalloutSpec, LineChartSpec, RulerSpec};
use crate::effects::{ClipSpec, FilterSpec, MaskSpec};
use crate::formula::{FormulaSpec, MorphSpec, SvgMorphSpec};
use crate::paint::{PaintSpec, StrokeSpec, TrimSpec};
use crate::particles::{ParticlesSpec, TrajectorySpec};
use crate::value::{Animated, is_false};
use motion_core::{BlendMode, FillRule, MaskMode, Pivot, Point, Rect};
use schemars::{JsonSchema, Schema, SchemaGenerator};
use serde::{Deserialize, Serialize};
use std::borrow::Cow;

/// One node of a declarative scene tree.
///
/// JSON selects the content with exactly one key (`children`, `rect`, `circle`,
/// `text`, ...) next to optional style, transform and effect keys. Angles are
/// clockwise degrees; scalar, point and color properties accept [`Animated`] values.
///
/// ```
/// use motion_scene::{Element, StrokeSpec};
/// use motion_core::Color;
/// let dot = Element::circle([40.0, 40.0], 12.0)
///     .fill(Color::WHITE)
///     .stroke(StrokeSpec::new(2.0, Color::BLACK))
///     .rotate(45.0);
/// let json = serde_json::to_string(&dot)?;
/// assert!(json.starts_with(r#"{"circle":{"center":[40.0,40.0],"radius":12.0}"#));
/// assert_eq!(serde_json::from_str::<Element>(&json)?, dot);
/// # Ok::<(), serde_json::Error>(())
/// ```
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(try_from = "RawElement", into = "RawElement")]
pub struct Element {
    /// Identifier that expressions in the same scene bind to with `@id.property`.
    pub id: Option<String>,
    /// Diagnostic name.
    pub name: Option<String>,
    /// Drawn content.
    pub content: Content,
    /// Fill paint for geometry and text.
    pub fill: Option<PaintSpec>,
    /// Stroke for geometry and text.
    pub stroke: Option<StrokeSpec>,
    /// Geometry fill rule.
    pub fill_rule: Option<FillRule>,
    /// Drawn arc-length range of geometry.
    pub trim: Option<TrimSpec>,
    /// Parent-space point receiving the pivot, `[x, y]`.
    pub position: Option<Animated<Point>>,
    /// Parent-space translation `[x, y]`, after scale and rotation.
    pub translate: Option<Animated<[f64; 2]>>,
    /// Clockwise rotation in degrees about the pivot.
    pub rotate: Option<Animated<f64>>,
    /// Uniform scale about the pivot.
    pub scale: Option<Animated<f64>>,
    /// Per-axis scale `[x, y]` about the pivot, multiplied with `scale`.
    pub scale_xy: Option<Animated<[f64; 2]>>,
    /// Local transform anchor; the bounds center by default.
    pub pivot: Option<Pivot>,
    /// Subtree opacity in `0..=1`.
    pub opacity: Option<Animated<f64>>,
    /// Subtree blend mode.
    pub blend: Option<BlendMode>,
    /// Sibling drawing order; ties keep insertion order.
    pub z_index: Option<i32>,
    /// Removes the element entirely when set.
    pub hidden: bool,
    /// Ordered subtree filters.
    pub filters: Vec<FilterSpec>,
    /// Subtree clip.
    pub clip: Option<ClipSpec>,
    /// Subtree content mask.
    pub mask: Option<Box<MaskSpec>>,
}

impl Element {
    /// Creates an element drawing `content`.
    pub fn new(content: impl Into<Content>) -> Self {
        Self {
            content: content.into(),
            ..Self::default()
        }
    }

    /// Groups child elements.
    pub fn group(children: impl IntoIterator<Item = Element>) -> Self {
        Self::new(Content::Group(children.into_iter().collect()))
    }

    /// Creates a rectangle from `[left, top, right, bottom]`.
    pub fn rect(bounds: impl Into<Animated<Rect>>) -> Self {
        Self::new(RectSpec::new(bounds))
    }

    /// Creates a circle.
    pub fn circle(center: impl Into<Animated<Point>>, radius: impl Into<Animated<f64>>) -> Self {
        Self::new(CircleSpec {
            center: center.into(),
            radius: radius.into(),
        })
    }

    /// Creates an ellipse.
    pub fn ellipse(
        center: impl Into<Animated<Point>>,
        radii: impl Into<Animated<[f64; 2]>>,
    ) -> Self {
        Self::new(EllipseSpec {
            center: center.into(),
            radii: radii.into(),
        })
    }

    /// Creates a straight segment.
    pub fn line(from: impl Into<Animated<Point>>, to: impl Into<Animated<Point>>) -> Self {
        Self::new(LineSpec {
            from: from.into(),
            to: to.into(),
        })
    }

    /// Creates an open polyline from fixed or compile-time sampled points.
    pub fn polyline(points: PointsSpec) -> Self {
        Self::new(Content::Polyline(points))
    }

    /// Creates geometry from SVG path data.
    pub fn path(data: impl Into<String>) -> Self {
        Self::new(PathSpec { data: data.into() })
    }

    /// Creates paragraph text.
    pub fn text(text: TextSpec) -> Self {
        Self::new(text)
    }

    /// Sets the identifier that `@id.property` expressions bind to.
    pub fn id(mut self, id: impl Into<String>) -> Self {
        self.id = Some(id.into());
        self
    }

    /// Sets the diagnostic name.
    pub fn name(mut self, name: impl Into<String>) -> Self {
        self.name = Some(name.into());
        self
    }

    /// Sets the fill paint.
    pub fn fill(mut self, paint: impl Into<PaintSpec>) -> Self {
        self.fill = Some(paint.into());
        self
    }

    /// Sets the stroke.
    pub fn stroke(mut self, stroke: StrokeSpec) -> Self {
        self.stroke = Some(stroke);
        self
    }

    /// Sets the geometry fill rule.
    pub fn fill_rule(mut self, rule: FillRule) -> Self {
        self.fill_rule = Some(rule);
        self
    }

    /// Draws only a normalized arc-length range.
    pub fn trim(mut self, trim: TrimSpec) -> Self {
        self.trim = Some(trim);
        self
    }

    /// Places the pivot at a parent-space point.
    pub fn position(mut self, position: impl Into<Animated<Point>>) -> Self {
        self.position = Some(position.into());
        self
    }

    /// Translates after scale and rotation.
    pub fn translate(mut self, offset: impl Into<Animated<[f64; 2]>>) -> Self {
        self.translate = Some(offset.into());
        self
    }

    /// Rotates clockwise by degrees.
    pub fn rotate(mut self, degrees: impl Into<Animated<f64>>) -> Self {
        self.rotate = Some(degrees.into());
        self
    }

    /// Scales uniformly.
    pub fn scale(mut self, factor: impl Into<Animated<f64>>) -> Self {
        self.scale = Some(factor.into());
        self
    }

    /// Scales each axis.
    pub fn scale_xy(mut self, factors: impl Into<Animated<[f64; 2]>>) -> Self {
        self.scale_xy = Some(factors.into());
        self
    }

    /// Selects the transform anchor.
    pub fn pivot(mut self, pivot: Pivot) -> Self {
        self.pivot = Some(pivot);
        self
    }

    /// Sets subtree opacity.
    pub fn opacity(mut self, opacity: impl Into<Animated<f64>>) -> Self {
        self.opacity = Some(opacity.into());
        self
    }

    /// Sets the subtree blend mode.
    pub fn blend(mut self, mode: BlendMode) -> Self {
        self.blend = Some(mode);
        self
    }

    /// Sets the sibling drawing order.
    pub fn z_index(mut self, index: i32) -> Self {
        self.z_index = Some(index);
        self
    }

    /// Removes the element from output.
    pub fn hidden(mut self) -> Self {
        self.hidden = true;
        self
    }

    /// Appends a filter.
    pub fn filter(mut self, filter: FilterSpec) -> Self {
        self.filters.push(filter);
        self
    }

    /// Sets the subtree clip.
    pub fn clip(mut self, clip: ClipSpec) -> Self {
        self.clip = Some(clip);
        self
    }

    /// Masks the subtree with another element.
    pub fn mask(mut self, element: Element, mode: MaskMode) -> Self {
        self.mask = Some(Box::new(MaskSpec {
            element,
            mode: Some(mode),
        }));
        self
    }
}

impl JsonSchema for Element {
    fn schema_name() -> Cow<'static, str> {
        "Element".into()
    }

    fn json_schema(generator: &mut SchemaGenerator) -> Schema {
        let mut schema = RawElement::json_schema(generator);
        schema.insert(
            "description".into(),
            "A scene node with at most one content key (children, rect, circle, ...).".into(),
        );
        schema
    }
}

macro_rules! content_conversions {
    ($($spec:ident => $variant:ident),+ $(,)?) => {
        $(
            impl From<$spec> for Content {
                fn from(spec: $spec) -> Self {
                    Self::$variant(spec)
                }
            }
        )+
    };
}

content_conversions!(
    RectSpec => Rect,
    CircleSpec => Circle,
    EllipseSpec => Ellipse,
    LineSpec => Line,
    ArcSpec => Arc,
    RegularPolygonSpec => RegularPolygon,
    StarSpec => Star,
    RingSpec => Ring,
    ArrowSpec => Arrow,
    SplineSpec => Spline,
    PathSpec => Path,
    TextSpec => Text,
    ImageSpec => Image,
    SvgSpec => Svg,
    ParticlesSpec => Particles,
    TrajectorySpec => Trajectory,
    RulerSpec => Ruler,
    CalloutSpec => Callout,
    LineChartSpec => LineChart,
    CaptionsSpec => Captions,
    FormulaSpec => Formula,
    MorphSpec => Morph,
    SvgMorphSpec => SvgMorph,
);

/// The JSON object form: one optional key per content kind.
#[derive(Default, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
struct RawElement {
    /// Identifier for `@id.property` bindings.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    id: Option<String>,
    /// Diagnostic name.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    name: Option<String>,
    /// Child elements.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    children: Option<Vec<Element>>,
    /// Rectangle content.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    rect: Option<RectSpec>,
    /// Circle content.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    circle: Option<CircleSpec>,
    /// Ellipse content.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    ellipse: Option<EllipseSpec>,
    /// Line content.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    line: Option<LineSpec>,
    /// Arc content.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    arc: Option<ArcSpec>,
    /// Closed polygon content.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    polygon: Option<PointsSpec>,
    /// Open polyline content.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    polyline: Option<PointsSpec>,
    /// Regular polygon content.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    regular_polygon: Option<RegularPolygonSpec>,
    /// Star content.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    star: Option<StarSpec>,
    /// Ring content.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    ring: Option<RingSpec>,
    /// Arrow content.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    arrow: Option<ArrowSpec>,
    /// Spline content.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    spline: Option<SplineSpec>,
    /// SVG path content.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    path: Option<PathSpec>,
    /// Text content.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    text: Option<TextSpec>,
    /// Image content.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    image: Option<ImageSpec>,
    /// SVG content.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    svg: Option<SvgSpec>,
    /// Particle simulation content.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    particles: Option<ParticlesSpec>,
    /// Analytic particle trajectory content.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    trajectory: Option<TrajectorySpec>,
    /// Axis ruler content.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    ruler: Option<RulerSpec>,
    /// Callout content.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    callout: Option<CalloutSpec>,
    /// Line chart content.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    line_chart: Option<LineChartSpec>,
    /// Caption track content.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    captions: Option<CaptionsSpec>,
    /// Math formula content.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    formula: Option<FormulaSpec>,
    /// Path morph content.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    morph: Option<MorphSpec>,
    /// SVG morph content.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    svg_morph: Option<SvgMorphSpec>,
    /// Fill paint for geometry and text.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    fill: Option<PaintSpec>,
    /// Stroke for geometry and text.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    stroke: Option<StrokeSpec>,
    /// Geometry fill rule.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    fill_rule: Option<FillRule>,
    /// Drawn arc-length range of geometry.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    trim: Option<TrimSpec>,
    /// Parent-space point receiving the pivot.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    position: Option<Animated<Point>>,
    /// Parent-space translation.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    translate: Option<Animated<[f64; 2]>>,
    /// Clockwise rotation in degrees.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    rotate: Option<Animated<f64>>,
    /// Uniform scale.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    scale: Option<Animated<f64>>,
    /// Per-axis scale.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    scale_xy: Option<Animated<[f64; 2]>>,
    /// Transform anchor.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pivot: Option<Pivot>,
    /// Subtree opacity.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    opacity: Option<Animated<f64>>,
    /// Subtree blend mode.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    blend: Option<BlendMode>,
    /// Sibling drawing order.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    z_index: Option<i32>,
    /// Removes the element when true.
    #[serde(default, skip_serializing_if = "is_false")]
    hidden: bool,
    /// Ordered subtree filters.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    filters: Vec<FilterSpec>,
    /// Subtree clip.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    clip: Option<ClipSpec>,
    /// Subtree content mask.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    mask: Option<Box<MaskSpec>>,
}

impl TryFrom<RawElement> for Element {
    type Error = String;

    fn try_from(raw: RawElement) -> Result<Self, String> {
        let candidates = [
            raw.children
                .map(|value| ("children", Content::Group(value))),
            raw.rect.map(|value| ("rect", Content::Rect(value))),
            raw.circle.map(|value| ("circle", Content::Circle(value))),
            raw.ellipse
                .map(|value| ("ellipse", Content::Ellipse(value))),
            raw.line.map(|value| ("line", Content::Line(value))),
            raw.arc.map(|value| ("arc", Content::Arc(value))),
            raw.polygon
                .map(|value| ("polygon", Content::Polygon(value))),
            raw.polyline
                .map(|value| ("polyline", Content::Polyline(value))),
            raw.regular_polygon
                .map(|value| ("regular_polygon", Content::RegularPolygon(value))),
            raw.star.map(|value| ("star", Content::Star(value))),
            raw.ring.map(|value| ("ring", Content::Ring(value))),
            raw.arrow.map(|value| ("arrow", Content::Arrow(value))),
            raw.spline.map(|value| ("spline", Content::Spline(value))),
            raw.path.map(|value| ("path", Content::Path(value))),
            raw.text.map(|value| ("text", Content::Text(value))),
            raw.image.map(|value| ("image", Content::Image(value))),
            raw.svg.map(|value| ("svg", Content::Svg(value))),
            raw.particles
                .map(|value| ("particles", Content::Particles(value))),
            raw.trajectory
                .map(|value| ("trajectory", Content::Trajectory(value))),
            raw.ruler.map(|value| ("ruler", Content::Ruler(value))),
            raw.callout
                .map(|value| ("callout", Content::Callout(value))),
            raw.line_chart
                .map(|value| ("line_chart", Content::LineChart(value))),
            raw.captions
                .map(|value| ("captions", Content::Captions(value))),
            raw.formula
                .map(|value| ("formula", Content::Formula(value))),
            raw.morph.map(|value| ("morph", Content::Morph(value))),
            raw.svg_morph
                .map(|value| ("svg_morph", Content::SvgMorph(value))),
        ];
        let mut selected = candidates.into_iter().flatten();
        let content = match (selected.next(), selected.next()) {
            (None, _) => Content::Empty,
            (Some((_, content)), None) => content,
            (Some((first, _)), Some((second, _))) => {
                return Err(format!(
                    "Element has both `{first}` and `{second}`; choose one content key"
                ));
            }
        };
        Ok(Self {
            id: raw.id,
            name: raw.name,
            content,
            fill: raw.fill,
            stroke: raw.stroke,
            fill_rule: raw.fill_rule,
            trim: raw.trim,
            position: raw.position,
            translate: raw.translate,
            rotate: raw.rotate,
            scale: raw.scale,
            scale_xy: raw.scale_xy,
            pivot: raw.pivot,
            opacity: raw.opacity,
            blend: raw.blend,
            z_index: raw.z_index,
            hidden: raw.hidden,
            filters: raw.filters,
            clip: raw.clip,
            mask: raw.mask,
        })
    }
}

impl From<Element> for RawElement {
    fn from(element: Element) -> Self {
        let mut raw = Self {
            id: element.id,
            name: element.name,
            fill: element.fill,
            stroke: element.stroke,
            fill_rule: element.fill_rule,
            trim: element.trim,
            position: element.position,
            translate: element.translate,
            rotate: element.rotate,
            scale: element.scale,
            scale_xy: element.scale_xy,
            pivot: element.pivot,
            opacity: element.opacity,
            blend: element.blend,
            z_index: element.z_index,
            hidden: element.hidden,
            filters: element.filters,
            clip: element.clip,
            mask: element.mask,
            ..Self::default()
        };
        match element.content {
            Content::Empty => {}
            Content::Group(children) => raw.children = Some(children),
            Content::Rect(spec) => raw.rect = Some(spec),
            Content::Circle(spec) => raw.circle = Some(spec),
            Content::Ellipse(spec) => raw.ellipse = Some(spec),
            Content::Line(spec) => raw.line = Some(spec),
            Content::Arc(spec) => raw.arc = Some(spec),
            Content::Polygon(spec) => raw.polygon = Some(spec),
            Content::Polyline(spec) => raw.polyline = Some(spec),
            Content::RegularPolygon(spec) => raw.regular_polygon = Some(spec),
            Content::Star(spec) => raw.star = Some(spec),
            Content::Ring(spec) => raw.ring = Some(spec),
            Content::Arrow(spec) => raw.arrow = Some(spec),
            Content::Spline(spec) => raw.spline = Some(spec),
            Content::Path(spec) => raw.path = Some(spec),
            Content::Text(spec) => raw.text = Some(spec),
            Content::Image(spec) => raw.image = Some(spec),
            Content::Svg(spec) => raw.svg = Some(spec),
            Content::Particles(spec) => raw.particles = Some(spec),
            Content::Trajectory(spec) => raw.trajectory = Some(spec),
            Content::Ruler(spec) => raw.ruler = Some(spec),
            Content::Callout(spec) => raw.callout = Some(spec),
            Content::LineChart(spec) => raw.line_chart = Some(spec),
            Content::Captions(spec) => raw.captions = Some(spec),
            Content::Formula(spec) => raw.formula = Some(spec),
            Content::Morph(spec) => raw.morph = Some(spec),
            Content::SvgMorph(spec) => raw.svg_morph = Some(spec),
        }
        raw
    }
}
