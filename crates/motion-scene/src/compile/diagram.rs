//! Compilation of rulers, callouts and line charts.

use super::Compiler;
use super::element::Built;
use crate::diagram::{CalloutSpec, LineChartSpec, RulerSpec};

#[cfg(feature = "diagram")]
use super::failure::failure;
#[cfg(feature = "diagram")]
use super::style::point;
#[cfg(feature = "diagram")]
use crate::diagram::{OrientationSpec, ScaleKind, ScaleSpec};
#[cfg(feature = "diagram")]
use motion_content::diagram::{Callout, LineChart, Ruler, Scale};
#[cfg(feature = "diagram")]
use motion_core::{LineCap, LineJoin, Node, Rect, Size, StrokeStyle};

/// Most series a chart may hold, matching the engine limit.
#[cfg(feature = "diagram")]
const MAX_SERIES: usize = 32;

impl Compiler<'_> {
    /// Compiles a ruler into a static node; its labels are text drawn at render time.
    #[cfg(feature = "diagram")]
    pub(crate) fn ruler(&mut self, spec: &RulerSpec, path: &str) -> Option<Built> {
        let scale = self.scale(&spec.scale, spec.scale.range, &format!("{path}.scale"))?;
        let mut ruler = match spec.orientation {
            OrientationSpec::Horizontal => Ruler::horizontal(scale, spec.baseline),
            OrientationSpec::Vertical => Ruler::vertical(scale, spec.baseline),
        };
        if let Some(count) = spec.ticks {
            let count = count as usize;
            ruler = if spec.nice {
                ruler.nice_ticks(count)
            } else {
                ruler.ticks(count)
            };
        }
        if let Some(length) = spec.tick_length {
            ruler = ruler.tick_length(length);
        }
        if let Some(width) = spec.line_width {
            ruler = ruler.line_width(width);
        }
        if let Some(size) = spec.font_size {
            ruler = ruler.font_size(size);
        }
        if let Some([width, height]) = spec.label_size {
            ruler = ruler.label_size(Size::new(width, height));
        }
        if let Some(color) = &spec.color {
            ruler = ruler.color(color.color());
        }
        if let Some(unit) = &spec.unit {
            ruler = ruler.unit(unit.as_str());
        }
        self.fixed(ruler.node(), path)
    }

    /// Compiles a callout into a static node.
    #[cfg(feature = "diagram")]
    pub(crate) fn callout(&mut self, spec: &CalloutSpec, path: &str) -> Option<Built> {
        let [left, top, right, bottom] = spec.bounds;
        let mut callout = Callout::new(
            spec.text.as_str(),
            point(spec.target),
            Rect::new(left, top, right, bottom),
        );
        if let Some(color) = &spec.color {
            callout = callout.color(color.color());
        }
        if let Some(size) = spec.font_size {
            callout = callout.font_size(size);
        }
        if let Some(width) = spec.leader_width {
            callout = callout.leader_width(width);
        }
        self.fixed(callout.node(), path)
    }

    /// Compiles a line chart; it is laid out per frame against the render fonts.
    #[cfg(feature = "diagram")]
    pub(crate) fn line_chart(&mut self, spec: &LineChartSpec, path: &str) -> Option<Built> {
        let [left, top, right, bottom] = spec.bounds;
        let horizontal = self.scale(
            &spec.horizontal,
            Some([left, right]),
            &format!("{path}.horizontal"),
        );
        let vertical = self.scale(
            &spec.vertical,
            Some([bottom, top]),
            &format!("{path}.vertical"),
        );
        if spec.series.len() > MAX_SERIES {
            self.limit(path, "series", "A chart holds at most 32 series");
            return None;
        }
        let (horizontal, vertical) = (horizontal?, vertical?);
        let mut chart = LineChart::new(Rect::new(left, top, right, bottom), horizontal, vertical)
            .map_err(|error| self.error(path, "bounds", error.message))
            .ok()?;
        if let Some(count) = spec.ticks {
            chart = chart
                .ticks(count as usize)
                .map_err(|error| self.error(path, "ticks", error.message))
                .ok()?;
        }
        for (index, series) in spec.series.iter().enumerate() {
            let here = format!("{path}.series[{index}]");
            let stroke = StrokeStyle {
                width: series.width.unwrap_or(2.0),
                cap: series.cap.unwrap_or(LineCap::Round),
                join: match series.join.unwrap_or(crate::options::LineJoinSpec::Round) {
                    crate::options::LineJoinSpec::Miter => LineJoin::Miter { limit: 4.0 },
                    crate::options::LineJoinSpec::Round => LineJoin::Round,
                    crate::options::LineJoinSpec::Bevel => LineJoin::Bevel,
                },
            };
            let points = series.points.iter().map(|entry| entry.map(|[x, y]| (x, y)));
            chart = chart
                .gapped_series(points, series.color.color(), stroke)
                .map_err(|error| self.error(&here, "points", error.message))
                .ok()?;
        }
        Some(Built::with_context(true, move |context| {
            chart.node(context).unwrap_or_else(|error| {
                failure(
                    error
                        .downcast_ref::<motion_core::ValidationError>()
                        .cloned()
                        .unwrap_or_else(|| {
                            motion_core::ValidationError::invalid(
                                "line_chart",
                                "chart",
                                error.to_string(),
                            )
                        }),
                )
            })
        }))
    }

    #[cfg(feature = "diagram")]
    fn scale(&mut self, spec: &ScaleSpec, range: Option<[f64; 2]>, path: &str) -> Option<Scale> {
        let Some([start, end]) = range else {
            self.error(path, "range", "A ruler scale requires a `range`");
            return None;
        };
        let domain = (spec.domain[0], spec.domain[1]);
        let scale = match spec.kind {
            ScaleKind::Linear => Scale::linear(domain, (start, end)),
            ScaleKind::Log10 => Scale::log10(domain, (start, end)),
        };
        scale
            .map(|scale| scale.clamp(spec.clamp))
            .map_err(|error| self.error(path, "domain", error.message))
            .ok()
    }

    /// Turns an eagerly built node into static content or a located diagnostic.
    #[cfg(feature = "diagram")]
    fn fixed(&mut self, node: motion_core::Result<Node>, path: &str) -> Option<Built> {
        match node {
            Ok(node) => Some(Built::Fixed(node)),
            Err(error) => {
                self.error(path, "diagram", error.to_string());
                None
            }
        }
    }

    #[cfg(not(feature = "diagram"))]
    pub(crate) fn ruler(&mut self, _: &RulerSpec, path: &str) -> Option<Built> {
        self.unsupported(path, "Diagrams require feature `diagram`");
        None
    }

    #[cfg(not(feature = "diagram"))]
    pub(crate) fn callout(&mut self, _: &CalloutSpec, path: &str) -> Option<Built> {
        self.unsupported(path, "Diagrams require feature `diagram`");
        None
    }

    #[cfg(not(feature = "diagram"))]
    pub(crate) fn line_chart(&mut self, _: &LineChartSpec, path: &str) -> Option<Built> {
        self.unsupported(path, "Diagrams require feature `diagram`");
        None
    }
}
