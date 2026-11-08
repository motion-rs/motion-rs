//! Deterministic authoring mathematics and interpolation.

pub use libm::{atan2, cos, exp, hypot, pow, sin, sqrt, tan};
/// Single-precision variants for pixel-level texture generation.
pub use libm::{cosf, expf, hypotf, sinf, sqrtf};

/// Blends values with their established interpolation policy, including overshoot.
pub fn lerp<Value: crate::Interpolate>(start: &Value, end: &Value, progress: f64) -> Value {
    start.interpolate(end, progress)
}

/// Restricts progress to [0, 1], preserving NaN.
pub fn clamp01(value: f64) -> f64 {
    value.clamp(0.0, 1.0)
}

/// Cubic Hermite progress; equal edges produce a hard step, and NaN is preserved.
pub fn smoothstep(edge0: f64, edge1: f64, value: f64) -> f64 {
    if value.is_nan() {
        return value;
    }
    if edge0 == edge1 {
        return f64::from(value >= edge1);
    }
    let progress = clamp01((value - edge0) / (edge1 - edge0));
    progress * progress * (3.0 - 2.0 * progress)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn deterministic_helpers_preserve_interpolation_and_interval_boundaries() {
        assert_eq!(sin(0.25).to_bits(), libm::sin(0.25).to_bits());
        assert_eq!(lerp(&2.0, &6.0, 1.5), 8.0);
        assert_eq!(smoothstep(2.0, 4.0, 3.0), 0.5);
        assert_eq!(smoothstep(2.0, 4.0, 0.0), 0.0);
        assert_eq!(smoothstep(2.0, 4.0, 5.0), 1.0);
        assert_eq!(smoothstep(1.0, 1.0, 1.0), 1.0);
        assert!(clamp01(f64::NAN).is_nan());
    }
}
