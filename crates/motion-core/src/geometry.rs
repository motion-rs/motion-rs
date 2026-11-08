//! Double-precision authoring points, vectors, transforms and bounds.

use std::ops::{Add, Div, Mul, Neg, Sub};

pub mod bounds;
pub mod flatten;
mod transform;

pub use transform::{Transform, TransformComponents};

/// A position in two-dimensional design coordinates, without implicit narrowing.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Point {
    /// Horizontal position, increasing toward the right.
    pub x: f64,
    /// Vertical position, increasing downward.
    pub y: f64,
}

impl Point {
    /// The coordinate origin.
    pub const ORIGIN: Self = Self::new(0.0, 0.0);

    /// Preserves the supplied coordinates; validation belongs to the consumer.
    pub const fn new(horizontal: f64, vertical: f64) -> Self {
        Self {
            x: horizontal,
            y: vertical,
        }
    }

    /// Returns Euclidean distance using deterministic pure-Rust math.
    pub fn distance(self, other: Self) -> f64 {
        (self - other).length()
    }

    /// Linearly interpolates or extrapolates with double-precision progress.
    pub fn interpolate(&self, target: &Self, progress: f64) -> Self {
        Self::new(
            self.x * (1.0 - progress) + target.x * progress,
            self.y * (1.0 - progress) + target.y * progress,
        )
    }
}

impl Add<Vector2> for Point {
    type Output = Self;

    fn add(self, displacement: Vector2) -> Self {
        Self::new(self.x + displacement.x, self.y + displacement.y)
    }
}

impl Sub<Vector2> for Point {
    type Output = Self;

    fn sub(self, displacement: Vector2) -> Self {
        Self::new(self.x - displacement.x, self.y - displacement.y)
    }
}

impl Sub for Point {
    type Output = Vector2;

    fn sub(self, origin: Self) -> Vector2 {
        Vector2::new(self.x - origin.x, self.y - origin.y)
    }
}

/// A displacement or direction, distinct from a [`Point`] position.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Vector2 {
    /// Horizontal displacement.
    pub x: f64,
    /// Vertical displacement.
    pub y: f64,
}

impl Vector2 {
    /// A displacement with no movement.
    pub const ZERO: Self = Self::new(0.0, 0.0);

    /// Preserves the supplied components; validation belongs to the consumer.
    pub const fn new(horizontal: f64, vertical: f64) -> Self {
        Self {
            x: horizontal,
            y: vertical,
        }
    }

    /// Returns a unit vector at clockwise radians from the positive horizontal axis.
    /// Principal quarter-turn angles preserve exact axis alignment.
    pub fn from_angle(radians: f64) -> Self {
        if radians == 0.0 {
            Self::new(1.0, 0.0)
        } else if radians.abs() == std::f64::consts::PI {
            Self::new(-1.0, 0.0)
        } else if radians == std::f64::consts::FRAC_PI_2 {
            Self::new(0.0, 1.0)
        } else if radians == -std::f64::consts::FRAC_PI_2 {
            Self::new(0.0, -1.0)
        } else {
            Self::new(libm::cos(radians), libm::sin(radians))
        }
    }

    /// Returns Euclidean length without squaring unscaled components.
    pub fn length(self) -> f64 {
        libm::hypot(self.x, self.y)
    }

    /// Returns a unit direction, or zero for a zero vector.
    /// Finite very large or very small components are scaled before normalization.
    /// Nonfinite input does not become a finite zero vector.
    pub fn normalize(self) -> Self {
        if self.x == 0.0 && self.y == 0.0 {
            return Self::ZERO;
        }
        let magnitude = self.x.abs().max(self.y.abs());
        let scaled = self / magnitude;
        scaled / scaled.length()
    }

    /// Returns the scalar dot product.
    pub fn dot(self, other: Self) -> f64 {
        self.x * other.x + self.y * other.y
    }

    /// Returns the signed two-dimensional cross product.
    pub fn cross(self, other: Self) -> f64 {
        self.x * other.y - self.y * other.x
    }

    /// Rotates clockwise in the downward-positive coordinate system.
    pub fn rotate(self, radians: f64) -> Self {
        let direction = Self::from_angle(radians);
        Self::new(
            direction.x * self.x - direction.y * self.y,
            direction.y * self.x + direction.x * self.y,
        )
    }

    /// Linearly interpolates or extrapolates with double-precision progress.
    pub fn interpolate(&self, target: &Self, progress: f64) -> Self {
        Self::new(
            self.x * (1.0 - progress) + target.x * progress,
            self.y * (1.0 - progress) + target.y * progress,
        )
    }
}

impl Add for Vector2 {
    type Output = Self;

    fn add(self, other: Self) -> Self {
        Self::new(self.x + other.x, self.y + other.y)
    }
}

impl Sub for Vector2 {
    type Output = Self;

    fn sub(self, other: Self) -> Self {
        Self::new(self.x - other.x, self.y - other.y)
    }
}

impl Neg for Vector2 {
    type Output = Self;

    fn neg(self) -> Self {
        Self::new(-self.x, -self.y)
    }
}

impl Mul<f64> for Vector2 {
    type Output = Self;

    fn mul(self, factor: f64) -> Self {
        Self::new(self.x * factor, self.y * factor)
    }
}

impl Mul<Vector2> for f64 {
    type Output = Vector2;

    fn mul(self, vector: Vector2) -> Vector2 {
        vector * self
    }
}

impl Div<f64> for Vector2 {
    type Output = Self;

    fn div(self, divisor: f64) -> Self {
        Self::new(self.x / divisor, self.y / divisor)
    }
}

/// Readable angle literals returning double-precision radians.
/// Nonfinite floating inputs propagate; consuming geometry validates them.
pub trait AngleLiteral {
    /// Converts degrees to radians, as in `45.degrees()`.
    fn degrees(self) -> f64;

    /// Converts complete revolutions to radians, as in `0.5.turns()`.
    fn turns(self) -> f64;
}

impl AngleLiteral for f64 {
    fn degrees(self) -> f64 {
        self.to_radians()
    }

    fn turns(self) -> f64 {
        self * std::f64::consts::TAU
    }
}

impl AngleLiteral for i32 {
    fn degrees(self) -> f64 {
        f64::from(self).degrees()
    }

    fn turns(self) -> f64 {
        f64::from(self).turns()
    }
}

impl AngleLiteral for i64 {
    fn degrees(self) -> f64 {
        (self as f64).degrees()
    }

    fn turns(self) -> f64 {
        (self as f64).turns()
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn design_points_preserve_precision_and_distinguish_displacements() {
        use super::{Point, Vector2};

        let origin = Point::new(16_777_217.125, -14.5);
        let displacement = Vector2::new(3.0, 4.0);
        let target = origin + displacement;
        assert_eq!(target - origin, displacement);
        assert_eq!(target - displacement, origin);
        assert_eq!(origin.distance(target), 5.0);
        assert_ne!(origin.x, f64::from(origin.x as f32));
        let progress = 1.0 / 3.0;
        assert_eq!(
            Point::ORIGIN.interpolate(&Point::new(1.0, 1.0), progress).x,
            progress
        );
        assert_eq!(
            Point::ORIGIN.interpolate(&Point::new(2.0, 4.0), 1.5),
            Point::new(3.0, 6.0)
        );
    }

    #[test]
    fn design_vectors_normalize_extreme_lengths_and_rotate_clockwise() {
        use super::{AngleLiteral, Vector2};

        assert_eq!(Vector2::ZERO.normalize(), Vector2::ZERO);
        for component in [f64::MAX, 1e-300, f64::from_bits(1)] {
            let unit = Vector2::new(component, component).normalize();
            assert!((unit.length() - 1.0).abs() < 1e-15);
            assert!((unit.x - std::f64::consts::FRAC_1_SQRT_2).abs() < 1e-15);
        }
        for component in [f64::NAN, f64::INFINITY] {
            let invalid = Vector2::new(component, 0.0).normalize();
            assert!(!invalid.x.is_finite() || !invalid.y.is_finite());
        }
        let rotated = Vector2::new(2.0, 0.0).rotate(90.degrees());
        assert!(rotated.x.abs() < 1e-14);
        assert!((rotated.y - 2.0).abs() < 1e-14);
        assert_eq!(Vector2::from_angle(180.degrees()), Vector2::new(-1.0, 0.0));
        assert_eq!(Vector2::from_angle(-90.degrees()), Vector2::new(0.0, -1.0));
        assert_eq!(Vector2::new(1.0, 0.0).cross(Vector2::new(0.0, 1.0)), 1.0);
        assert_eq!(Vector2::new(3.0, 4.0).dot(Vector2::new(-4.0, 3.0)), 0.0);
        assert_eq!(0.5.turns(), 180.degrees());
        assert_eq!(2_i64.turns(), 720.degrees());
        assert_eq!(2.0 * Vector2::new(3.0, 4.0) / 2.0, Vector2::new(3.0, 4.0));
    }
}
