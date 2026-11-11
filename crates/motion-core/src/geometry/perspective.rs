//! Planar 2.5D perspective: a flat layer rotated in 3D and viewed through a pinhole camera.

use crate::{Path2D, PathVerb, Point, ValidationError};

#[cfg(test)]
mod tests;

/// Maximum cubic pieces used for each curve verb in [`Perspective::project_path`].
pub const MAX_PERSPECTIVE_CURVE_PIECES: u32 = 64;

/// Smallest accepted fraction of the camera distance between the camera and a point.
const NEAR_FRACTION: f64 = 1e-3;

const LABEL: &str = "perspective";

/// Rotates the plane `z = 0` about a pivot and projects it onto the viewport.
///
/// The camera sits `distance` units in front of the plane on the `z` axis, looking
/// toward the plane, and the pivot stays fixed on screen. Rotations apply in the order
/// z (in-plane), y, then x, in radians. Positive `rotation_x` swings the bottom edge
/// toward the viewer, and positive `rotation_y` swings the left edge toward the viewer.
/// A point at depth `z` scales by `distance / (distance - z)`.
///
/// Clipping is explicit: a point closer than a thousandth of `distance` to the camera
/// plane, or behind it, is an error rather than being clipped or wrapped. A plane seen
/// from behind remains drawable (mirrored); [`Self::faces_camera`] reports the side so
/// callers can hide it. Only geometry is projected, so the result renders identically
/// on the CPU and GPU backends.
///
/// ```
/// use motion_core::{Perspective, Point};
/// let tilt = Perspective::new(Point::new(100.0, 100.0), 0.0, 0.5, 0.0, 800.0)?;
/// let near = tilt.project(Point::new(0.0, 100.0))?;
/// let far = tilt.project(Point::new(200.0, 100.0))?;
/// assert!(100.0 - near.x > far.x - 100.0);
/// assert!(tilt.faces_camera());
/// # Ok::<(), motion_core::ValidationError>(())
/// ```
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Perspective {
    pivot: Point,
    rotation: [f64; 3],
    distance: f64,
}

impl Perspective {
    /// Creates a projection; angles are finite radians and `distance` is finite and positive.
    pub fn new(
        pivot: Point,
        rotation_x: f64,
        rotation_y: f64,
        rotation_z: f64,
        distance: f64,
    ) -> Result<Self, ValidationError> {
        let finite = [pivot.x, pivot.y, rotation_x, rotation_y, rotation_z]
            .iter()
            .all(|value| value.is_finite());
        if !finite {
            return Err(ValidationError::invalid(
                LABEL,
                "rotation",
                "Pivot and angles must be finite",
            ));
        }
        if !distance.is_finite() || distance <= 0.0 {
            return Err(ValidationError::invalid(
                LABEL,
                "distance",
                "Camera distance must be finite and positive",
            ));
        }
        Ok(Self {
            pivot,
            rotation: [rotation_x, rotation_y, rotation_z],
            distance,
        })
    }

    /// The fixed point of the projection.
    pub fn pivot(&self) -> Point {
        self.pivot
    }

    /// Rotation angles about the x, y and z axes, in radians.
    pub fn rotation(&self) -> [f64; 3] {
        self.rotation
    }

    /// Camera distance from the plane, in the same units as path coordinates.
    pub fn distance(&self) -> f64 {
        self.distance
    }

    /// Whether the rotated plane's front side points toward the camera.
    pub fn faces_camera(&self) -> bool {
        libm::cos(self.rotation[0]) * libm::cos(self.rotation[1]) > 0.0
    }

    /// Projects one plane point, failing when it reaches the camera plane.
    pub fn project(&self, point: Point) -> Result<Point, ValidationError> {
        let (x, y) = (point.x - self.pivot.x, point.y - self.pivot.y);
        let [rotation_x, rotation_y, rotation_z] = self.rotation;
        let (sin, cos) = (libm::sin(rotation_z), libm::cos(rotation_z));
        let (x, y) = (x * cos - y * sin, x * sin + y * cos);
        let (sin, cos) = (libm::sin(rotation_y), libm::cos(rotation_y));
        let (x, z) = (x * cos, -x * sin);
        let (sin, cos) = (libm::sin(rotation_x), libm::cos(rotation_x));
        let (y, z) = (y * cos - z * sin, y * sin + z * cos);
        let depth = self.distance - z;
        if !depth.is_finite() || depth <= self.distance * NEAR_FRACTION {
            return Err(ValidationError::invalid(
                LABEL,
                "depth",
                "Point reaches or passes the camera plane",
            ));
        }
        let scale = self.distance / depth;
        Ok(Point::new(
            self.pivot.x + x * scale,
            self.pivot.y + y * scale,
        ))
    }

    /// Projects every verb of a path; each curve becomes `pieces` cubics whose controls
    /// are projected, which approximates the exact rational curve.
    ///
    /// Lines stay lines, because projection preserves straight edges. Fails when `pieces`
    /// is outside `1..=`[`MAX_PERSPECTIVE_CURVE_PIECES`], a point reaches the camera
    /// plane, or the result exceeds the path verb limit.
    pub fn project_path(&self, path: &Path2D, pieces: u32) -> Result<Path2D, ValidationError> {
        if !(1..=MAX_PERSPECTIVE_CURVE_PIECES).contains(&pieces) {
            return Err(ValidationError::limit(
                LABEL,
                "pieces",
                "Curve pieces must be between 1 and 64",
            ));
        }
        let mut result = Path2D::new();
        let mut current = Point::ORIGIN;
        let mut start = Point::ORIGIN;
        for verb in path.verbs() {
            match *verb {
                PathVerb::MoveTo(point) => {
                    result.try_move_to(self.project(point)?)?;
                    current = point;
                    start = point;
                }
                PathVerb::LineTo(point) => {
                    result.try_line_to(self.project(point)?)?;
                    current = point;
                }
                PathVerb::QuadraticTo(control, end) => {
                    let first = lerp(current, control, 2.0 / 3.0);
                    let second = lerp(end, control, 2.0 / 3.0);
                    self.cubic(&mut result, [current, first, second, end], pieces)?;
                    current = end;
                }
                PathVerb::CubicTo(first, second, end) => {
                    self.cubic(&mut result, [current, first, second, end], pieces)?;
                    current = end;
                }
                PathVerb::Close => {
                    result.try_close()?;
                    current = start;
                }
            }
        }
        Ok(result)
    }

    fn cubic(
        &self,
        result: &mut Path2D,
        curve: [Point; 4],
        pieces: u32,
    ) -> Result<(), ValidationError> {
        let curve = kurbo::CubicBez::new(
            (curve[0].x, curve[0].y),
            (curve[1].x, curve[1].y),
            (curve[2].x, curve[2].y),
            (curve[3].x, curve[3].y),
        );
        for index in 0..pieces {
            let piece = kurbo::ParamCurve::subsegment(
                &curve,
                f64::from(index) / f64::from(pieces)..f64::from(index + 1) / f64::from(pieces),
            );
            let [first, second, end] = [piece.p1, piece.p2, piece.p3];
            result.try_cubic_to(
                self.project(Point::new(first.x, first.y))?,
                self.project(Point::new(second.x, second.y))?,
                self.project(Point::new(end.x, end.y))?,
            )?;
        }
        Ok(())
    }
}

fn lerp(from: Point, to: Point, amount: f64) -> Point {
    Point::new(
        from.x + (to.x - from.x) * amount,
        from.y + (to.y - from.y) * amount,
    )
}
