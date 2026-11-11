//! Diagram annotations and tick axes.

mod chart;
mod scale;

pub use chart::{Label, LabelLayout, LineChart};
pub use scale::Scale;

use crate::Text;

use motion_core::{
    Camera, Color, LineCap, LineJoin, Node, NodeModifiers, Path2D, PathVerb, Point, Rect,
    SceneContext, Shape, Size, StrokeStyle, ValidationError,
};

fn invalid(message: &str) -> ValidationError {
    ValidationError::invalid("diagram", "input", message)
}

fn valid(bounds: Rect) -> bool {
    bounds.is_finite()
        && bounds.width().is_finite()
        && bounds.height().is_finite()
        && !bounds.is_empty()
}

/// A text label and leader, with explicitly authored label dimensions.
#[derive(Clone, Debug)]
pub struct Callout {
    text: String,
    target: Point,
    bounds: Rect,
    color: Color,
    font_size: f64,
    leader_width: f64,
}

impl Callout {
    /// Captures a target and opposite-corner label bounds; validation is deferred.
    pub fn new(text: impl Into<String>, target: Point, bounds: Rect) -> Self {
        Self {
            text: text.into(),
            target,
            bounds,
            color: Color::from_hex(0x23272c),
            font_size: 14.0,
            leader_width: 1.5,
        }
    }

    /// Replaces the text and leader color.
    pub fn color(mut self, color: Color) -> Self {
        self.color = color;
        self
    }

    /// Replaces logical font size, valid in (0,4096].
    pub fn font_size(mut self, size: f64) -> Self {
        self.font_size = size;
        self
    }

    /// Replaces the positive logical leader width.
    pub fn leader_width(mut self, width: f64) -> Self {
        self.leader_width = width;
        self
    }

    fn attachment(&self) -> Result<Point, ValidationError> {
        if !valid(self.bounds)
            || ![
                self.target.x,
                self.target.y,
                self.font_size,
                self.leader_width,
            ]
            .iter()
            .all(|value| value.is_finite())
            || self.font_size <= 0.0
            || self.font_size > 4096.0
            || self.leader_width <= 0.0
        {
            return Err(invalid(
                "Callout requires finite geometry, positive font/leader sizes and nonempty label bounds",
            ));
        }
        Ok(Point::new(
            self.target.x.clamp(self.bounds.left, self.bounds.right),
            self.target.y.clamp(self.bounds.top, self.bounds.bottom),
        ))
    }

    /// Builds ordinary text and path nodes without discovering fonts.
    pub fn node(self) -> motion_core::Result<Node> {
        let attachment = self.attachment()?;
        let mut nodes = Vec::new();
        if attachment != self.target {
            let mut path = Path2D::new();
            path.try_move_to(self.target)?;
            path.try_line_to(attachment)?;
            nodes.push(Node::from(path.stroke(self.leader_width, self.color)));
        }
        nodes.push(
            Node::from(
                Text::new(self.text)
                    .size(self.font_size)
                    .max_width(self.bounds.width())
                    .fill(self.color),
            )
            .translate(self.bounds.left, self.bounds.top)
            .clip_with(self.bounds, motion_core::ClipMode::Geometric),
        );
        Ok(Node::group(nodes))
    }

    /// Projects the world target while keeping label dimensions in viewport space.
    /// Outside targets hide the whole callout, but invalid hidden settings still fail.
    pub fn node_in(
        mut self,
        context: &SceneContext<'_>,
        camera: Camera,
    ) -> motion_core::Result<Node> {
        self.target = camera.world_to_screen(context, self.target)?;
        self.attachment()?;
        if !context.bounds().contains(self.target) {
            return Ok(Node::empty());
        }
        Ok(self
            .node()?
            .clip_with(context.bounds(), motion_core::ClipMode::Geometric))
    }
}

/// A horizontal/vertical axis with fixed-size, nonoverlapping labels.
#[derive(Clone, Debug)]
pub struct Ruler {
    scale: Scale,
    baseline: f64,
    vertical: bool,
    tick_count: usize,
    nice: bool,
    tick_length: f64,
    line_width: f64,
    font_size: f64,
    label_size: Size,
    color: Color,
    unit: String,
}

impl Ruler {
    fn new(scale: Scale, baseline: f64, vertical: bool) -> Self {
        Self {
            scale,
            baseline,
            vertical,
            tick_count: 5,
            nice: false,
            tick_length: 6.0,
            line_width: 1.0,
            font_size: 12.0,
            label_size: Size::new(96.0, 24.0),
            color: Color::from_hex(0x23272c),
            unit: String::new(),
        }
    }

    /// Creates an X-axis at the supplied Y coordinate.
    pub fn horizontal(scale: Scale, baseline: f64) -> Self {
        Self::new(scale, baseline, false)
    }

    /// Creates a Y-axis at the supplied X coordinate.
    pub fn vertical(scale: Scale, baseline: f64) -> Self {
        Self::new(scale, baseline, true)
    }

    /// Selects uniform ticks including endpoints; valid count is 2..=256.
    pub fn ticks(mut self, count: usize) -> Self {
        self.tick_count = count;
        self.nice = false;
        self
    }

    /// Selects at most the supplied number of aligned ticks; valid budget is 2..=256.
    pub fn nice_ticks(mut self, count: usize) -> Self {
        self.tick_count = count;
        self.nice = true;
        self
    }

    /// Replaces finite nonnegative tick length.
    pub fn tick_length(mut self, length: f64) -> Self {
        self.tick_length = length;
        self
    }

    /// Replaces positive finite axis/mark width.
    pub fn line_width(mut self, width: f64) -> Self {
        self.line_width = width;
        self
    }

    /// Replaces label font size in (0,4096].
    pub fn font_size(mut self, size: f64) -> Self {
        self.font_size = size;
        self
    }

    /// Replaces explicitly authored positive label dimensions.
    pub fn label_size(mut self, size: Size) -> Self {
        self.label_size = size;
        self
    }

    /// Replaces axis and label color.
    pub fn color(mut self, color: Color) -> Self {
        self.color = color;
        self
    }

    /// Replaces the formatting suffix, without changing physical units.
    pub fn unit(mut self, unit: impl Into<String>) -> Self {
        self.unit = unit.into();
        self
    }

    /// Builds nodes with two-decimal default numeric labels.
    pub fn node(&self) -> motion_core::Result<Node> {
        self.node_with_labels(|value| {
            let value = if value == 0.0 { 0.0 } else { value };
            let number = if value != 0.0 && !(0.001..100000.0).contains(&value.abs()) {
                format!("{value:.2e}")
            } else {
                format!("{value:.2}")
            };
            if self.unit.is_empty() {
                number
            } else {
                format!("{number} {}", self.unit)
            }
        })
    }

    fn recipe(
        &self,
        mut formatter: impl FnMut(f64) -> String,
    ) -> motion_core::Result<(Path2D, Vec<(String, Rect)>)> {
        if ![
            self.baseline,
            self.tick_length,
            self.line_width,
            self.font_size,
            self.label_size.width,
            self.label_size.height,
        ]
        .iter()
        .all(|value| value.is_finite())
            || self.tick_length < 0.0
            || self.line_width <= 0.0
            || self.font_size <= 0.0
            || self.font_size > 4096.0
            || self.label_size.width <= 0.0
            || self.label_size.height <= 0.0
        {
            return Err(invalid("Invalid ruler geometry or label dimensions").into());
        }
        let ticks = if self.nice {
            self.scale.nice_ticks(self.tick_count)?
        } else {
            self.scale.ticks(self.tick_count)?
        };
        let point = |coordinate| {
            if self.vertical {
                Point::new(self.baseline, coordinate)
            } else {
                Point::new(coordinate, self.baseline)
            }
        };
        let range = self.scale.range();
        let mut outline = Path2D::new();
        let mut append = |first: Point, second: Point| -> motion_core::Result {
            let mut path = Path2D::new();
            path.try_move_to(first)?;
            path.try_line_to(second)?;
            let style = StrokeStyle {
                width: self.line_width,
                cap: LineCap::Round,
                join: LineJoin::Round,
            };
            for verb in style.outline(&path, "ruler.stroke")?.verbs() {
                match *verb {
                    PathVerb::MoveTo(point) => outline.try_move_to(point)?,
                    PathVerb::LineTo(point) => outline.try_line_to(point)?,
                    PathVerb::QuadraticTo(control, end) => {
                        outline.try_quadratic_to(control, end)?
                    }
                    PathVerb::CubicTo(first, second, end) => {
                        outline.try_cubic_to(first, second, end)?
                    }
                    PathVerb::Close => outline.try_close()?,
                }
            }
            Ok(())
        };
        append(point(range.0), point(range.1))?;
        let mut labels: Vec<(String, Rect)> = Vec::new();
        for value in ticks {
            let coordinate = self.scale.map(value)?;
            let first = point(coordinate);
            let end = if self.vertical {
                Point::new(first.x + self.tick_length, first.y)
            } else {
                Point::new(first.x, first.y + self.tick_length)
            };
            append(first, end)?;
            let origin = if self.vertical {
                Point::new(end.x + 6.0, coordinate - self.label_size.height * 0.5)
            } else {
                Point::new(coordinate - self.label_size.width * 0.5, end.y + 6.0)
            };
            let bounds = Rect::from_origin_size(origin, self.label_size);
            if !valid(bounds) {
                return Err(invalid("Ruler label bounds are not representable").into());
            }
            if labels
                .last()
                .is_some_and(|(_, previous)| !previous.intersect(bounds).is_empty())
            {
                return Err(invalid(
                    "Ruler label boxes overlap; reduce ticks or adjust label size",
                )
                .into());
            }
            labels.push((formatter(value), bounds));
        }
        Ok((outline, labels))
    }

    /// Eagerly formats once per tick and returns text plus one union fill.
    /// The common fill prevents axis/mark intersections from multiplying alpha.
    pub fn node_with_labels(
        &self,
        formatter: impl FnMut(f64) -> String,
    ) -> motion_core::Result<Node> {
        let (outline, labels) = self.recipe(formatter)?;
        let mut nodes = vec![Node::from(outline.fill(self.color))];
        nodes.extend(labels.into_iter().map(|(text, bounds)| {
            Node::from(
                Text::new(text)
                    .size(self.font_size)
                    .max_width(bounds.width())
                    .fill(self.color),
            )
            .translate(bounds.left, bounds.top)
            .clip_with(bounds, motion_core::ClipMode::Geometric)
        }));
        Ok(Node::group(nodes))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rulers_preserve_axis_ticks_formatting_and_fixed_label_geometry() -> motion_core::Result {
        let scale = Scale::linear((0.0, 100.0), (100.0, 500.0))?;
        let ruler = Ruler::horizontal(scale, 30.0).ticks(3).unit("AU");
        let mut visited = Vec::new();
        let (outline, labels) = ruler.recipe(|value| {
            visited.push(value);
            format!("{value:.2} AU")
        })?;
        assert_eq!(visited, [0.0, 50.0, 100.0]);
        assert_eq!(
            labels[0],
            ("0.00 AU".into(), Rect::new(52.0, 42.0, 148.0, 66.0))
        );
        assert_eq!(labels.len(), 3);
        assert!(outline.bounding_box().contains(Point::new(100.0, 30.0)));
        assert!(outline.bounding_box().contains(Point::new(500.0, 30.0)));
        let vertical = Ruler::vertical(Scale::log10((1.0, 10000.0), (40.0, 440.0))?, 20.0)
            .ticks(3)
            .label_size(Size::new(80.0, 20.0));
        let mut values = Vec::new();
        let (_, labels) = vertical.recipe(|value| {
            values.push(value);
            format!("{value:.0} km")
        })?;
        assert_eq!(values, [1.0, 100.0, 10000.0]);
        assert_eq!(
            labels[0],
            ("1 km".into(), Rect::new(32.0, 30.0, 112.0, 50.0))
        );
        Ok(())
    }

    #[test]
    fn nice_rulers_keep_selection_replacement_empty_axis_and_density_errors() -> motion_core::Result
    {
        let base = Ruler::horizontal(Scale::linear((3.0, 97.0), (100.0, 860.0))?, 30.0);
        let mut values = Vec::new();
        base.clone().ticks(1).nice_ticks(6).recipe(|value| {
            values.push(value);
            value.to_string()
        })?;
        assert_eq!(values, [20.0, 40.0, 60.0, 80.0]);
        let mut values = Vec::new();
        base.nice_ticks(1).ticks(3).recipe(|value| {
            values.push(value);
            value.to_string()
        })?;
        assert_eq!(values, [3.0, 50.0, 97.0]);
        let (axis, labels) = Ruler::vertical(Scale::linear((3.1, 3.9), (0.0, 100.0))?, 0.0)
            .nice_ticks(2)
            .recipe(|_| panic!("no aligned ticks"))?;
        assert!(labels.is_empty());
        assert!(!axis.bounding_box().is_empty());
        let scale = Scale::linear((0.0, 1.0), (0.0, 100.0))?;
        for count in [1, 5, 257] {
            assert!(
                Ruler::horizontal(scale, 0.0)
                    .nice_ticks(count)
                    .node()
                    .is_err()
            );
        }
        let base = Ruler::horizontal(scale, 0.0)
            .ticks(2)
            .label_size(Size::new(24.0, 18.0));
        assert!(base.clone().node().is_ok());
        assert!(base.clone().tick_length(0.0).node().is_ok());
        for invalid in [
            base.clone().line_width(0.0),
            base.clone().font_size(f64::NAN),
            base.clone().font_size(4097.0),
            base.clone().tick_length(-1.0),
            base.label_size(Size::default()),
        ] {
            assert!(invalid.node().is_err());
        }
        Ok(())
    }

    #[test]
    fn callouts_keep_nearest_attachment_projection_and_hidden_validation() -> motion_core::Result {
        let bounds = Rect::new(40.0, 20.0, 100.0, 50.0);
        assert_eq!(
            Callout::new("Target", Point::new(10.0, 30.0), bounds).attachment()?,
            Point::new(40.0, 30.0)
        );
        assert_eq!(
            Callout::new("Target", Point::new(50.0, 30.0), bounds).attachment()?,
            Point::new(50.0, 30.0)
        );
        let cache = motion_core::SceneCache::new(0);
        let context = SceneContext::new(
            motion_core::Time::ZERO,
            motion_core::fps!(30),
            Size::new(320.0, 180.0),
            &cache,
        );
        let view = Camera {
            zoom: 2.0,
            ..Camera::default()
        };
        assert_eq!(
            view.world_to_screen(&context, Point::new(10.0, 0.0))?,
            Point::new(180.0, 90.0)
        );
        let hidden =
            Callout::new("Target", Point::new(200.0, 0.0), bounds).node_in(&context, view)?;
        assert!(
            hidden
                .compile(
                    context.size(),
                    motion_core::PixelSize::new(320, 180),
                    motion_core::ValidationLimits::default()
                )?
                .commands
                .is_empty()
        );
        assert!(
            Callout::new("Target", Point::new(200.0, 0.0), bounds)
                .font_size(0.0)
                .node_in(&context, view)
                .is_err()
        );
        Ok(())
    }
}
