//! Coverage extraction from a completed linear-premultiplied mask.

use motion_core::drawing::MaskMode;

pub(crate) fn coverage(pixel: [f32; 4], mode: MaskMode) -> f64 {
    let coverage = if pixel[3] == 0.0 {
        0.0
    } else {
        match mode {
            MaskMode::Alpha | MaskMode::InvertedAlpha => f64::from(pixel[3]),
            MaskMode::Luminance | MaskMode::InvertedLuminance => {
                motion_core::color::math::rec709_luminance_f64([
                    f64::from(pixel[0]),
                    f64::from(pixel[1]),
                    f64::from(pixel[2]),
                ])
            }
        }
    }
    .clamp(0.0, 1.0);
    if matches!(mode, MaskMode::InvertedAlpha | MaskMode::InvertedLuminance) {
        1.0 - coverage
    } else {
        coverage
    }
}

#[cfg(test)]
mod tests;
