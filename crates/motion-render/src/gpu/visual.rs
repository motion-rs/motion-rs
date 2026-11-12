//! Visual comparison for native startup and coverage fixtures, without byte-identity gates.

use motion_core::ValidationError;
use motion_core::color::{linear_to_srgb, srgb_to_linear};

/// Edge length, in pixels, of the square tiles whose RMS is checked independently.
const TILE_PIXELS: usize = 16;

/// Samples per pixel: alpha, then display RGB over black and over white.
const SAMPLES_PER_PIXEL: usize = 7;

/// Acceptance limits for one fixture, in normalized display units (one 8-bit code is 1/255).
#[derive(Clone, Copy, Debug, PartialEq)]
pub(super) struct VisualTolerance {
    /// Root-mean-square difference over every visible pixel.
    pub(super) rms: f64,
    /// Root-mean-square difference inside each 16x16 tile that has a visible pixel.
    pub(super) tile_rms: f64,
    /// Slack added to the 3x3 neighborhood range before a pixel counts as an outlier.
    pub(super) neighborhood_slack: f64,
    /// Fraction of visible pixels allowed outside the slack-expanded neighborhood range.
    pub(super) max_outlier_ratio: f64,
}

impl VisualTolerance {
    /// Same backend, device and inputs: replay may differ only by rounding.
    pub(super) const REPLAY: Self = Self {
        rms: 0.002,
        tile_rms: 0.004,
        neighborhood_slack: 2.0 / 255.0,
        max_outlier_ratio: 0.0,
    };

    /// Solid and gradient paints, blending and group opacity over analytic or pixel-aligned
    /// coverage.
    pub(super) const PAINT: Self = Self {
        rms: 0.005,
        tile_rms: 0.01,
        neighborhood_slack: 3.0 / 255.0,
        max_outlier_ratio: 0.0,
    };

    /// Antialiased paths and geometric clips without overlapping contours inside a pixel.
    #[cfg(test)]
    pub(super) const COVERAGE: Self = Self {
        rms: 0.01,
        tile_rms: 0.03,
        neighborhood_slack: 4.0 / 255.0,
        max_outlier_ratio: 0.0,
    };

    /// Self-intersecting or crossing contours. Opposite windings inside one pixel cancel in
    /// the GPU's signed-area accumulation, so isolated pixels at dense crossings may fall
    /// outside the reference neighborhood.
    pub(super) const OVERLAP: Self = Self {
        rms: 0.05,
        tile_rms: 0.12,
        neighborhood_slack: 8.0 / 255.0,
        max_outlier_ratio: 0.03,
    };

    fn validate(self) -> Result<(), ValidationError> {
        if [self.rms, self.tile_rms, self.neighborhood_slack]
            .iter()
            .all(|value| value.is_finite() && *value >= 0.0)
            && (0.0..=1.0).contains(&self.max_outlier_ratio)
        {
            return Ok(());
        }
        Err(ValidationError::invalid(
            "gpu.visual",
            "tolerance",
            "Visual tolerances must be finite and non-negative",
        ))
    }
}

/// Measured differences between two RGBA8 images, reported on success and failure.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(super) struct VisualDifference {
    /// Pixels visible in either image; mutually transparent pixels are excluded.
    pub(super) visible_pixels: usize,
    /// Root-mean-square difference over visible pixels.
    pub(super) rms: f64,
    /// Largest per-tile RMS difference.
    pub(super) worst_tile_rms: f64,
    /// Top-left pixel of the tile with the largest RMS difference.
    pub(super) worst_tile: [usize; 2],
    /// Pixels outside the other image's slack-expanded 3x3 neighborhood range.
    pub(super) outliers: usize,
    /// First outlier in row-major order.
    pub(super) first_outlier: Option<[usize; 2]>,
}

impl VisualDifference {
    /// Outliers as a fraction of visible pixels.
    pub(super) fn outlier_ratio(&self) -> f64 {
        self.outliers as f64 / self.visible_pixels.max(1) as f64
    }
}

/// Compares straight-alpha sRGB images of `width` columns against one fixture tolerance.
///
/// Every pixel contributes alpha and its appearance composited over black and white, so
/// color hidden by low alpha and alpha hidden by matching color are both visible. The
/// global and tile RMS limits bound diffuse error without letting transparent padding
/// dilute it. The neighborhood check accepts antialiasing that moves an edge by up to
/// one pixel, but rejects missing geometry, clip leakage, dropped thin features and
/// inverted alpha. Both images must contain a visible pixel.
pub(super) fn compare_rgba8(
    actual: &[[u8; 4]],
    expected: &[[u8; 4]],
    width: usize,
    tolerance: VisualTolerance,
) -> Result<VisualDifference, ValidationError> {
    tolerance.validate()?;
    let difference = measure(actual, expected, width, tolerance.neighborhood_slack)?;
    let failed = if difference.rms > tolerance.rms {
        Some(("appearance", "RMS", difference.rms, tolerance.rms))
    } else if difference.worst_tile_rms > tolerance.tile_rms {
        let worst = difference.worst_tile_rms;
        Some(("tile", "tile RMS", worst, tolerance.tile_rms))
    } else if difference.outlier_ratio() > tolerance.max_outlier_ratio {
        let ratio = difference.outlier_ratio();
        Some((
            "neighborhood",
            "outlier ratio",
            ratio,
            tolerance.max_outlier_ratio,
        ))
    } else {
        None
    };
    match failed {
        Some((field, metric, value, limit)) => Err(ValidationError::invalid(
            "gpu.visual",
            field,
            format!(
                "Visual {metric} {value:.6} exceeds fixture tolerance {limit:.6}; {difference:?}"
            ),
        )),
        None => Ok(difference),
    }
}

fn measure(
    actual: &[[u8; 4]],
    expected: &[[u8; 4]],
    width: usize,
    slack: f64,
) -> Result<VisualDifference, ValidationError> {
    if actual.len() != expected.len() || width == 0 || !actual.len().is_multiple_of(width) {
        return Err(ValidationError::invalid(
            "gpu.visual",
            "comparison",
            "Visual comparison requires equal pixel counts that fill whole rows",
        ));
    }
    let height = actual.len() / width;
    let actual_samples: Vec<_> = actual.iter().map(|pixel| samples(*pixel)).collect();
    let expected_samples: Vec<_> = expected.iter().map(|pixel| samples(*pixel)).collect();
    let tiles_wide = width.div_ceil(TILE_PIXELS);
    let mut tiles = vec![(0.0, 0_usize); tiles_wide * height.div_ceil(TILE_PIXELS)];
    let mut squared = 0.0;
    let mut visible_pixels = 0;
    let mut outliers = 0;
    let mut first_outlier = None;
    for y in 0..height {
        for x in 0..width {
            let index = y * width + x;
            if actual[index][3] == 0 && expected[index][3] == 0 {
                continue;
            }
            let (a, b) = (actual_samples[index], expected_samples[index]);
            let pixel_squared: f64 = a.iter().zip(b).map(|(a, b)| (a - b).powi(2)).sum();
            squared += pixel_squared;
            visible_pixels += 1;
            let tile = &mut tiles[y / TILE_PIXELS * tiles_wide + x / TILE_PIXELS];
            tile.0 += pixel_squared;
            tile.1 += 1;
            if !within_neighborhood(&a, &expected_samples, width, x, y, slack)
                || !within_neighborhood(&b, &actual_samples, width, x, y, slack)
            {
                outliers += 1;
                first_outlier.get_or_insert([x, y]);
            }
        }
    }
    let actual_visible = actual.iter().any(|pixel| pixel[3] > 0);
    let expected_visible = expected.iter().any(|pixel| pixel[3] > 0);
    if !actual_visible || !expected_visible {
        return Err(ValidationError::invalid(
            "gpu.visual",
            "nonblank",
            "Visual comparison requires a visible pixel in both images",
        ));
    }
    let rms_of =
        |squared: f64, pixels: usize| (squared / (pixels * SAMPLES_PER_PIXEL).max(1) as f64).sqrt();
    let (worst_tile, worst_tile_rms) = tiles
        .iter()
        .enumerate()
        .map(|(index, (squared, pixels))| (index, rms_of(*squared, *pixels)))
        .fold(
            (0, 0.0),
            |worst, tile| if tile.1 > worst.1 { tile } else { worst },
        );
    Ok(VisualDifference {
        visible_pixels,
        rms: rms_of(squared, visible_pixels),
        worst_tile_rms,
        worst_tile: [
            worst_tile % tiles_wide * TILE_PIXELS,
            worst_tile / tiles_wide * TILE_PIXELS,
        ],
        outliers,
        first_outlier,
    })
}

/// Normalized alpha followed by display RGB over black and over white.
fn samples(pixel: [u8; 4]) -> [f64; SAMPLES_PER_PIXEL] {
    let [red, green, blue, alpha] = pixel.map(|value| f64::from(value) / 255.0);
    let linear = [red, green, blue].map(srgb_to_linear);
    let mut samples = [alpha; SAMPLES_PER_PIXEL];
    for (background_index, background) in [0.0, 1.0].into_iter().enumerate() {
        for (channel, value) in linear.iter().enumerate() {
            samples[1 + background_index * 3 + channel] =
                linear_to_srgb(value * alpha + background * (1.0 - alpha));
        }
    }
    samples
}

/// Whether every sample lies within the other image's 3x3 range around the same pixel.
fn within_neighborhood(
    samples: &[f64; SAMPLES_PER_PIXEL],
    other: &[[f64; SAMPLES_PER_PIXEL]],
    width: usize,
    x: usize,
    y: usize,
    slack: f64,
) -> bool {
    let height = other.len() / width;
    let mut low = [f64::INFINITY; SAMPLES_PER_PIXEL];
    let mut high = [f64::NEG_INFINITY; SAMPLES_PER_PIXEL];
    for row in y.saturating_sub(1)..(y + 2).min(height) {
        for column in x.saturating_sub(1)..(x + 2).min(width) {
            for (sample, value) in other[row * width + column].iter().enumerate() {
                low[sample] = low[sample].min(*value);
                high[sample] = high[sample].max(*value);
            }
        }
    }
    samples
        .iter()
        .enumerate()
        .all(|(sample, value)| *value >= low[sample] - slack && *value <= high[sample] + slack)
}

#[cfg(test)]
mod tests {
    use super::*;

    const LIMIT: VisualTolerance = VisualTolerance {
        rms: 0.01,
        tile_rms: 0.01,
        neighborhood_slack: 3.0 / 255.0,
        max_outlier_ratio: 0.0,
    };

    fn square(size: usize, left: usize, right: usize, pixel: [u8; 4]) -> Vec<[u8; 4]> {
        (0..size * size)
            .map(|index| {
                let (x, y) = (index % size, index / size);
                if (left..right).contains(&x) && (left..right).contains(&y) {
                    pixel
                } else {
                    [0; 4]
                }
            })
            .collect()
    }

    #[test]
    fn visual_comparison_allows_rounding_but_rejects_missing_color_and_alpha() {
        let reference = [[200, 80, 40, 128]; 4];
        assert!(compare_rgba8(&[[202, 79, 41, 127]; 4], &reference, 2, LIMIT).is_ok());
        assert!(compare_rgba8(&[[40, 80, 200, 128]; 4], &reference, 2, LIMIT).is_err());
        assert!(compare_rgba8(&[[200, 80, 40, 0]; 4], &reference, 2, LIMIT).is_err());
        assert!(compare_rgba8(&[[200, 80, 40, 255]; 4], &reference, 2, LIMIT).is_err());
        assert!(compare_rgba8(&reference[..3], &reference, 2, LIMIT).is_err());
        assert!(compare_rgba8(&reference[..3], &reference[..3], 2, LIMIT).is_err());
        for invalid in [
            VisualTolerance {
                rms: f64::NAN,
                ..LIMIT
            },
            VisualTolerance {
                max_outlier_ratio: 1.5,
                ..LIMIT
            },
        ] {
            assert!(compare_rgba8(&reference, &reference, 2, invalid).is_err());
        }
    }

    #[test]
    fn blank_images_are_not_accepted_as_matching() {
        let blank = [[0; 4]; 4];
        assert_eq!(
            compare_rgba8(&blank, &blank, 2, LIMIT).unwrap_err().field,
            "nonblank"
        );
        assert!(compare_rgba8(&blank, &[[200, 80, 40, 128]; 4], 2, LIMIT).is_err());
        assert!(compare_rgba8(&[[255, 0, 0, 0]; 4], &[[0, 255, 0, 0]; 4], 2, LIMIT).is_err());
    }

    #[test]
    fn transparent_padding_cannot_dilute_visible_errors() {
        let mut reference = [[0; 4]; 4096];
        let mut actual = reference;
        reference[0] = [255; 4];
        actual[0] = [0, 0, 0, 255];
        assert!(compare_rgba8(&actual, &reference, 64, LIMIT).is_err());
        reference[1] = [255; 4];
        actual[1] = [255; 4];
        let relaxed = VisualTolerance {
            rms: 1.0,
            tile_rms: 1.0,
            ..LIMIT
        };
        let error = compare_rgba8(&actual, &reference, 64, relaxed).unwrap_err();
        assert_eq!(error.field, "neighborhood");
    }

    #[test]
    fn one_pixel_edge_shift_passes_but_missing_and_leaked_geometry_fail() {
        let white = [255; 4];
        let reference = square(32, 8, 24, white);
        let relaxed = VisualTolerance {
            rms: 1.0,
            tile_rms: 1.0,
            ..LIMIT
        };
        let shifted = square(32, 9, 25, white);
        let difference = compare_rgba8(&shifted, &reference, 32, relaxed).unwrap();
        assert_eq!(difference.outliers, 0);
        assert!(difference.rms > LIMIT.rms);
        let shrunk = square(32, 8, 23, white);
        let mut hole = reference.clone();
        hole[16 * 32 + 16] = [0; 4];
        let leaked = square(32, 6, 24, white);
        for broken in [hole, leaked] {
            let error = compare_rgba8(&broken, &reference, 32, relaxed).unwrap_err();
            assert_eq!(error.field, "neighborhood");
        }
        assert!(compare_rgba8(&shrunk, &reference, 32, relaxed).is_ok());
    }

    #[test]
    fn dropped_thin_lines_and_local_tile_errors_are_reported() {
        let mut reference = vec![[0, 0, 0, 255]; 32 * 32];
        for y in 0..32 {
            reference[y * 32 + 20] = [255; 4];
        }
        let dropped = vec![[0, 0, 0, 255]; 32 * 32];
        let relaxed = VisualTolerance {
            rms: 1.0,
            tile_rms: 1.0,
            ..LIMIT
        };
        let error = compare_rgba8(&dropped, &reference, 32, relaxed).unwrap_err();
        assert_eq!(error.field, "neighborhood");
        let budget = VisualTolerance {
            max_outlier_ratio: 32.0 / 1024.0,
            ..relaxed
        };
        let difference = compare_rgba8(&dropped, &reference, 32, budget).unwrap();
        assert_eq!(difference.outliers, 32);
        let mut local = reference.clone();
        for y in 16..32 {
            for x in 0..4 {
                local[y * 32 + x] = [12, 12, 12, 255];
            }
        }
        let global_only = VisualTolerance {
            rms: 1.0,
            tile_rms: 0.01,
            neighborhood_slack: 1.0,
            max_outlier_ratio: 0.0,
        };
        let error = compare_rgba8(&local, &reference, 32, global_only).unwrap_err();
        assert_eq!(error.field, "tile");
    }
}
