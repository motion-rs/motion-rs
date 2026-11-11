//! Compilation of element content into styled geometry, text, images and SVG.

use super::Compiler;
use super::element::Built;
use super::style::{Style, point};
use super::value::Lift;
use crate::content::{Content, ImageSpec, SvgSpec};
use motion_core::{
    ArcShape, Arrow, Circle, Ellipse, Line, Node, Polygon, Polyline, Rect, RegularPolygon, Ring,
    RoundedRect, Spline, Star, Time, Vector2,
};
use std::sync::Arc;

/// Largest accepted polygon, polyline or spline point count.
const MAX_POINTS: usize = 65_536;

/// Largest accepted regular-polygon side or star point count.
const MAX_VERTICES: u32 = 4096;

impl Compiler<'_> {
    /// Compiles content; `style` is None when styling already failed.
    pub(crate) fn content(
        &mut self,
        content: &Content,
        style: Option<Arc<Style>>,
        path: &str,
        depth: usize,
    ) -> Option<Built> {
        match content {
            Content::Empty => style.map(|_| Built::Fixed(Node::empty())),
            Content::Group(children) => {
                let children = children
                    .iter()
                    .enumerate()
                    .map(|(index, child)| {
                        self.element(child, &format!("{path}.children[{index}]"), depth + 1)
                    })
                    .collect::<Vec<_>>();
                let children = children.into_iter().collect::<Option<Vec<_>>>()?;
                style?;
                let varying = children.iter().any(Built::varying);
                Some(Built::with_context(varying, move |context| {
                    Node::group(
                        children
                            .iter()
                            .map(|child| child.at(context))
                            .collect::<Vec<_>>(),
                    )
                }))
            }
            Content::Rect(spec) => {
                let path = format!("{path}.rect");
                let bounds = self.animated(&spec.bounds, &format!("{path}.bounds"));
                let radius = spec
                    .corner_radius
                    .as_ref()
                    .map(|radius| self.animated(radius, &format!("{path}.corner_radius")));
                let (bounds, radius) = (bounds?, radius.lift()?);
                let varying = bounds.varying() || radius.as_ref().is_some_and(|r| r.varying());
                shape(style?, varying, move |style, time| {
                    let [left, top, right, bottom] = bounds.at(time);
                    let rect = Rect::new(left, top, right, bottom);
                    match &radius {
                        Some(radius) => style.shape(RoundedRect::new(rect, radius.at(time)), time),
                        None => style.shape(Ok(rect), time),
                    }
                })
            }
            Content::Circle(spec) => {
                let path = format!("{path}.circle");
                let center = self.animated(&spec.center, &format!("{path}.center"));
                let radius = self.animated(&spec.radius, &format!("{path}.radius"));
                let (center, radius) = (center?, radius?);
                let varying = center.varying() || radius.varying();
                shape(style?, varying, move |style, time| {
                    style.shape(Circle::new(point(center.at(time)), radius.at(time)), time)
                })
            }
            Content::Ellipse(spec) => {
                let path = format!("{path}.ellipse");
                let center = self.animated(&spec.center, &format!("{path}.center"));
                let radii = self.animated(&spec.radii, &format!("{path}.radii"));
                let (center, radii) = (center?, radii?);
                let varying = center.varying() || radii.varying();
                shape(style?, varying, move |style, time| {
                    let [horizontal, vertical] = radii.at(time);
                    let ellipse =
                        Ellipse::new(point(center.at(time)), Vector2::new(horizontal, vertical));
                    style.shape(ellipse, time)
                })
            }
            Content::Line(spec) => {
                let path = format!("{path}.line");
                let from = self.animated(&spec.from, &format!("{path}.from"));
                let to = self.animated(&spec.to, &format!("{path}.to"));
                let (from, to) = (from?, to?);
                let varying = from.varying() || to.varying();
                shape(style?, varying, move |style, time| {
                    style.shape(Line::new(point(from.at(time)), point(to.at(time))), time)
                })
            }
            Content::Arc(spec) => {
                let path = format!("{path}.arc");
                let center = self.animated(&spec.center, &format!("{path}.center"));
                let radius = self.animated(&spec.radius, &format!("{path}.radius"));
                let start = self.animated(&spec.start_angle, &format!("{path}.start_angle"));
                let sweep = self.animated(&spec.sweep_angle, &format!("{path}.sweep_angle"));
                let (center, radius, start, sweep) = (center?, radius?, start?, sweep?);
                let varying =
                    center.varying() || radius.varying() || start.varying() || sweep.varying();
                shape(style?, varying, move |style, time| {
                    let arc = ArcShape::new(
                        point(center.at(time)),
                        radius.at(time),
                        start.at(time).to_radians(),
                        sweep.at(time).to_radians(),
                    );
                    style.shape(arc, time)
                })
            }
            Content::Polygon(spec) => {
                let points = self.points(&spec.points, &format!("{path}.polygon"))?;
                let polygon = Polygon::new(points)
                    .map_err(|error| {
                        self.error(&format!("{path}.polygon"), "points", error.message)
                    })
                    .ok()?;
                shape(style?, false, move |style, time| {
                    style.shape(Ok(polygon.clone()), time)
                })
            }
            Content::Polyline(spec) => {
                let points = self.points(&spec.points, &format!("{path}.polyline"))?;
                let polyline = Polyline::new(points)
                    .map_err(|error| {
                        self.error(&format!("{path}.polyline"), "points", error.message)
                    })
                    .ok()?;
                shape(style?, false, move |style, time| {
                    style.shape(Ok(polyline.clone()), time)
                })
            }
            Content::RegularPolygon(spec) => {
                let path = format!("{path}.regular_polygon");
                let center = self.animated(&spec.center, &format!("{path}.center"));
                let radius = self.animated(&spec.radius, &format!("{path}.radius"));
                let sides = self.count(spec.sides, 3, &path, "sides");
                let (center, radius, sides) = (center?, radius?, sides?);
                let varying = center.varying() || radius.varying();
                shape(style?, varying, move |style, time| {
                    let polygon =
                        RegularPolygon::new(point(center.at(time)), radius.at(time), sides);
                    style.shape(polygon, time)
                })
            }
            Content::Star(spec) => {
                let path = format!("{path}.star");
                let center = self.animated(&spec.center, &format!("{path}.center"));
                let outer = self.animated(&spec.outer_radius, &format!("{path}.outer_radius"));
                let inner = self.animated(&spec.inner_radius, &format!("{path}.inner_radius"));
                let count = self.count(spec.points, 2, &path, "points");
                let (center, outer, inner, count) = (center?, outer?, inner?, count?);
                let varying = center.varying() || outer.varying() || inner.varying();
                shape(style?, varying, move |style, time| {
                    let star = Star::new(
                        point(center.at(time)),
                        count,
                        outer.at(time),
                        inner.at(time),
                    );
                    style.shape(star, time)
                })
            }
            Content::Ring(spec) => {
                let path = format!("{path}.ring");
                let center = self.animated(&spec.center, &format!("{path}.center"));
                let inner = self.animated(&spec.inner_radius, &format!("{path}.inner_radius"));
                let outer = self.animated(&spec.outer_radius, &format!("{path}.outer_radius"));
                let sector = spec.start_angle.is_some() || spec.sweep_angle.is_some();
                let start = self.animated_or(
                    spec.start_angle.as_ref(),
                    0.0,
                    &format!("{path}.start_angle"),
                );
                let sweep = self.animated_or(
                    spec.sweep_angle.as_ref(),
                    360.0,
                    &format!("{path}.sweep_angle"),
                );
                let (center, inner, outer, start, sweep) =
                    (center?, inner?, outer?, start?, sweep?);
                let varying = center.varying()
                    || inner.varying()
                    || outer.varying()
                    || start.varying()
                    || sweep.varying();
                shape(style?, varying, move |style, time| {
                    let mut ring =
                        Ring::new(point(center.at(time)), inner.at(time), outer.at(time));
                    if sector {
                        ring = ring.sweep(start.at(time).to_radians(), sweep.at(time).to_radians());
                    }
                    style.shape(Ok(ring), time)
                })
            }
            Content::Arrow(spec) => {
                let path = format!("{path}.arrow");
                let from = self.animated(&spec.from, &format!("{path}.from"));
                let to = self.animated(&spec.to, &format!("{path}.to"));
                let width = spec
                    .width
                    .as_ref()
                    .map(|width| self.animated(width, &format!("{path}.width")));
                let size = spec
                    .head_size
                    .as_ref()
                    .map(|size| self.animated(size, &format!("{path}.head_size")));
                let (from, to, width, size) = (from?, to?, width.lift()?, size.lift()?);
                let head = spec.head;
                let varying = from.varying()
                    || to.varying()
                    || width.as_ref().is_some_and(|width| width.varying())
                    || size.as_ref().is_some_and(|size| size.varying());
                shape(style?, varying, move |style, time| {
                    let mut arrow = Arrow::new(point(from.at(time)), point(to.at(time)));
                    if let Some(head) = head {
                        arrow = arrow.head(head);
                    }
                    if let Some(width) = &width {
                        arrow = arrow.width(width.at(time));
                    }
                    if let Some(size) = &size {
                        arrow = arrow.head_size(size.at(time));
                    }
                    style.shape(Ok(arrow), time)
                })
            }
            Content::Spline(spec) => {
                let path = format!("{path}.spline");
                let points = self.points(&spec.points, &path)?;
                let mut spline = Spline::through(points)
                    .map_err(|error| self.error(&path, "points", error.message))
                    .ok()?;
                if let Some(tension) = spec.tension {
                    spline = spline.tension(tension);
                }
                shape(style?, false, move |style, time| {
                    style.shape(Ok(spline.clone()), time)
                })
            }
            Content::Path(spec) => {
                let geometry = self.svg_path(&spec.data, &format!("{path}.path"))?;
                shape(style?, false, move |style, time| {
                    style.shape(Ok(geometry.clone()), time)
                })
            }
            Content::Text(spec) => self.text(spec, style, &format!("{path}.text")),
            Content::Image(spec) => style.and_then(|_| self.image(spec, &format!("{path}.image"))),
            Content::Svg(spec) => style.and_then(|_| self.svg(spec, &format!("{path}.svg"))),
            Content::Particles(spec) => self.particles(spec, style, &format!("{path}.particles")),
            Content::Trajectory(spec) => {
                self.trajectory(spec, style, &format!("{path}.trajectory"))
            }
            Content::Ruler(spec) => style.and_then(|_| self.ruler(spec, &format!("{path}.ruler"))),
            Content::Callout(spec) => {
                style.and_then(|_| self.callout(spec, &format!("{path}.callout")))
            }
            Content::LineChart(spec) => {
                style.and_then(|_| self.line_chart(spec, &format!("{path}.line_chart")))
            }
            Content::Captions(spec) => {
                style.and_then(|_| self.captions(spec, &format!("{path}.captions")))
            }
            Content::Formula(spec) => {
                style.and_then(|_| self.formula(spec, &format!("{path}.formula")))
            }
            Content::Morph(spec) => self.morph(spec, style, &format!("{path}.morph")),
            Content::SvgMorph(spec) => {
                style.and_then(|_| self.svg_morph(spec, &format!("{path}.svg_morph")))
            }
        }
    }

    fn points(&mut self, points: &[[f64; 2]], path: &str) -> Option<Vec<motion_core::Point>> {
        if points.len() > MAX_POINTS {
            self.limit(path, "points", "Point lists are limited to 65536 entries");
            return None;
        }
        if points.iter().flatten().any(|value| !value.is_finite()) {
            self.error(path, "points", "Every coordinate must be finite");
            return None;
        }
        Some(points.iter().copied().map(point).collect())
    }

    fn count(&mut self, count: u32, minimum: u32, path: &str, field: &str) -> Option<usize> {
        if (minimum..=MAX_VERTICES).contains(&count) {
            Some(count as usize)
        } else {
            self.error(
                path,
                field,
                format!("Count must be in {minimum}..={MAX_VERTICES}"),
            );
            None
        }
    }

    #[cfg(feature = "vector")]
    fn image(&mut self, spec: &ImageSpec, path: &str) -> Option<Built> {
        use motion_content::{Fit, Image};
        let source = self.resolve(&spec.source, &format!("{path}.source"))?;
        let mut image = Image::open(motion_core::Source::File(source));
        if let Some([width, height]) = spec.size {
            image = image.size(width, height);
        }
        if let Some(fit) = spec.fit {
            image = image.fit(match fit {
                crate::options::FitSpec::Fill => Fit::Fill,
                crate::options::FitSpec::Contain => Fit::Contain,
                crate::options::FitSpec::Cover => Fit::Cover,
                crate::options::FitSpec::None => Fit::None,
            });
        }
        if let Some(radius) = spec.corner_radius {
            image = image.corner_radius(radius);
        }
        if let Some(sampling) = spec.sampling {
            image = image.sampling(sampling);
        }
        Some(Built::Fixed(Node::from(image)))
    }

    #[cfg(not(feature = "vector"))]
    fn image(&mut self, _: &ImageSpec, path: &str) -> Option<Built> {
        self.unsupported(path, "Images require feature `image`");
        None
    }

    #[cfg(feature = "svg")]
    fn svg(&mut self, spec: &SvgSpec, path: &str) -> Option<Built> {
        let write = spec
            .write
            .as_ref()
            .map(|write| self.animated(write, &format!("{path}.write")));
        let source = self.resolve(&spec.source, &format!("{path}.source"));
        let (write, source) = (write.lift()?, source?);
        let mut svg = motion_content::Svg::open(motion_core::Source::File(source));
        if let Some([width, height]) = spec.size {
            svg = svg.fit(motion_core::Size::new(width, height));
        }
        let varying = write.as_ref().is_some_and(super::value::Prop::varying);
        Some(Built::new(varying, move |time| {
            let mut svg = svg.clone();
            if let Some(write) = &write {
                svg = svg.write(write.at(time));
            }
            Node::from(svg)
        }))
    }

    #[cfg(not(feature = "svg"))]
    fn svg(&mut self, _: &SvgSpec, path: &str) -> Option<Built> {
        self.unsupported(path, "SVG requires feature `svg`");
        None
    }
}

/// Builds styled geometry, evaluated once when neither geometry nor style varies.
fn shape(
    style: Arc<Style>,
    varying: bool,
    build: impl Fn(&Style, Time) -> Node + Send + Sync + 'static,
) -> Option<Built> {
    let varying = varying || style.varying();
    Some(Built::new(varying, move |time| build(&style, time)))
}
