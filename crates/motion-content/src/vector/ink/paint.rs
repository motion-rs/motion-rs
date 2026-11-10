//! Paint evaluation for baked ink materials.

use super::paper::image_sample;
use motion_core::{Paint, Point, Rect, ValidationError};

/// Evaluates a paint at an absolute point as encoded straight-alpha RGBA.
pub(super) fn paint_sample(
    paint: &Paint,
    point: Point,
    bounds: Rect,
) -> Result<[f64; 4], ValidationError> {
    let linear = match paint {
        Paint::Solid(color) => color.to_linear_premultiplied(),
        Paint::Linear(gradient) => gradient.sample(point, bounds)?,
        Paint::Radial(gradient) => gradient.sample(point, bounds)?,
        Paint::Sweep(gradient) => gradient.sample(point, bounds)?,
        Paint::Pattern(pattern) => image_sample(pattern, point)?,
        _ => {
            return Err(ValidationError::invalid(
                "brush_stroke",
                "paint",
                "Unsupported brush paint",
            ));
        }
    };
    let mut encoded = [0.0; 4];
    encoded[3] = linear[3];
    if linear[3] > 0.0 {
        for channel in 0..3 {
            encoded[channel] = motion_core::color::linear_to_srgb(linear[channel] / linear[3]);
        }
    }
    Ok(encoded)
}
