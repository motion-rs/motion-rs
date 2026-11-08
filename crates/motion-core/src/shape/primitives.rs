//! Built-in geometric primitives.
//!
//! Invalid constructors retain a checked error. Use try_path to inspect it;
//! drawing validates invalid bounds before Shape::path could panic.
//!
//! ```
//! use motion_core::prelude::*;
//! use motion_core::{
//!     DisplayList, RasterLimits, ResourcePool, ValidatedDisplayList, MAX_PATH_VERBS,
//!     MAX_STROKE_VERBS, MAX_SVG_PATH_BYTES, sample_offsets,
//! };
//! let curve = Spline::through([
//!     Point::new(1.0, 6.0),
//!     Point::new(4.0, 1.0),
//!     Point::new(7.0, 6.0),
//! ])
//! .stroke(0.5, Color::WHITE)
//! .trim(0.0, 0.8)
//! .dash(&[1.0, 0.5], -0.2);
//! Node::from(curve).compile(
//!     Size::new(8.0, 8.0),
//!     PixelSize::new(8, 8),
//!     ValidationLimits::default(),
//! )?;
//! # Ok::<(), motion_core::Error>(())
//! ```

use crate::{MAX_PATH_VERBS, Path2D, Point, Rect, Shape, ValidationError, Vector2};
use std::{ops::Range, sync::Arc as Shared};

fn invalid(message: &str) -> ValidationError {
    ValidationError::invalid("shape", "geometry", message)
}

fn radius(value: f64) -> Result<(), ValidationError> {
    if value.is_finite() && value >= 0.0 {
        Ok(())
    } else {
        Err(invalid(
            "Radii and dimensions must be finite and nonnegative",
        ))
    }
}

fn points(values: impl IntoIterator<Item = Point>) -> Result<Shared<[Point]>, ValidationError> {
    let mut result = Vec::new();
    for value in values {
        if result.len() >= MAX_PATH_VERBS - 1 {
            return Err(ValidationError::limit(
                "shape",
                "points",
                "Shape point allowance exceeded",
            ));
        }
        if !value.x.is_finite() || !value.y.is_finite() {
            return Err(invalid("Shape points must be finite"));
        }
        result.push(value);
    }
    Ok(result.into())
}

fn polyline(values: &[Point], closed: bool) -> Result<Path2D, ValidationError> {
    let mut path = Path2D::new();
    if let Some(first) = values.first() {
        path.try_move_to(*first)?;
        for value in &values[1..] {
            path.try_line_to(*value)?;
        }
        if closed {
            path.try_close()?;
        }
    }
    Ok(path)
}

macro_rules! shape_impl {
    ($name:ident) => {
        impl Shape for $name {
            fn path(&self) -> Path2D {
                self.try_path()
                    .expect("Shape::path requires valid geometry")
            }
            fn bounding_box(&self) -> Rect {
                self.try_path()
                    .map(|path| path.bounding_box())
                    .unwrap_or(Rect::new(f64::NAN, 0.0, 0.0, 0.0))
            }
        }
    };
}

macro_rules! stored_shape {
    ($name:ident, $description:literal) => {
        #[doc = $description]
        #[derive(Clone, Debug)]
        pub struct $name {
            path: Result<Path2D, ValidationError>,
        }
        impl $name {
            /// Returns checked geometry without resource work or narrowing.
            pub fn try_path(&self) -> Result<Path2D, ValidationError> {
                self.path.clone()
            }
        }
        shape_impl!($name);
    };
}

stored_shape!(
    Circle,
    "A circular contour, using bounded cubic arc approximation."
);

impl Circle {
    /// Builds a clockwise closed contour; zero radius is degenerate geometry.
    pub fn new(center: Point, radius: f64) -> Self {
        Self {
            path: Ellipse::new(center, Vector2::new(radius, radius)).try_path(),
        }
    }
}

stored_shape!(Ellipse, "An axis-aligned elliptical contour.");

impl Ellipse {
    /// Builds a clockwise closed ellipse from nonnegative horizontal/vertical radii.
    pub fn new(center: Point, radii: Vector2) -> Self {
        Self {
            path: (|| {
                let mut path = Path2D::new();
                path.try_arc_to(center, radii, 0.0, std::f64::consts::TAU)?;
                path.try_close()?;
                Ok(path)
            })(),
        }
    }
}

stored_shape!(
    Arc,
    "An open circular arc; fill closes its chord, stroke keeps open ends."
);

impl Arc {
    /// Builds a clockwise-positive arc in radians, connecting no implicit center.
    pub fn new(center: Point, radius: f64, start_angle: f64, sweep_angle: f64) -> Self {
        Self {
            path: (|| {
                let mut path = Path2D::new();
                path.try_arc_to(
                    center,
                    Vector2::new(radius, radius),
                    start_angle,
                    sweep_angle,
                )?;
                Ok(path)
            })(),
        }
    }
}

stored_shape!(
    RoundedRect,
    "A rectangle with uniformly rounded, clamped corner radii."
);

impl RoundedRect {
    /// Clamps the radius to half the smaller valid rectangle dimension.
    pub fn new(rect: Rect, corner_radius: f64) -> Self {
        Self {
            path: (|| {
                crate::drawing::validate_rect(rect, "shape", "bounds")?;
                radius(corner_radius)?;
                let corner_radius = corner_radius
                    .min(rect.width() * 0.5)
                    .min(rect.height() * 0.5);
                if corner_radius == 0.0 {
                    return Path2D::from_rect(rect);
                }
                let mut path = Path2D::new();
                path.try_move_to(Point::new(rect.left + corner_radius, rect.top))?;
                let radii = Vector2::new(corner_radius, corner_radius);
                for (center, start) in [
                    (
                        Point::new(rect.right - corner_radius, rect.top + corner_radius),
                        -std::f64::consts::FRAC_PI_2,
                    ),
                    (
                        Point::new(rect.right - corner_radius, rect.bottom - corner_radius),
                        0.0,
                    ),
                    (
                        Point::new(rect.left + corner_radius, rect.bottom - corner_radius),
                        std::f64::consts::FRAC_PI_2,
                    ),
                    (
                        Point::new(rect.left + corner_radius, rect.top + corner_radius),
                        std::f64::consts::PI,
                    ),
                ] {
                    path.try_arc_to(center, radii, start, std::f64::consts::FRAC_PI_2)?;
                }
                path.try_close()?;
                Ok(path)
            })(),
        }
    }
}

stored_shape!(Line, "An open line segment with f64 endpoints.");

impl Line {
    /// Builds a segment without an implicit closed interior.
    pub fn new(from: Point, to: Point) -> Self {
        Self {
            path: polyline(&[from, to], false),
        }
    }
}

stored_shape!(Polyline, "An open bounded sequence of line segments.");

impl Polyline {
    /// Captures at most MAX_PATH_VERBS-1 finite points; empty input is empty.
    pub fn new(values: impl IntoIterator<Item = Point>) -> Self {
        Self {
            path: points(values).and_then(|values| polyline(&values, false)),
        }
    }
}

stored_shape!(
    Polygon,
    "A closed bounded polygon; self-intersections use the fill rule."
);

impl Polygon {
    /// Captures points and closes the last edge to the first point.
    pub fn new(values: impl IntoIterator<Item = Point>) -> Self {
        Self {
            path: points(values).and_then(|values| polyline(&values, true)),
        }
    }
}

fn radial(
    center: Point,
    outer: f64,
    inner: Option<f64>,
    count: usize,
) -> Result<Path2D, ValidationError> {
    radius(outer)?;
    if let Some(inner) = inner {
        radius(inner)?;
        if inner > outer {
            return Err(invalid(
                "Star inner radius must not exceed its outer radius",
            ));
        }
    }
    let minimum = if inner.is_some() { 2 } else { 3 };
    let vertices = count
        .checked_mul(if inner.is_some() { 2 } else { 1 })
        .filter(|count| *count < MAX_PATH_VERBS - 1);
    let vertices = vertices
        .filter(|_| count >= minimum)
        .ok_or_else(|| invalid("Radial shape point count is outside its supported range"))?;
    let values = (0..vertices)
        .map(|index| {
            let length = if index % 2 == 1 {
                inner.unwrap_or(outer)
            } else {
                outer
            };
            center
                + Vector2::from_angle(
                    -std::f64::consts::FRAC_PI_2
                        + std::f64::consts::TAU * index as f64 / vertices as f64,
                ) * length
        })
        .collect::<Vec<_>>();
    polyline(&values, true)
}

stored_shape!(
    RegularPolygon,
    "A regular polygon with its first vertex above the center."
);

impl RegularPolygon {
    /// Builds at least three equally spaced vertices on a radius.
    pub fn new(center: Point, radius: f64, sides: usize) -> Self {
        Self {
            path: radial(center, radius, None, sides),
        }
    }
}

stored_shape!(Star, "A closed star alternating outer and inner radii.");

impl Star {
    /// Builds at least two tips, alternating finite outer and inner radii.
    pub fn new(center: Point, count: usize, outer_radius: f64, inner_radius: f64) -> Self {
        Self {
            path: radial(center, outer_radius, Some(inner_radius), count),
        }
    }
}

/// A annulus or closed annular sector with opposite-winding inner contour.
#[derive(Clone, Debug)]
pub struct Ring {
    center: Point,
    inner: f64,
    outer: f64,
    start: f64,
    sweep: f64,
}

impl Ring {
    /// Builds a complete ring with 0 <= inner <= outer.
    pub fn new(center: Point, inner_radius: f64, outer_radius: f64) -> Self {
        Self {
            center,
            inner: inner_radius,
            outer: outer_radius,
            start: 0.0,
            sweep: std::f64::consts::TAU,
        }
    }

    /// Selects a clockwise-positive annular sector in radians; clamps to one turn.
    pub fn sweep(mut self, start_angle: f64, sweep_angle: f64) -> Self {
        self.start = start_angle;
        self.sweep = sweep_angle;
        self
    }

    /// Returns checked annular geometry; full rings retain a real winding hole.
    pub fn try_path(&self) -> Result<Path2D, ValidationError> {
        radius(self.inner)?;
        radius(self.outer)?;
        if self.inner > self.outer || !self.sweep.is_finite() {
            return Err(invalid("Ring requires inner <= outer and a finite sweep"));
        }
        let sweep = self
            .sweep
            .clamp(-std::f64::consts::TAU, std::f64::consts::TAU);
        let mut path = Path2D::new();
        path.try_arc_to(
            self.center,
            Vector2::new(self.outer, self.outer),
            self.start,
            sweep,
        )?;
        if sweep.abs() == std::f64::consts::TAU {
            path.try_close()?;
            if self.inner > 0.0 {
                let inner = Arc::new(self.center, self.inner, self.start, -sweep).try_path()?;
                for verb in inner.verbs() {
                    append(&mut path, *verb)?;
                }
                path.try_close()?;
            }
        } else {
            path.try_arc_to(
                self.center,
                Vector2::new(self.inner, self.inner),
                self.start + sweep,
                -sweep,
            )?;
            path.try_close()?;
        }
        Ok(path)
    }
}
shape_impl!(Ring);

fn append(path: &mut Path2D, verb: crate::PathVerb) -> Result<(), ValidationError> {
    match verb {
        crate::PathVerb::MoveTo(value) => path.try_move_to(value),
        crate::PathVerb::LineTo(value) => path.try_line_to(value),
        crate::PathVerb::QuadraticTo(control, end) => path.try_quadratic_to(control, end),
        crate::PathVerb::CubicTo(first, second, end) => path.try_cubic_to(first, second, end),
        crate::PathVerb::Close => path.try_close(),
    }
}

/// arrow-head geometry; Vee is an open strokable head.
#[derive(Clone, Copy, Debug, Default)]
pub enum ArrowHead {
    /// Filled triangular head integrated with the shaft.
    #[default]
    Triangle,
    /// Two open sides, intended for stroke paint.
    Vee,
    /// Shaft segment only.
    None,
}

/// A single-outline arrow, with independent shaft and head geometry.
#[derive(Clone, Debug)]
pub struct Arrow {
    from: Point,
    to: Point,
    head: ArrowHead,
    size: f64,
    head_width: Option<f64>,
    width: f64,
}

impl Arrow {
    /// Creates a width-one shaft and a ten-unit triangular head.
    pub fn new(from: Point, to: Point) -> Self {
        Self {
            from,
            to,
            head: ArrowHead::Triangle,
            size: 10.0,
            head_width: None,
            width: 1.0,
        }
    }

    /// Replaces head geometry.
    pub fn head(mut self, head: ArrowHead) -> Self {
        self.head = head;
        self
    }

    /// Sets head length and width; head length clamps to endpoint distance.
    pub fn head_size(mut self, size: f64) -> Self {
        self.size = size;
        self.head_width = None;
        self
    }

    /// Sets independent positive head length and full width. Length clamps to
    /// endpoint distance; explicit width stays unchanged for a short arrow.
    pub fn head_dimensions(mut self, length: f64, width: f64) -> Self {
        self.size = length;
        self.head_width = Some(width);
        self
    }

    /// Sets the full shaft width in logical coordinates.
    pub fn width(mut self, width: f64) -> Self {
        self.width = width;
        self
    }

    /// Returns a single filled arrow, an open Vee or shaft-only segment.
    pub fn try_path(&self) -> Result<Path2D, ValidationError> {
        radius(self.size)?;
        radius(self.width)?;
        if let Some(width) = self.head_width {
            radius(width)?;
            if self.size <= 0.0 || width <= 0.0 {
                return Err(invalid("Explicit arrow head dimensions must be positive"));
            }
        }
        let line = Line::new(self.from, self.to).try_path()?;
        let displacement = self.to - self.from;
        let length = displacement.length();
        if !length.is_finite() {
            return Err(invalid("Arrow length must remain finite"));
        }
        if length == 0.0 {
            return Ok(Path2D::new());
        }
        if matches!(self.head, ArrowHead::None) {
            return Ok(line);
        }
        let direction = displacement.normalize();
        let normal = Vector2::new(-direction.y, direction.x);
        let head = self.size.min(length);
        let half_head_width = self.head_width.unwrap_or(head) * 0.5;
        let neck = self.to - direction * head;
        if matches!(self.head, ArrowHead::Vee) {
            let mut path = line;
            path.try_move_to(neck + normal * half_head_width)?;
            path.try_line_to(self.to)?;
            path.try_line_to(neck - normal * half_head_width)?;
            return Ok(path);
        }
        polyline(
            &[
                self.from + normal * (self.width * 0.5),
                neck + normal * (self.width * 0.5),
                neck + normal * half_head_width,
                self.to,
                neck - normal * half_head_width,
                neck - normal * (self.width * 0.5),
                self.from - normal * (self.width * 0.5),
            ],
            true,
        )
    }
}
shape_impl!(Arrow);

/// A cardinal Catmull-Rom interpolating spline with clamped endpoint neighbors.
#[derive(Clone, Debug)]
pub struct Spline {
    points: Result<Shared<[Point]>, ValidationError>,
    tension: f64,
}

impl Spline {
    /// Captures a bounded point list, including empty/single-point degeneracies.
    pub fn through(values: impl IntoIterator<Item = Point>) -> Self {
        Self {
            points: points(values),
            tension: 0.0,
        }
    }

    /// Sets tension in 0..=1; zero is Catmull-Rom, one has zero endpoint tangents.
    pub fn tension(mut self, tension: f64) -> Self {
        self.tension = tension;
        self
    }

    /// Produces cubic spans passing through every supplied point.
    pub fn try_path(&self) -> Result<Path2D, ValidationError> {
        if !self.tension.is_finite() || !(0.0..=1.0).contains(&self.tension) {
            return Err(invalid("Spline tension must be in 0..=1"));
        }
        let values = self.points.as_ref().map_err(Clone::clone)?;
        let mut path = Path2D::new();
        if let Some(first) = values.first() {
            path.try_move_to(*first)?;
        }
        let factor = (1.0 - self.tension) / 6.0;
        for index in 0..values.len().saturating_sub(1) {
            let before = values[index.saturating_sub(1)];
            let after = values[(index + 2).min(values.len() - 1)];
            path.try_cubic_to(
                values[index] + (values[index + 1] - before) * factor,
                values[index + 1] - (after - values[index]) * factor,
                values[index + 1],
            )?;
        }
        Ok(path)
    }
}
shape_impl!(Spline);

/// A deterministically sampled function/parametric curve with finite output.
/// Callbacks must be pure; their panics propagate like other user callbacks.
#[derive(Clone)]
pub struct FunctionGraph {
    function: Shared<dyn Fn(f64) -> Point + Send + Sync>,
    domain: Range<f64>,
    samples: usize,
}

impl std::fmt::Debug for FunctionGraph {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("FunctionGraph")
            .field("domain", &self.domain)
            .field("samples", &self.samples)
            .finish_non_exhaustive()
    }
}

impl FunctionGraph {
    /// Samples y=f(x), including both domain endpoints, with 128 default points.
    pub fn new(function: impl Fn(f64) -> f64 + Send + Sync + 'static, domain: Range<f64>) -> Self {
        Self::parametric(
            move |parameter| Point::new(parameter, function(parameter)),
            domain,
        )
    }

    /// Samples an explicit two-dimensional curve over the given parameter domain.
    pub fn parametric(
        function: impl Fn(f64) -> Point + Send + Sync + 'static,
        domain: Range<f64>,
    ) -> Self {
        Self {
            function: Shared::new(function),
            domain,
            samples: 128,
        }
    }

    /// Sets 2..MAX_PATH_VERBS sample points; excessive requests fail before callbacks.
    pub fn samples(mut self, samples: usize) -> Self {
        self.samples = samples;
        self
    }

    /// Evaluates a bounded sample list and rejects nonfinite output, never bridges gaps.
    pub fn try_path(&self) -> Result<Path2D, ValidationError> {
        if !self.domain.start.is_finite()
            || !self.domain.end.is_finite()
            || self.domain.start >= self.domain.end
            || !(2..MAX_PATH_VERBS).contains(&self.samples)
        {
            return Err(invalid(
                "Function graph requires a finite increasing domain and bounded sample count",
            ));
        }
        let mut path = Path2D::new();
        for index in 0..self.samples {
            let progress = index as f64 / (self.samples - 1) as f64;
            let parameter = if index == 0 {
                self.domain.start
            } else if index + 1 == self.samples {
                self.domain.end
            } else {
                self.domain.start * (1.0 - progress) + self.domain.end * progress
            };
            let value = (self.function)(parameter);
            if index == 0 {
                path.try_move_to(value)?;
            } else {
                path.try_line_to(value)?;
            }
        }
        Ok(path)
    }
}
shape_impl!(FunctionGraph);

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{Color, Node, PixelSize, Point, Rect, Size, ValidationLimits};

    #[test]
    fn arrows_keep_independent_head_dimensions_single_fill_and_empty_endpoints() {
        for length in [40.0, 3.0] {
            let arrow = Arrow::new(Point::ORIGIN, Point::new(length, 0.0))
                .width(2.0)
                .head_dimensions(12.0, 10.0);
            let path = arrow.try_path().unwrap();
            assert_eq!(path.bounding_box(), Rect::new(0.0, -5.0, length, 5.0));
            assert!(path.verbs().iter().any(|verb| matches!(verb, crate::PathVerb::LineTo(point) if *point == Point::new(length, 0.0))));
            let commands = crate::DrawContext::default()
                .record(
                    &arrow.fill(Color::from_hex(0x118677)),
                    ValidationLimits::default(),
                )
                .unwrap();
            assert_eq!(commands.len(), 1);
            assert!(matches!(
                commands[0],
                crate::DrawCommand::Fill {
                    rule: crate::FillRule::NonZero,
                    ..
                }
            ));
        }
        let empty = Arrow::new(Point::new(2.0, 3.0), Point::new(2.0, 3.0))
            .head_dimensions(12.0, 10.0)
            .try_path()
            .unwrap();
        assert!(empty.verbs().is_empty());
        for invalid in [
            Arrow::new(Point::ORIGIN, Point::ORIGIN).head_dimensions(0.0, 10.0),
            Arrow::new(Point::ORIGIN, Point::ORIGIN).head_dimensions(12.0, f64::NAN),
        ] {
            assert!(invalid.try_path().is_err());
        }
    }

    #[test]
    fn shapes_validate_bounds_degeneracies_splines_and_function_samples() {
        assert_eq!(
            Circle::new(Point::ORIGIN, 3.0).bounding_box(),
            Rect::new(-3.0, -3.0, 3.0, 3.0)
        );
        assert_eq!(
            RoundedRect::new(Rect::new(0.0, 0.0, 6.0, 4.0), 100.0).bounding_box(),
            Rect::new(0.0, 0.0, 6.0, 4.0)
        );
        assert!(Circle::new(Point::ORIGIN, -1.0).try_path().is_err());
        assert!(
            Star::new(Point::ORIGIN, usize::MAX, 2.0, 1.0)
                .try_path()
                .is_err()
        );
        assert!(
            RegularPolygon::new(Point::ORIGIN, 1.0, 2)
                .try_path()
                .is_err()
        );
        assert!(Ring::new(Point::ORIGIN, 3.0, 2.0).try_path().is_err());
        assert!(
            Arrow::new(Point::ORIGIN, Point::ORIGIN)
                .try_path()
                .unwrap()
                .verbs()
                .is_empty()
        );
        assert!(Circle::new(Point::ORIGIN, 0.0).try_path().unwrap().length() == 0.0);
        let knots = [Point::ORIGIN, Point::new(2.0, 3.0), Point::new(4.0, 1.0)];
        let spline = Spline::through(knots).try_path().unwrap();
        assert!(
            matches!(spline.verbs()[1], crate::PathVerb::CubicTo(_, _, end) if end == knots[1])
        );
        assert!(Spline::through(knots).tension(f64::NAN).try_path().is_err());
        let graph = FunctionGraph::new(|value| value * value, -2.0..2.0)
            .samples(5)
            .try_path()
            .unwrap();
        assert_eq!(
            graph.verbs()[0],
            crate::PathVerb::MoveTo(Point::new(-2.0, 4.0))
        );
        assert_eq!(
            graph.verbs().last(),
            Some(&crate::PathVerb::LineTo(Point::new(2.0, 4.0)))
        );
        assert!(
            FunctionGraph::new(|_| f64::NAN, 0.0..1.0)
                .try_path()
                .is_err()
        );
        assert!(
            FunctionGraph::new(|_| panic!("Invalid count must precede callback"), 0.0..1.0)
                .samples(usize::MAX)
                .try_path()
                .is_err()
        );
        assert!(
            Node::from(Circle::new(Point::ORIGIN, -1.0).fill(Color::WHITE))
                .compile(
                    Size::new(8.0, 8.0),
                    PixelSize::new(8, 8),
                    ValidationLimits::default()
                )
                .is_err()
        );
    }
}
