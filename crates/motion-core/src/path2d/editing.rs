//! Chainable and fallible path-building operations, including elliptical arcs.

use super::{MAX_PATH_VERBS, Path2D, PathVerb, invalid};
use crate::{Point, ValidationError, Vector2};
use std::sync::Arc;

impl Path2D {
    /// Starts a subpath; invalid input is reported when the path is compiled.
    pub fn move_to(&mut self, point: Point) -> &mut Self {
        self.author(PathVerb::MoveTo(point))
    }

    /// Starts a new subpath.
    pub fn try_move_to(&mut self, point: Point) -> Result<(), ValidationError> {
        self.push(PathVerb::MoveTo(point))
    }

    /// Adds a line, deferring invalid state or coordinates to compilation.
    pub fn line_to(&mut self, point: Point) -> &mut Self {
        self.author(PathVerb::LineTo(point))
    }

    /// Adds a line; requires a preceding move.
    pub fn try_line_to(&mut self, point: Point) -> Result<(), ValidationError> {
        self.push(PathVerb::LineTo(point))
    }

    /// Adds a quadratic curve, deferring invalid input to compilation.
    pub fn quadratic_to(&mut self, control: Point, end: Point) -> &mut Self {
        self.author(PathVerb::QuadraticTo(control, end))
    }

    /// Adds a quadratic Bezier; requires a preceding move.
    pub fn try_quadratic_to(&mut self, control: Point, end: Point) -> Result<(), ValidationError> {
        self.push(PathVerb::QuadraticTo(control, end))
    }

    /// Adds a cubic curve, deferring invalid input to compilation.
    pub fn cubic_to(&mut self, first: Point, second: Point, end: Point) -> &mut Self {
        self.author(PathVerb::CubicTo(first, second, end))
    }

    /// Adds a cubic Bezier; requires a preceding move.
    pub fn try_cubic_to(
        &mut self,
        first: Point,
        second: Point,
        end: Point,
    ) -> Result<(), ValidationError> {
        self.push(PathVerb::CubicTo(first, second, end))
    }

    /// Appends an arc and retains the first invalid input for compile-time reporting.
    pub fn arc_to(
        &mut self,
        center: Point,
        radii: Vector2,
        start_angle: f64,
        sweep_angle: f64,
    ) -> &mut Self {
        if self.data.error.is_none()
            && let Err(error) = self.try_arc_to(center, radii, start_angle, sweep_angle)
        {
            Arc::make_mut(&mut self.data).error = Some(error);
        }
        self
    }

    /// Appends a clockwise elliptical arc in radians, connecting to its start.
    /// Radii must be finite and nonnegative. Cubics span at most pi/8 radians;
    /// errors, including exhausted verb limits, leave the path unchanged.
    /// Circular arcs use a cubic approximation, not rational/exact circle verbs.
    ///
    /// ```
    /// use motion_core::{Path2D, Point, Vector2};
    /// let mut path = Path2D::new();
    /// path.arc_to(
    ///     Point::ORIGIN,
    ///     Vector2::new(4.0, 4.0),
    ///     0.0,
    ///     std::f64::consts::FRAC_PI_2,
    /// );
    /// assert!((path.length() - 2.0 * std::f64::consts::PI).abs() < 1e-6);
    /// let middle = path.trim(0.25, 0.75);
    /// assert!((middle.length() - path.length() * 0.5).abs() < 1e-7);
    /// # Ok::<(), motion_core::ValidationError>(())
    /// ```
    pub fn try_arc_to(
        &mut self,
        center: Point,
        radii: Vector2,
        start_angle: f64,
        sweep_angle: f64,
    ) -> Result<(), ValidationError> {
        self.elliptical_arc(center, radii, start_angle, sweep_angle, 0.0)
    }

    pub(super) fn elliptical_arc(
        &mut self,
        center: Point,
        radii: Vector2,
        start_angle: f64,
        sweep_angle: f64,
        rotation: f64,
    ) -> Result<(), ValidationError> {
        if ![
            center.x,
            center.y,
            radii.x,
            radii.y,
            start_angle,
            sweep_angle,
            rotation,
        ]
        .iter()
        .all(|value| value.is_finite())
            || radii.x < 0.0
            || radii.y < 0.0
        {
            return Err(invalid(
                "arc",
                "Arc parameters must be finite with nonnegative radii",
            ));
        }
        let count = libm::ceil(sweep_angle.abs() / std::f64::consts::FRAC_PI_8);
        if count > MAX_PATH_VERBS.saturating_sub(self.verbs().len() + 1) as f64 {
            return Err(ValidationError::limit(
                "path2d",
                "verbs",
                "Arc exceeds the verb allowance",
            ));
        }
        let start = start_angle.rem_euclid(std::f64::consts::TAU);
        let rotate = |vector: Vector2| vector.rotate(rotation);
        let direction = |angle: f64| {
            let angle = angle.rem_euclid(std::f64::consts::TAU);
            Vector2::from_angle(if angle > std::f64::consts::PI {
                angle - std::f64::consts::TAU
            } else {
                angle
            })
        };
        let position = |angle: f64| {
            let vector = direction(angle);
            center + rotate(Vector2::new(radii.x * vector.x, radii.y * vector.y))
        };
        let tangent = |angle: f64| {
            let vector = direction(angle);
            rotate(Vector2::new(-radii.x * vector.y, radii.y * vector.x))
        };
        let mut result = self.clone();
        let first = position(start);
        match result.data.current {
            None => result.try_move_to(first)?,
            Some(current) if current != first => result.try_line_to(first)?,
            _ => {}
        }
        if count > 0.0 {
            let step = sweep_angle / count;
            let factor = 4.0 / 3.0 * libm::tan(step / 4.0);
            for index in 0..count as usize {
                let angle = start + step * index as f64;
                let next = start + step * (index + 1) as f64;
                result.try_cubic_to(
                    position(angle) + tangent(angle) * factor,
                    position(next) - tangent(next) * factor,
                    position(next),
                )?;
            }
        }
        *self = result;
        Ok(())
    }

    /// Closes the current contour, deferring invalid state to compilation.
    pub fn close(&mut self) -> &mut Self {
        self.author(PathVerb::Close)
    }

    /// Closes the current subpath; requires a preceding move.
    pub fn try_close(&mut self) -> Result<(), ValidationError> {
        self.push(PathVerb::Close)
    }

    /// Closes and returns this path for direct styling.
    pub fn closed(mut self) -> Self {
        self.close();
        self
    }

    fn author(&mut self, verb: PathVerb) -> &mut Self {
        if let Err(error) = self.push(verb)
            && self.data.error.is_none()
        {
            Arc::make_mut(&mut self.data).error = Some(error);
        }
        self
    }
}
