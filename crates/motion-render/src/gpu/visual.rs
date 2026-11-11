//! Visual comparison for native startup and coverage fixtures, without byte-identity gates.

use motion_core::ValidationError;
use motion_core::color::{linear_to_srgb, srgb_to_linear};

/// Checks alpha and appearance on black and white, excluding mutually transparent padding.
pub(super) fn compare_rgba8(
    actual: &[[u8; 4]],
    expected: &[[u8; 4]],
    rms_limit: f64,
) -> Result<(), ValidationError> {
    if actual.len() != expected.len() || !rms_limit.is_finite() || rms_limit <= 0.0 {
        return Err(ValidationError::invalid(
            "gpu.visual",
            "comparison",
            "Visual comparison requires equal pixel counts and a finite positive tolerance",
        ));
    }
    let mut squared = 0.0;
    let mut samples = 0_u64;
    for (actual, expected) in actual.iter().zip(expected) {
        let a = actual.map(|value| f64::from(value) / 255.0);
        let b = expected.map(|value| f64::from(value) / 255.0);
        if a[3] == 0.0 && b[3] == 0.0 {
            continue;
        }
        squared += (a[3] - b[3]).powi(2);
        samples += 1;
        let a_linear = [a[0], a[1], a[2]].map(srgb_to_linear);
        let b_linear = [b[0], b[1], b[2]].map(srgb_to_linear);
        for background in [0.0, 1.0] {
            for channel in 0..3 {
                let first = linear_to_srgb(a_linear[channel] * a[3] + background * (1.0 - a[3]));
                let second = linear_to_srgb(b_linear[channel] * b[3] + background * (1.0 - b[3]));
                squared += (first - second).powi(2);
                samples += 1;
            }
        }
    }
    let rms = (squared / samples.max(1) as f64).sqrt();
    if rms > rms_limit {
        return Err(ValidationError::invalid(
            "gpu.visual",
            "appearance",
            format!("Visual RMS difference {rms:.6} exceeds fixture tolerance {rms_limit:.6}"),
        ));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn visual_comparison_allows_rounding_but_rejects_missing_color_and_alpha() {
        let reference = [[200, 80, 40, 128]; 4];
        assert!(compare_rgba8(&[[202, 79, 41, 127]; 4], &reference, 0.01).is_ok());
        assert!(compare_rgba8(&[[40, 80, 200, 128]; 4], &reference, 0.01).is_err());
        assert!(compare_rgba8(&[[200, 80, 40, 0]; 4], &reference, 0.01).is_err());
        assert!(compare_rgba8(&[[200, 80, 40, 255]; 4], &reference, 0.01).is_err());
        assert!(compare_rgba8(&reference[..3], &reference, 0.01).is_err());
        assert!(compare_rgba8(&reference, &reference, f64::NAN).is_err());
    }

    #[test]
    fn transparent_padding_cannot_dilute_visible_errors() {
        let mut reference = [[0; 4]; 4096];
        let mut actual = reference;
        reference[0] = [255; 4];
        actual[0] = [0, 0, 0, 255];
        assert!(compare_rgba8(&actual, &reference, 0.01).is_err());
        assert!(compare_rgba8(&[[255, 0, 0, 0]], &[[0, 255, 0, 0]], 0.01).is_ok());
    }
}
