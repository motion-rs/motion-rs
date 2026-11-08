//! Compiled fill, stroke, trim and gradient styling shared by geometry and text.

use super::Compiler;
use super::value::{Lift, Prop};
use crate::element::Element;
use crate::options::LineJoinSpec;
use crate::paint::{GradientOptions, GradientSpec, PaintSpec, StrokeSpec};
use motion_core::{
    Color, FillRule, LineCap, LineJoin, LinearGradient, Node, Paint, Point, RadialGradient, Shape,
    StrokeStyle, SweepGradient, Time,
};
use std::sync::Arc;

/// Default stroke miter limit, matching [`StrokeStyle::default`].
const DEFAULT_MITER_LIMIT: f64 = 4.0;

/// Largest accepted miter limit.
const MAX_MITER_LIMIT: f64 = 1024.0;

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

/// Paint that is fixed or carries an animated solid color.
pub(crate) enum PaintProp {
    Solid(Prop<Color>),
    Fixed(Paint),
}

impl PaintProp {
    fn at(&self, time: Time) -> Paint {
        match self {
            Self::Solid(color) => Paint::Solid(color.at(time)),
            Self::Fixed(paint) => paint.clone(),
        }
    }

    fn varying(&self) -> bool {
        matches!(self, Self::Solid(color) if color.varying())
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
    pub(crate) fn shape<Geometry: Shape + 'static>(&self, geometry: Geometry, time: Time) -> Node {
        let mut styled = match (&self.fill, &self.stroke) {
            (Some(fill), _) => geometry.fill(fill.at(time)),
            (None, Some(stroke)) => geometry.stroke(stroke.width.at(time), stroke.paint.at(time)),
            (None, None) => geometry.fill(Color::TRANSPARENT),
        };
        if let Some(stroke) = &self.stroke {
            let width = stroke.width.at(time);
            if self.fill.is_some() {
                styled = styled.stroke(width, stroke.paint.at(time));
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
        Node::from(styled)
    }

    /// Applies fill and stroke paint to text.
    #[cfg(feature = "text")]
    pub(crate) fn text(&self, text: motion_content::Text, time: Time) -> motion_content::Text {
        let mut text = text;
        if let Some(fill) = &self.fill {
            text = text.fill(fill.at(time));
        }
        if let Some(stroke) = &self.stroke {
            text = text.stroke(stroke.width.at(time), stroke.paint.at(time));
        }
        text
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
            rule: element.fill_rule.map(Into::into),
            trim: trim.lift()?,
        }))
    }

    fn stroke(&mut self, stroke: &StrokeSpec, path: &str) -> Option<StrokeProp> {
        let width = self.animated(&stroke.width, &format!("{path}.width"));
        let paint = self.paint(&stroke.paint, &format!("{path}.paint"));
        let limit = self.bounded(
            stroke.miter_limit.unwrap_or(DEFAULT_MITER_LIMIT),
            1.0,
            MAX_MITER_LIMIT,
            path,
            "miter_limit",
        );
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
        let join = match stroke.join.unwrap_or_default() {
            LineJoinSpec::Miter => LineJoin::Miter { limit: limit? },
            LineJoinSpec::Round => LineJoin::Round,
            LineJoinSpec::Bevel => LineJoin::Bevel,
        };
        Some(StrokeProp {
            width: width?,
            paint: paint?,
            cap: stroke.cap.unwrap_or_default().into(),
            join,
            dash: dash?,
        })
    }

    pub(crate) fn paint(&mut self, paint: &PaintSpec, path: &str) -> Option<PaintProp> {
        match paint {
            PaintSpec::Solid(color) => self.animated(color, path).map(PaintProp::Solid),
            PaintSpec::Gradient(gradient) => self.gradient(gradient, path).map(PaintProp::Fixed),
        }
    }

    fn gradient(&mut self, gradient: &GradientSpec, path: &str) -> Option<Paint> {
        let paint = match gradient {
            GradientSpec::Linear(spec) => {
                let mut gradient = LinearGradient::new(point(spec.start), point(spec.end));
                for stop in &spec.stops {
                    gradient = gradient.stop(stop.offset, stop.color.color());
                }
                Paint::Linear(with_options!(gradient, spec.options))
            }
            GradientSpec::Radial(spec) => {
                let mut gradient = RadialGradient::new(point(spec.center), spec.radius);
                if let Some(focal) = spec.focal {
                    gradient = gradient.focal(point(focal));
                }
                for stop in &spec.stops {
                    gradient = gradient.stop(stop.offset, stop.color.color());
                }
                Paint::Radial(with_options!(gradient, spec.options))
            }
            GradientSpec::Sweep(spec) => {
                let mut gradient = SweepGradient::new(
                    point(spec.center),
                    spec.start_angle.to_radians(),
                    spec.end_angle.to_radians(),
                );
                for stop in &spec.stops {
                    gradient = gradient.stop(stop.offset, stop.color.color());
                }
                Paint::Sweep(with_options!(gradient, spec.options))
            }
        };
        match paint.validate() {
            Ok(()) => Some(paint),
            Err(error) => {
                self.error(path, "gradient", error.message);
                None
            }
        }
    }
}

/// Converts a document point.
pub(crate) fn point([x, y]: [f64; 2]) -> Point {
    Point::new(x, y)
}
