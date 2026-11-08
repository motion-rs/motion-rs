//! Double-precision affine authoring transforms, with reading-order composition.

use super::{Point, Vector2};

/// A two-dimensional affine transform in double-precision design coordinates.
///
/// Coefficients are column-major `[xx, yx, xy, yy, translation_x, translation_y]`.
/// Positive rotation is clockwise in the downward-positive coordinate system.
/// [`Self::then`] applies transforms in reading order, not matrix-product order.
/// Device-space geometry retains these f64 coefficients until explicit storage boundaries.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Transform {
    coefficients: [f64; 6],
}

impl Default for Transform {
    fn default() -> Self {
        Self::IDENTITY
    }
}

impl Transform {
    /// Leaves positions and vectors unchanged
    pub const IDENTITY: Self = Self::from_coefficients([1.0, 0.0, 0.0, 1.0, 0.0, 0.0]);

    /// Preserves raw affine coefficients; consuming geometry validates them.
    pub const fn from_coefficients(coefficients: [f64; 6]) -> Self {
        Self { coefficients }
    }

    /// Returns column-major affine coefficients without reducing precision.
    pub const fn coefficients(self) -> [f64; 6] {
        self.coefficients
    }

    /// Translates positions without changing vectors.
    pub const fn translate(horizontal: f64, vertical: f64) -> Self {
        Self::from_coefficients([1.0, 0.0, 0.0, 1.0, horizontal, vertical])
    }

    /// Rotates clockwise around the coordinate origin, in radians.
    pub fn rotate(radians: f64) -> Self {
        let direction = Vector2::from_angle(radians);
        Self::from_coefficients([
            direction.x,
            direction.y,
            -direction.y,
            direction.x,
            0.0,
            0.0,
        ])
    }

    /// Uniformly scales around the coordinate origin; negative values reflect.
    pub const fn scale(factor: f64) -> Self {
        Self::scale_non_uniform(factor, factor)
    }

    /// Scales each axis independently around the coordinate origin.
    pub const fn scale_non_uniform(horizontal: f64, vertical: f64) -> Self {
        Self::from_coefficients([horizontal, 0.0, 0.0, vertical, 0.0, 0.0])
    }

    /// Skews each axis by the tangent of its angle in radians.
    /// Both shears act in one matrix, rather than as sequential operations.
    pub fn skew(horizontal: f64, vertical: f64) -> Self {
        Self::from_coefficients([
            1.0,
            libm::tan(vertical),
            libm::tan(horizontal),
            1.0,
            0.0,
            0.0,
        ])
    }

    /// Applies this transform first, followed by `next`.
    ///
    /// ```
    /// use motion_core::{Point, Transform};
    /// let transform = Transform::translate(10.0, 0.0).then(Transform::scale(2.0));
    /// assert_eq!(transform.apply(Point::new(1.0, 3.0)), Point::new(22.0, 6.0));
    /// ```
    pub fn then(self, next: Self) -> Self {
        let first = self.coefficients;
        let second = next.coefficients;
        let translation = next.apply(Point::new(first[4], first[5]));
        Self::from_coefficients([
            second[0] * first[0] + second[2] * first[1],
            second[1] * first[0] + second[3] * first[1],
            second[0] * first[2] + second[2] * first[3],
            second[1] * first[2] + second[3] * first[3],
            translation.x,
            translation.y,
        ])
    }

    /// Maps a position, including translation.
    pub fn apply(self, point: Point) -> Point {
        let matrix = self.coefficients;
        Point::new(
            matrix[0] * point.x + matrix[2] * point.y + matrix[4],
            matrix[1] * point.x + matrix[3] * point.y + matrix[5],
        )
    }

    /// Maps a displacement, ignoring translation.
    pub fn apply_vector(self, vector: Vector2) -> Vector2 {
        let matrix = self.coefficients;
        Vector2::new(
            matrix[0] * vector.x + matrix[2] * vector.y,
            matrix[1] * vector.x + matrix[3] * vector.y,
        )
    }

    /// Returns whether all affine coefficients are finite.
    pub fn is_finite(self) -> bool {
        self.coefficients.iter().all(|value| value.is_finite())
    }

    /// Inverts a finite nonsingular transform when its inverse remains finite.
    /// Returns `None` for singular/nonfinite inputs or unrepresentable results.
    /// Rows are scaled before computing the determinant to avoid ordinary
    /// underflow/overflow for extremely small, large or unequal axis scales.
    pub fn inverse(self) -> Option<Self> {
        if !self.is_finite() {
            return None;
        }
        let matrix = self.coefficients;
        let horizontal_scale = matrix[0].abs().max(matrix[2].abs());
        let vertical_scale = matrix[1].abs().max(matrix[3].abs());
        if horizontal_scale == 0.0 || vertical_scale == 0.0 {
            return None;
        }
        let normalized = [
            matrix[0] / horizontal_scale,
            matrix[1] / vertical_scale,
            matrix[2] / horizontal_scale,
            matrix[3] / vertical_scale,
        ];
        let determinant = normalized[0] * normalized[3] - normalized[1] * normalized[2];
        if determinant == 0.0 {
            return None;
        }
        let divide = |coefficient: f64, scale: f64| {
            let quotient = coefficient / determinant;
            if quotient.is_finite() {
                quotient / scale
            } else {
                (coefficient / scale) / determinant
            }
        };
        let linear = [
            divide(normalized[3], horizontal_scale),
            divide(-normalized[1], horizontal_scale),
            divide(-normalized[2], vertical_scale),
            divide(normalized[0], vertical_scale),
        ];
        let inverse = Self::from_coefficients([
            linear[0],
            linear[1],
            linear[2],
            linear[3],
            -linear[0] * matrix[4] - linear[2] * matrix[5],
            -linear[1] * matrix[4] - linear[3] * matrix[5],
        ]);
        inverse.is_finite().then_some(inverse)
    }

    /// Decomposes into scale, horizontal shear, rotation, then translation.
    ///
    /// The horizontal scale is positive; reflection is carried by vertical scale.
    /// Returns `None` for singular/nonfinite inputs or nonfinite components.
    /// Shear is stored as a coefficient, not an angle, to avoid losing precision
    /// when a large tangent is rounded to a right angle.
    pub fn decompose(self) -> Option<TransformComponents> {
        if !self.is_finite() {
            return None;
        }
        let matrix = self.coefficients;
        let horizontal_scale = libm::hypot(matrix[0], matrix[1]);
        if horizontal_scale == 0.0 || !horizontal_scale.is_finite() {
            return None;
        }
        let cosine = matrix[0] / horizontal_scale;
        let sine = matrix[1] / horizontal_scale;
        let projection = cosine * matrix[2] + sine * matrix[3];
        let vertical_scale = cosine * matrix[3] - sine * matrix[2];
        if vertical_scale == 0.0 || !vertical_scale.is_finite() {
            return None;
        }
        let shear = projection / vertical_scale;
        if !shear.is_finite() {
            return None;
        }
        Some(TransformComponents {
            translation: Vector2::new(matrix[4], matrix[5]),
            rotation: libm::atan2(matrix[1], matrix[0]),
            scale: Vector2::new(horizontal_scale, vertical_scale),
            shear,
        })
    }

    /// Interpolates decomposed components and takes the shortest rotation arc.
    /// A half-turn tie rotates clockwise; progress may extrapolate. Exact zero/one
    /// return the original matrices. If either endpoint cannot be decomposed,
    /// coefficients interpolate directly, allowing singular endpoints such as
    /// zero-scale entrances. Reflection changes can pass through a singular scale.
    pub fn interpolate(&self, target: &Self, progress: f64) -> Self {
        if progress == 0.0 {
            return *self;
        }
        if progress == 1.0 {
            return *target;
        }
        let (Some(first), Some(second)) = (self.decompose(), target.decompose()) else {
            return Self::from_coefficients(std::array::from_fn(|index| {
                self.coefficients[index] * (1.0 - progress) + target.coefficients[index] * progress
            }));
        };
        let mut rotation = (second.rotation - first.rotation).rem_euclid(std::f64::consts::TAU);
        if rotation > std::f64::consts::PI {
            rotation -= std::f64::consts::TAU;
        }
        TransformComponents {
            translation: first.translation.interpolate(&second.translation, progress),
            rotation: first.rotation + rotation * progress,
            scale: first.scale.interpolate(&second.scale, progress),
            shear: first.shear * (1.0 - progress) + second.shear * progress,
        }
        .compose()
    }
}

/// Canonical affine factors in scale, shear, rotation, translation application order.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct TransformComponents {
    /// Position-independent displacement applied last.
    pub translation: Vector2,
    /// Clockwise radians around the origin.
    pub rotation: f64,
    /// Per-axis scale; decomposition places any reflection in the vertical factor.
    pub scale: Vector2,
    /// Horizontal shear coefficient: adds `shear * vertical` to the horizontal axis.
    /// Unlike [`Transform::skew`], this field is not an angle.
    pub shear: f64,
}

impl TransformComponents {
    /// Reconstructs an affine matrix without converting shear into an angle.
    pub fn compose(self) -> Transform {
        let direction = Vector2::from_angle(self.rotation);
        Transform::from_coefficients([
            direction.x * self.scale.x,
            direction.y * self.scale.x,
            (direction.x * self.shear - direction.y) * self.scale.y,
            (direction.y * self.shear + direction.x) * self.scale.y,
            self.translation.x,
            self.translation.y,
        ])
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::AngleLiteral;

    fn close(actual: f64, expected: f64) {
        let tolerance = 1e-12 * actual.abs().max(expected.abs()).max(1.0);
        assert!(
            (actual - expected).abs() <= tolerance,
            "{actual} != {expected}"
        );
    }

    fn point_close(actual: Point, expected: Point) {
        close(actual.x, expected.x);
        close(actual.y, expected.y);
    }

    #[test]
    fn reading_order_matches_sequential_application_and_preserves_vector_semantics() {
        let first = Transform::translate(10.0, -4.0);
        let second = Transform::rotate(90.degrees());
        let point = Point::new(2.0, 3.0);
        point_close(
            first.then(second).apply(point),
            second.apply(first.apply(point)),
        );
        point_close(first.then(second).apply(point), Point::new(1.0, 12.0));
        point_close(second.then(first).apply(point), Point::new(7.0, -2.0));
        assert_eq!(
            first.apply_vector(Vector2::new(2.0, 3.0)),
            Vector2::new(2.0, 3.0)
        );
        assert_eq!(Transform::default(), Transform::IDENTITY);
        let offset = Transform::translate(16_777_217.125, 0.0);
        assert_eq!(offset.apply(Point::ORIGIN).x, 16_777_217.125);
    }

    #[test]
    fn inverse_handles_reflection_shear_and_extreme_axis_scales() {
        let transforms = [
            Transform::IDENTITY,
            Transform::scale_non_uniform(-2.0, 3.0)
                .then(Transform::skew(15.degrees(), -10.degrees()))
                .then(Transform::rotate(37.degrees()))
                .then(Transform::translate(31.0, -17.0)),
            Transform::scale_non_uniform(1e-200, 1e200),
            Transform::scale_non_uniform(1e200, 1e200),
            Transform::scale_non_uniform(1e-200, 1e-200),
        ];
        for transform in transforms {
            let inverse = transform.inverse().unwrap();
            for point in [
                Point::ORIGIN,
                Point::new(-2.5, 3.75),
                Point::new(17.0, -9.0),
            ] {
                point_close(inverse.apply(transform.apply(point)), point);
                point_close(transform.apply(inverse.apply(point)), point);
            }
        }
        assert!(Transform::scale_non_uniform(0.0, 1.0).inverse().is_none());
        assert!(Transform::scale(f64::from_bits(1)).inverse().is_none());
        assert!(Transform::translate(f64::NAN, 1.0).inverse().is_none());
        assert!(Transform::scale(f64::INFINITY).inverse().is_none());
    }

    #[test]
    fn decomposition_recomposes_and_carries_reflection_in_vertical_scale() {
        for scale in [
            Vector2::new(2.0, 3.0),
            Vector2::new(-2.0, 3.0),
            Vector2::new(-2.0, -3.0),
        ] {
            for shear in [-2.5, 0.0, 1.25, 1e20] {
                let transform = TransformComponents {
                    translation: Vector2::new(21.5, -12.25),
                    rotation: 0.0,
                    scale,
                    shear,
                }
                .compose();
                let parts = transform.decompose().unwrap();
                assert!(parts.scale.x > 0.0);
                assert_eq!(
                    parts.scale.y.is_sign_negative(),
                    (scale.x * scale.y).is_sign_negative()
                );
                for (actual, expected) in parts
                    .compose()
                    .coefficients()
                    .into_iter()
                    .zip(transform.coefficients())
                {
                    close(actual, expected);
                }
            }
        }
        let rotated = Transform::scale_non_uniform(-2.0, 3.0)
            .then(Transform::skew(0.2, -0.15))
            .then(Transform::rotate(-127.degrees()));
        let recomposed = rotated.decompose().unwrap().compose();
        point_close(
            recomposed.apply(Point::new(13.0, -8.0)),
            rotated.apply(Point::new(13.0, -8.0)),
        );
        assert!(Transform::scale(0.0).decompose().is_none());
        assert!(Transform::rotate(f64::NAN).decompose().is_none());
    }

    #[test]
    fn transform_interpolation_uses_short_rotation_arcs_and_preserves_endpoints() {
        let first = Transform::rotate(170.degrees());
        let second = Transform::rotate(-170.degrees());
        let midpoint = first.interpolate(&second, 0.5);
        point_close(midpoint.apply(Point::new(1.0, 0.0)), Point::new(-1.0, 0.0));
        assert_eq!(first.interpolate(&second, 0.0), first);
        assert_eq!(first.interpolate(&second, 1.0), second);
        let half_turn = Transform::IDENTITY.interpolate(&Transform::rotate(180.degrees()), 0.5);
        point_close(half_turn.apply(Point::new(1.0, 0.0)), Point::new(0.0, 1.0));
        let singular = Transform::scale(0.0).interpolate(&Transform::scale(2.0), 0.5);
        point_close(singular.apply(Point::new(2.0, 3.0)), Point::new(2.0, 3.0));
        let extended = Transform::IDENTITY.interpolate(&Transform::translate(10.0, -20.0), 1.5);
        assert_eq!(extended.apply(Point::ORIGIN), Point::new(15.0, -30.0));
    }
}
