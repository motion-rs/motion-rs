//! Compiled fill, stroke, trim and gradient styling shared by geometry and text.

use super::Compiler;
use super::value::{Lift, Prop};
use crate::element::Element;
use crate::paint::{GradientOptions, GradientSpec, PaintSpec, StrokeSpec};
use motion_core::{
    Color, FillRule, LineCap, LineJoin, LinearGradient, Node, Paint, RadialGradient, Shape,
    StrokeStyle, SweepGradient, Time, ValidationError,
};
use std::sync::Arc;

/// Maximum full-turn rounding excess in radians after converting document degrees.
const FULL_TURN_ROUNDING_TOLERANCE: f64 = 4.0 * f64::EPSILON * std::f64::consts::TAU;

/// Applies optional space, units and extension to any gradient builder.
macro_rules! with_options {
    ($gradient:expr, $options:expr) => {{
        let options: GradientOptions = $options;
        let mut gradient = $gradient;
        if let Some(space) = options.space {
            gradient = gradient.space(space.into());
        }
        if let Some(units) = options.units {
            gradient = gradient.units(units.into());
        }
        if let Some(extend) = options.extend {
            gradient = gradient.extend(extend.into());
        }
        gradient
    }};
}

/// A gradient sampled at scene-local time before validation.
type GradientSampler = Arc<dyn Fn(Time) -> Paint + Send + Sync>;

/// Paint that is fixed, carries an animated solid color or samples an animated gradient.
pub(crate) enum PaintProp {
    Solid(Prop<Color>),
    Fixed(Paint),
    /// An animated gradient, validated again at every sampled time.
    Sampled(Arc<dyn Fn(Time) -> Result<Paint, ValidationError> + Send + Sync>),
}

impl PaintProp {
    pub(super) fn at(&self, time: Time) -> Result<Paint, ValidationError> {
        match self {
            Self::Solid(color) => Ok(Paint::Solid(color.at(time))),
            Self::Fixed(paint) => Ok(paint.clone()),
            Self::Sampled(sample) => sample(time),
        }
    }

    pub(super) fn varying(&self) -> bool {
        match self {
            Self::Solid(color) => color.varying(),
            Self::Fixed(_) => false,
            Self::Sampled(_) => true,
        }
    }
}

/// Compiled element styling.
pub(crate) struct Style {
    fill: Option<PaintProp>,
    stroke: Option<StrokeProp>,
    rule: Option<FillRule>,
    trim: Option<[Prop<f64>; 2]>,
}

impl Style {
    pub(crate) fn varying(&self) -> bool {
        self.fill.as_ref().is_some_and(PaintProp::varying)
            || self.stroke.as_ref().is_some_and(StrokeProp::varying)
            || self
                .trim
                .as_ref()
                .is_some_and(|[start, end]| start.varying() || end.varying())
    }

    /// Paints geometry; compilation guarantees at least one of fill or stroke.
    /// Invalid geometry or sampled paint becomes a node that reports its error when prepared.
    pub(crate) fn shape<Geometry: Shape + 'static>(
        &self,
        geometry: Result<Geometry, ValidationError>,
        time: Time,
    ) -> Node {
        self.try_shape(geometry, time)
            .unwrap_or_else(super::failure::failure)
    }

    fn try_shape<Geometry: Shape + 'static>(
        &self,
        geometry: Result<Geometry, ValidationError>,
        time: Time,
    ) -> Result<Node, ValidationError> {
        let geometry = geometry?;
        let mut styled = match (&self.fill, &self.stroke) {
            (Some(fill), _) => geometry.fill(fill.at(time)?),
            (None, Some(stroke)) => geometry.stroke(stroke.width.at(time), stroke.paint.at(time)?),
            (None, None) => geometry.fill(Color::TRANSPARENT),
        };
        if let Some(stroke) = &self.stroke {
            let width = stroke.width.at(time);
            if self.fill.is_some() {
                styled = styled.stroke(width, stroke.paint.at(time)?);
            }
            styled = styled.stroke_style(StrokeStyle {
                width,
                cap: stroke.cap,
                join: stroke.join,
            });
            if let Some((pattern, offset)) = &stroke.dash {
                styled = styled.dash(pattern, offset.at(time));
            }
        }
        if let Some([start, end]) = &self.trim {
            styled = styled.trim(start.at(time), end.at(time));
        }
        if let Some(rule) = self.rule {
            styled = styled.fill_rule(rule);
        }
        Ok(Node::from(styled))
    }

    /// Applies fill and stroke paint to text, failing when sampled paint is invalid.
    #[cfg(feature = "text")]
    pub(crate) fn text(
        &self,
        text: motion_content::Text,
        time: Time,
    ) -> Result<motion_content::Text, ValidationError> {
        let mut text = text;
        if let Some(fill) = &self.fill {
            text = text.fill(fill.at(time)?);
        }
        if let Some(stroke) = &self.stroke {
            text = text.stroke(stroke.width.at(time), stroke.paint.at(time)?);
        }
        Ok(text)
    }
}

struct StrokeProp {
    width: Prop<f64>,
    paint: PaintProp,
    cap: LineCap,
    join: LineJoin,
    dash: Option<(Arc<[f64]>, Prop<f64>)>,
}

impl StrokeProp {
    fn varying(&self) -> bool {
        self.width.varying()
            || self.paint.varying()
            || self
                .dash
                .as_ref()
                .is_some_and(|(_, offset)| offset.varying())
    }
}

impl Compiler<'_> {
    /// Compiles styling after checking it applies to the element content.
    pub(crate) fn style(&mut self, element: &Element, path: &str) -> Option<Arc<Style>> {
        let content = &element.content;
        let painted = element.fill.is_some() || element.stroke.is_some();
        let shaped = element.trim.is_some()
            || element.fill_rule.is_some()
            || element
                .stroke
                .as_ref()
                .is_some_and(|stroke| !stroke.dash.is_empty() || stroke.dash_offset.is_some());
        let mut valid = true;
        if painted && !content.accepts_paint() {
            self.error(
                path,
                "fill",
                "Fill and stroke apply only to geometry and text",
            );
            valid = false;
        }
        if shaped && !content.is_geometry() {
            self.error(
                path,
                "trim",
                "Trim, dash and fill rule apply only to geometry",
            );
            valid = false;
        }
        if content.is_geometry() && !painted {
            self.error(path, "fill", "Geometry requires a fill or a stroke");
            valid = false;
        }
        let fill = element
            .fill
            .as_ref()
            .map(|fill| self.paint(fill, &format!("{path}.fill")));
        let stroke = element
            .stroke
            .as_ref()
            .map(|stroke| self.stroke(stroke, &format!("{path}.stroke")));
        let trim = element.trim.as_ref().map(|trim| {
            let start = self.animated(&trim.start, &format!("{path}.trim.start"));
            let end = self.animated(&trim.end, &format!("{path}.trim.end"));
            Some([start?, end?])
        });
        if !valid {
            return None;
        }
        Some(Arc::new(Style {
            fill: fill.lift()?,
            stroke: stroke.lift()?,
            rule: element.fill_rule,
            trim: trim.lift()?,
        }))
    }

    fn stroke(&mut self, stroke: &StrokeSpec, path: &str) -> Option<StrokeProp> {
        let width = self.animated(&stroke.width, &format!("{path}.width"));
        let paint = self.paint(&stroke.paint, &format!("{path}.paint"));
        let join = stroke.join.unwrap_or(StrokeStyle::default().join);
        let cap = stroke.cap.unwrap_or_default();
        let checked = StrokeStyle {
            width: 1.0,
            cap,
            join,
        }
        .validate(path)
        .map_err(|error| self.error(path, "join", error.message))
        .is_ok();
        let dash = if stroke.dash.is_empty() {
            Some(None)
        } else {
            let valid = stroke
                .dash
                .iter()
                .all(|length| length.is_finite() && *length >= 0.0);
            if !valid {
                self.error(path, "dash", "Dash lengths must be finite and nonnegative");
            }
            let offset = self.animated_or(
                stroke.dash_offset.as_ref(),
                0.0,
                &format!("{path}.dash_offset"),
            );
            valid
                .then_some(())
                .and(offset)
                .map(|offset| Some((Arc::from(stroke.dash.as_slice()), offset)))
        };
        if !checked {
            return None;
        }
        Some(StrokeProp {
            width: width?,
            paint: paint?,
            cap,
            join,
            dash: dash?,
        })
    }

    pub(crate) fn paint(&mut self, paint: &PaintSpec, path: &str) -> Option<PaintProp> {
        match paint {
            PaintSpec::Solid(color) => self.animated(color, path).map(PaintProp::Solid),
            PaintSpec::Gradient(gradient) => self.gradient(gradient, path),
            PaintSpec::Pigment(pigment) => self.pigment(pigment, &format!("{path}.pigment")),
        }
    }

    fn gradient(&mut self, gradient: &GradientSpec, path: &str) -> Option<PaintProp> {
        let (varying, sample): (bool, GradientSampler) = match gradient {
            GradientSpec::Linear(spec) => {
                let start = self.animated(&spec.start, &format!("{path}.linear.start"));
                let end = self.animated(&spec.end, &format!("{path}.linear.end"));
                let (start, end) = (start?, end?);
                let spec = spec.clone();
                (
                    start.varying() || end.varying(),
                    Arc::new(move |time| {
                        let mut gradient = LinearGradient::new(start.at(time), end.at(time));
                        for stop in &spec.stops {
                            gradient = gradient.stop(stop.offset, stop.color);
                        }
                        Paint::Linear(with_options!(gradient, spec.options))
                    }),
                )
            }
            GradientSpec::Radial(spec) => {
                let center = self.animated(&spec.center, &format!("{path}.radial.center"));
                let radius = self.animated(&spec.radius, &format!("{path}.radial.radius"));
                let focal = spec
                    .focal
                    .as_ref()
                    .map(|focal| self.animated(focal, &format!("{path}.radial.focal")));
                let (center, radius, focal) = (center?, radius?, focal.lift()?);
                let spec = spec.clone();
                (
                    center.varying()
                        || radius.varying()
                        || focal.as_ref().is_some_and(Prop::varying),
                    Arc::new(move |time| {
                        let mut gradient = RadialGradient::new(center.at(time), radius.at(time));
                        if let Some(focal) = &focal {
                            gradient = gradient.focal(focal.at(time));
                        }
                        for stop in &spec.stops {
                            gradient = gradient.stop(stop.offset, stop.color);
                        }
                        Paint::Radial(with_options!(gradient, spec.options))
                    }),
                )
            }
            GradientSpec::Sweep(spec) => {
                let center = self.animated(&spec.center, &format!("{path}.sweep.center"));
                let start = self.animated(&spec.start_angle, &format!("{path}.sweep.start_angle"));
                let end = self.animated(&spec.end_angle, &format!("{path}.sweep.end_angle"));
                let (center, start, end) = (center?, start?, end?);
                let spec = spec.clone();
                (
                    center.varying() || start.varying() || end.varying(),
                    Arc::new(move |time| {
                        let start = start.at(time).to_radians();
                        let mut end = end.at(time).to_radians();
                        let span = end - start;
                        if span > std::f64::consts::TAU
                            && span - std::f64::consts::TAU <= FULL_TURN_ROUNDING_TOLERANCE
                        {
                            end = start + std::f64::consts::TAU;
                            if end - start > std::f64::consts::TAU {
                                end = end.next_down();
                            }
                        }
                        let mut gradient = SweepGradient::new(center.at(time), start, end);
                        for stop in &spec.stops {
                            gradient = gradient.stop(stop.offset, stop.color);
                        }
                        Paint::Sweep(with_options!(gradient, spec.options))
                    }),
                )
            }
        };
        let initial = sample(Time::ZERO);
        if let Err(error) = initial.validate() {
            self.error(path, "gradient", error.message);
            return None;
        }
        if !varying {
            return Some(PaintProp::Fixed(initial));
        }
        let path = path.to_owned();
        Some(PaintProp::Sampled(Arc::new(move |time| {
            let paint = sample(time);
            paint
                .validate()
                .map_err(|error| ValidationError::invalid(&path, "gradient", error.message))?;
            Ok(paint)
        })))
    }
}
