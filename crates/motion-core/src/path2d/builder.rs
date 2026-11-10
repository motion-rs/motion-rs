//! Chainable path construction that reports its first error once.

use super::Path2D;
use crate::{Point, ValidationError, Vector2};

/// Collects path verbs fluently and reports the first failure from [`Self::build`].
///
/// Once a step fails, later steps are ignored so the reported error is the first one.
///
/// ```
/// use motion_core::{Path2D, Point};
/// let path = Path2D::builder()
///     .move_to(Point::ORIGIN)
///     .line_to(Point::new(4.0, 0.0))
///     .close()
///     .build()?;
/// assert_eq!(path.verbs().len(), 3);
/// assert!(Path2D::builder().line_to(Point::ORIGIN).build().is_err());
/// # Ok::<(), motion_core::ValidationError>(())
/// ```
#[derive(Clone, Debug, Default)]
#[must_use = "a path builder does nothing until `build` is called"]
pub struct PathBuilder {
    path: Path2D,
    error: Option<ValidationError>,
}

impl PathBuilder {
    /// Creates an empty builder.
    pub fn new() -> Self {
        Self::default()
    }

    /// Starts a new subpath.
    pub fn move_to(self, point: Point) -> Self {
        self.step(|path| path.try_move_to(point))
    }

    /// Adds a line; requires a preceding move.
    pub fn line_to(self, point: Point) -> Self {
        self.step(|path| path.try_line_to(point))
    }

    /// Adds a quadratic Bezier; requires a preceding move.
    pub fn quadratic_to(self, control: Point, end: Point) -> Self {
        self.step(|path| path.try_quadratic_to(control, end))
    }

    /// Adds a cubic Bezier; requires a preceding move.
    pub fn cubic_to(self, first: Point, second: Point, end: Point) -> Self {
        self.step(|path| path.try_cubic_to(first, second, end))
    }

    /// Appends a clockwise elliptical arc; see [`Path2D::try_arc_to`].
    pub fn arc_to(self, center: Point, radii: Vector2, start_angle: f64, sweep_angle: f64) -> Self {
        self.step(|path| path.try_arc_to(center, radii, start_angle, sweep_angle))
    }

    /// Closes the current subpath; requires a preceding move.
    pub fn close(self) -> Self {
        self.step(Path2D::try_close)
    }

    /// Returns the finished path or the first error recorded while building.
    pub fn build(self) -> Result<Path2D, ValidationError> {
        match self.error {
            Some(error) => Err(error),
            None => Ok(self.path),
        }
    }

    fn step(mut self, apply: impl FnOnce(&mut Path2D) -> Result<(), ValidationError>) -> Self {
        if self.error.is_none()
            && let Err(error) = apply(&mut self.path)
        {
            self.error = Some(error);
        }
        self
    }
}
