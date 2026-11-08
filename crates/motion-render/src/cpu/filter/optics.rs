//! Per-pixel lens optics (distortion, vignette, bokeh, radial blur, flare, relight) and
//! channel-offset sampling for the scalar filter reference.

use super::{admitted, checked, release, sample, storage_pooled};
use crate::cpu::Surface;
use crate::raster::prepare::pixel_bounds;
use motion_core::budget::Budget;
use motion_core::{FilterOperation, PixelSize, Rect, ValidationError, Vector2};

pub(super) fn optical_pixel(
    surface: &Surface,
    position: [f64; 2],
    values: &[f64; 32],
    kernel: &[[f32; 2]; 64],
) -> [f64; 4] {
    // Lens, Bokeh with a nonzero radius and RadialBlur never read the unshifted source.
    let source = || source_pixel(surface, position);
    match values[0] as u32 {
        5 => {
            let delta = [position[0] + 0.5 - values[1], position[1] + 0.5 - values[2]];
            let mut result = source();
            for ghost in 0..values[3] as u32 {
                let scale = -(0.25 + f64::from(ghost) * 0.3);
                let sampled = sample(
                    surface,
                    values[1] + delta[0] / scale - 0.5,
                    values[2] + delta[1] / scale - 0.5,
                );
                let luma = if sampled[3] > 0.0 {
                    (sampled[0] * 0.2126 + sampled[1] * 0.7152 + sampled[2] * 0.0722) / sampled[3]
                } else {
                    0.0
                };
                let extracted = (luma - values[4]).max(0.0) / luma.max(1e-6);
                let ring = (1.0
                    - (libm::hypot(delta[0], delta[1]) / values[9] - 0.55).abs() / 0.08)
                    .clamp(0.0, 1.0);
                let weight = values[5] * extracted / values[3] * (0.75 + ring * 0.25);
                for channel in 0..3 {
                    result[channel] += sampled[channel] * values[6 + channel] * weight;
                }
                result[3] = (result[3] + sampled[3] * values[10] * weight).min(1.0);
            }
            result
        }
        0 => {
            let delta = [position[0] + 0.5 - values[4], position[1] + 0.5 - values[5]];
            let fit = if values[3] != 0.0 {
                (1.0 + values[1].max(0.0)) * (1.0 + values[2].abs())
            } else {
                1.0
            };
            let delta = delta.map(|value| value / fit);
            let squared = (delta[0] * delta[0] + delta[1] * delta[1]) / (values[6] * values[6]);
            let mut result = [0.0; 4];
            for channel in 0..3 {
                let scale = (1.0 + values[1] * squared)
                    * (1.0 + (channel as f64 - 1.0) * values[2] * squared);
                let sampled = sample(
                    surface,
                    values[4] + delta[0] * scale - 0.5,
                    values[5] + delta[1] * scale - 0.5,
                );
                result[channel] = sampled[channel];
                result[3] = result[3].max(sampled[3]);
            }
            result
        }
        1 => {
            let radius = values[6].max(values[7]);
            let axes = [
                values[6] + (radius - values[6]) * values[3],
                values[7] + (radius - values[7]) * values[3],
            ];
            let relative = [
                (position[0] + 0.5 - values[4]) / axes[0],
                (position[1] + 0.5 - values[5]) / axes[1],
            ];
            let cosine_squared = 1.0
                / (1.0
                    + (relative[0] * relative[0] + relative[1] * relative[1])
                        * values[1]
                        * values[1]);
            let factor = 1.0 - values[2] + values[2] * cosine_squared * cosine_squared;
            let source = source();
            [
                source[0] * factor,
                source[1] * factor,
                source[2] * factor,
                source[3],
            ]
        }
        2 => {
            if values[1] == 0.0 {
                return source();
            }
            let mut result = [0.0; 4];
            let mut total = 0.0;
            // `values[3]` is the radius-scaled tap count; later offsets are unused zeros.
            for tap in &kernel[..(values[3] as usize).min(kernel.len())] {
                let sampled = sample(
                    surface,
                    position[0] + f64::from(tap[0]),
                    position[1] + f64::from(tap[1]),
                );
                let luma = if sampled[3] > 0.0 {
                    (sampled[0] * 0.2126 + sampled[1] * 0.7152 + sampled[2] * 0.0722) / sampled[3]
                } else {
                    0.0
                };
                let weight = 1.0 + luma.max(0.0) * values[2];
                for channel in 0..4 {
                    result[channel] += sampled[channel] * weight;
                }
                total += weight;
            }
            result.map(|value| value / total)
        }
        3 => {
            let mut result = [0.0; 4];
            let mut total = 0.0;
            let mut weight = 1.0;
            for tap in 0..values[4] as usize {
                let fraction = tap as f64 / values[4].max(1.0) * values[3];
                let sampled = sample(
                    surface,
                    position[0] + (values[1] - position[0] - 0.5) * fraction,
                    position[1] + (values[2] - position[1] - 0.5) * fraction,
                );
                for channel in 0..4 {
                    result[channel] += sampled[channel] * weight;
                }
                total += weight;
                weight *= values[5];
            }
            result.map(|value| value / total)
        }
        _ => {
            let mut gradient = [0.0; 2];
            for vertical in -1..=1 {
                for horizontal in -1..=1 {
                    let sampled = sample(
                        surface,
                        position[0] + f64::from(horizontal),
                        position[1] + f64::from(vertical),
                    );
                    let height = if values[1] != 0.0 {
                        sampled[3]
                    } else if sampled[3] > 0.0 {
                        (sampled[0] * 0.2126 + sampled[1] * 0.7152 + sampled[2] * 0.0722)
                            / sampled[3]
                    } else {
                        0.0
                    };
                    gradient[0] +=
                        height * f64::from(horizontal) * if vertical == 0 { 2.0 } else { 1.0 }
                            / 8.0;
                    gradient[1] +=
                        height * f64::from(vertical) * if horizontal == 0 { 2.0 } else { 1.0 }
                            / 8.0;
                }
            }
            let length = libm::sqrt(1.0 + gradient[0] * gradient[0] + gradient[1] * gradient[1]);
            let normal = [-gradient[0] / length, -gradient[1] / length, 1.0 / length];
            let diffuse =
                (normal[0] * values[2] + normal[1] * values[3] + normal[2] * values[4]).max(0.0);
            let half_length = libm::sqrt(
                values[2] * values[2]
                    + values[3] * values[3]
                    + (values[4] + 1.0) * (values[4] + 1.0),
            );
            let highlight = if half_length > 0.0 {
                libm::pow(
                    ((normal[0] * values[2]
                        + normal[1] * values[3]
                        + normal[2] * (values[4] + 1.0))
                        / half_length)
                        .max(0.0),
                    32.0,
                ) * values[9]
            } else {
                0.0
            };
            let source = source();
            [
                source[0] * (values[8] + values[5] * diffuse) + source[3] * values[5] * highlight,
                source[1] * (values[8] + values[6] * diffuse) + source[3] * values[6] * highlight,
                source[2] * (values[8] + values[7] * diffuse) + source[3] * values[7] * highlight,
                source[3],
            ]
        }
    }
}

/// Equals `sample(surface, x, y)`, but reads whole-pixel positions directly. There the one
/// weighted term is `pixel * 1 * 1` and every other term is `±0.0`, so the bilinear sum starting
/// at `+0.0` is exactly `0.0 + pixel`.
pub(super) fn source_pixel(surface: &Surface, [x, y]: [f64; 2]) -> [f64; 4] {
    if x.fract() != 0.0 || y.fract() != 0.0 {
        return sample(surface, x, y);
    }
    let bounds = surface.bounds;
    if x < f64::from(bounds.left)
        || x >= f64::from(bounds.right)
        || y < f64::from(bounds.top)
        || y >= f64::from(bounds.bottom)
    {
        return [0.0; 4];
    }
    surface.pixels[surface.index(x as u32, y as u32)].map(|value| 0.0 + f64::from(value))
}

/// Side of the square tiles used to find output that a gather filter can skip.
const ZERO_TILE: u32 = 16;

/// Output tiles whose gather footprint lies entirely over zero source pixels.
///
/// Bokeh averages its taps with positive weights, and a tap over four zero pixels samples
/// `+0.0`, so every output pixel in such a tile is exactly zero. Skipping them leaves the
/// output bit-identical.
pub(super) struct ZeroFootprint {
    columns: u32,
    skip: Vec<bool>,
    bytes: u64,
}

impl ZeroFootprint {
    /// Marks skippable tiles for taps at most `kernel` away from each pixel, or returns `None`
    /// when the kernel reach is not a small finite distance.
    pub(super) fn new(
        surface: &Surface,
        kernel: &[[f32; 2]; 64],
        budget: &mut Budget,
        path: &str,
    ) -> Result<Option<Self>, ValidationError> {
        let extent = kernel
            .iter()
            .flatten()
            .map(|value| f64::from(value.abs()))
            .fold(0.0, f64::max);
        let bounds = surface.bounds;
        let (width, height) = (bounds.right - bounds.left, bounds.bottom - bounds.top);
        if !extent.is_finite() || extent >= f64::from(width.max(height)) {
            return Ok(None);
        }
        // A tap at offset `d` reads `floor(d)` and `floor(d) + 1`.
        let reach = extent.ceil() as u32 + 1;
        let columns = width.div_ceil(ZERO_TILE);
        let rows = height.div_ceil(ZERO_TILE);
        let tiles = columns as usize * rows as usize;
        let bytes = ((columns as u64 + 1) * (rows as u64 + 1)) * size_of::<u32>() as u64
            + tiles as u64 * size_of::<bool>() as u64;
        budget.bytes(bytes, path)?;
        budget.work(surface.pixels.len() as u64 + tiles as u64, path)?;
        // Inclusive prefix sums of occupied tiles, with a zero first row and column.
        let stride = columns as usize + 1;
        let mut occupied = vec![0_u32; stride * (rows as usize + 1)];
        for (index, pixel) in surface.pixels.iter().enumerate() {
            if *pixel != [0.0; 4] {
                let column = (index as u32 % width) / ZERO_TILE;
                let row = (index as u32 / width) / ZERO_TILE;
                occupied[(row as usize + 1) * stride + column as usize + 1] = 1;
            }
        }
        for row in 1..=rows as usize {
            for column in 1..=columns as usize {
                occupied[row * stride + column] += occupied[row * stride + column - 1]
                    + occupied[(row - 1) * stride + column]
                    - occupied[(row - 1) * stride + column - 1];
            }
        }
        let span = reach.div_ceil(ZERO_TILE);
        let mut skip = vec![false; tiles];
        for row in 0..rows {
            for column in 0..columns {
                let (left, right) = (
                    column.saturating_sub(span),
                    (column + span).min(columns - 1),
                );
                let (top, bottom) = (row.saturating_sub(span), (row + span).min(rows - 1));
                let count =
                    |row: u32, column: u32| occupied[row as usize * stride + column as usize];
                let sum = count(bottom + 1, right + 1) + count(top, left)
                    - count(top, right + 1)
                    - count(bottom + 1, left);
                skip[(row * columns + column) as usize] = sum == 0;
            }
        }
        drop(occupied);
        Ok(Some(Self {
            columns,
            skip,
            bytes,
        }))
    }

    /// Whether the output pixel at surface coordinates `column`, `row` is exactly zero.
    pub(super) fn contains(&self, surface: &Surface, column: u32, row: u32) -> bool {
        let column = (column - surface.bounds.left) / ZERO_TILE;
        let row = (row - surface.bounds.top) / ZERO_TILE;
        self.skip[(row * self.columns + column) as usize]
    }

    /// Releases the admitted scratch bytes.
    pub(super) fn release(self, budget: &mut Budget) {
        budget.release(self.bytes);
    }
}

/// The two source lines that bilinear sampling at `coordinate` blends, with their weights.
/// A line outside `start..end` is `None` and contributes nothing, like transparent black.
/// So does a line with weight zero: its term is `±0.0`, and a sum that starts at `+0.0` never
/// becomes `-0.0`, so adding it cannot change the sum. A whole-pixel offset thereby samples
/// one line instead of two.
pub(super) fn taps(coordinate: f64, start: u32, end: u32) -> [(f64, Option<usize>); 2] {
    let left = coordinate.floor();
    let fraction = coordinate - left;
    let line = |position: f64, weight: f64| {
        (weight != 0.0 && position >= f64::from(start) && position < f64::from(end))
            .then(|| (position as u32 - start) as usize)
    };
    let before = 1.0 - fraction;
    [
        (before, line(left, before)),
        (fraction, line(left + 1.0, fraction)),
    ]
}

pub(super) fn channels(
    surface: &mut Surface,
    domain: Rect,
    offsets: [Vector2; 3],
    budget: &mut Budget,
    path: &str,
) -> Result<Rect, ValidationError> {
    let output = FilterOperation::ChannelOffset {
        red: offsets[0],
        green: offsets[1],
        blue: offsets[2],
    }
    .bounds(domain, domain);
    admitted(surface, 12, budget, path)?;
    let mut pixels = storage_pooled(surface.pixels.len(), budget, path, &surface.pool)?;
    let bounds = surface.bounds.intersect(pixel_bounds(
        output,
        PixelSize::new(surface.bounds.right, surface.bounds.bottom),
    ));
    let width = (surface.bounds.right - surface.bounds.left) as usize;
    for row in bounds.top..bounds.bottom {
        // Each output channel samples its own offset copy, and only that channel and alpha of
        // the four taps are needed. The terms are summed in the order `sample` uses.
        let vertical = offsets.map(|offset| {
            taps(
                f64::from(row) - offset.y,
                surface.bounds.top,
                surface.bounds.bottom,
            )
        });
        let target = (row - surface.bounds.top) as usize * width;
        for column in bounds.left..bounds.right {
            let mut pixel = [0.0_f64; 4];
            for channel in 0..3 {
                let horizontal = taps(
                    f64::from(column) - offsets[channel].x,
                    surface.bounds.left,
                    surface.bounds.right,
                );
                let mut value = 0.0;
                let mut alpha = 0.0;
                for (factor_x, x) in horizontal {
                    for (factor_y, y) in vertical[channel] {
                        if let (Some(x), Some(y)) = (x, y) {
                            let source = surface.pixels[y * width + x];
                            value += f64::from(source[channel]) * factor_x * factor_y;
                            alpha += f64::from(source[3]) * factor_x * factor_y;
                        }
                    }
                }
                pixel[channel] = value;
                pixel[3] = pixel[3].max(alpha);
            }
            pixels[target + (column - surface.bounds.left) as usize] = checked(pixel, path)?;
        }
    }
    release(
        std::mem::replace(&mut surface.pixels, pixels),
        budget,
        &surface.pool,
    );
    Ok(output)
}
