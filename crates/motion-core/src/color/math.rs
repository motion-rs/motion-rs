//! Pure color arithmetic without rendering resources or implicit color conversion.

/// Decodes one extended sRGB channel to linear light without clamping HDR values.
/// Negative values use the linear toe; nonfinite inputs follow IEEE arithmetic.
pub fn srgb_to_linear(value: f32) -> f32 {
    if value <= 0.04045 {
        value / 12.92
    } else {
        libm::powf((value + 0.055) / 1.055, 2.4)
    }
}

/// Encodes one linear-light channel as extended sRGB without clamping HDR values.
/// Negative values use the linear toe; nonfinite inputs follow IEEE arithmetic.
pub fn linear_to_srgb(value: f32) -> f32 {
    if value <= 0.0031308 {
        value * 12.92
    } else {
        1.055 * libm::powf(value, 1.0 / 2.4) - 0.055
    }
}

/// Computes the Rec.709 weighted sum without transfer conversion or clamping.
/// This is relative luminance only when `rgb` is linear light. Encoded-space
/// masks may deliberately use the same weights without decoding their channels.
pub fn rec709_luminance(rgb: [f32; 3]) -> f32 {
    rgb[0] * 0.2126 + rgb[1] * 0.7152 + rgb[2] * 0.0722
}

/// Evaluates Rec.709 linear luminance without narrowing intermediates.
pub fn rec709_luminance_f64(rgb: [f64; 3]) -> f64 {
    rgb[0] * 0.2126 + rgb[1] * 0.7152 + rgb[2] * 0.0722
}

/// Multiplies straight RGB by alpha, returning premultiplied RGBA in the same space.
/// Zero alpha returns transparent black, ignoring hidden RGB even if nonfinite.
/// Other alpha values are not validated or clamped; callers normally supply
/// finite alpha in `0..=1`. HDR channels and arithmetic overflow are not clipped.
pub fn premultiply(rgb: [f32; 3], alpha: f32) -> [f32; 4] {
    if alpha == 0.0 {
        return [0.0; 4];
    }
    [rgb[0] * alpha, rgb[1] * alpha, rgb[2] * alpha, alpha]
}

/// Divides premultiplied RGB by alpha without changing color space or clamping.
/// Returns black when alpha is not positive, ignoring hidden RGB. Positive
/// alpha is used as-is, so very small alpha can produce HDR or overflow.
pub fn unpremultiply(pixel: [f32; 4]) -> [f32; 3] {
    if pixel[3] > 0.0 {
        [
            pixel[0] / pixel[3],
            pixel[1] / pixel[3],
            pixel[2] / pixel[3],
        ]
    } else {
        [0.0; 3]
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn transfer_functions_preserve_breakpoints_hdr_and_round_trips() {
        for value in [-0.5_f32, 0.0, 0.0031308, 0.04045, 0.18, 0.5, 1.0, 4.0, 64.0] {
            let decoded = if value <= 0.04045 {
                value / 12.92
            } else {
                libm::powf((value + 0.055) / 1.055, 2.4)
            };
            let encoded = if value <= 0.0031308 {
                value * 12.92
            } else {
                1.055 * libm::powf(value, 1.0 / 2.4) - 0.055
            };
            assert_eq!(srgb_to_linear(value).to_bits(), decoded.to_bits());
            assert_eq!(linear_to_srgb(value).to_bits(), encoded.to_bits());
            assert!(
                (linear_to_srgb(srgb_to_linear(value)) - value).abs() < 1e-5 * value.abs().max(1.0)
            );
        }
        assert!(srgb_to_linear(2.0) > 1.0);
        assert!(linear_to_srgb(2.0) > 1.0);
        assert!(srgb_to_linear(f32::NAN).is_nan());
        assert!(linear_to_srgb(f32::INFINITY).is_infinite());
    }

    #[test]
    fn weighted_sums_preserve_precision_and_do_not_convert_color_space() {
        for rgb in [[0.0_f32; 3], [1.0; 3], [0.1, 0.7, 0.3], [32.0, 0.0, 0.0]] {
            let expected = rgb[0] * 0.2126 + rgb[1] * 0.7152 + rgb[2] * 0.0722;
            assert_eq!(rec709_luminance(rgb).to_bits(), expected.to_bits());
        }
        let rgb = [f64::from(f32::MAX) * 65536.0, 0.0, 0.0];
        assert!(rec709_luminance_f64(rgb).is_finite());
        assert_eq!(rec709_luminance_f64(rgb), rgb[0] * 0.2126);
        assert_eq!(rec709_luminance([0.5; 3]), 0.5);
        assert_ne!(rec709_luminance([0.5; 3]), srgb_to_linear(0.5));
    }

    #[test]
    fn alpha_helpers_preserve_hdr_and_ignore_transparent_hidden_rgb() {
        let rgb = [4.0, 0.5, 0.25];
        let pixel = premultiply(rgb, 0.25);
        assert_eq!(pixel, [1.0, 0.125, 0.0625, 0.25]);
        assert_eq!(unpremultiply(pixel), rgb);
        assert_eq!(premultiply([f32::NAN, f32::INFINITY, 100.0], 0.0), [0.0; 4]);
        assert_eq!(
            unpremultiply([f32::NAN, f32::INFINITY, 100.0, 0.0]),
            [0.0; 3]
        );
        assert_eq!(unpremultiply([1.0, 2.0, 3.0, -0.5]), [0.0; 3]);
    }
}
