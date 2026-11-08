//! Straight-alpha color helpers, luminance thresholding and seeded grain noise for the scalar
//! filter reference.

use super::pass;
use crate::cpu::Surface;
use crate::prepared::PixelRect;
use motion_core::budget::Budget;
use motion_core::{Rect, ValidationError};

pub(super) fn straight(pixel: [f32; 4]) -> [f64; 4] {
    let alpha = f64::from(pixel[3]);
    if alpha == 0.0 {
        [0.0; 4]
    } else {
        [
            f64::from(pixel[0]) / alpha,
            f64::from(pixel[1]) / alpha,
            f64::from(pixel[2]) / alpha,
            alpha,
        ]
    }
}

pub(super) fn premultiplied(mut pixel: [f64; 4]) -> [f64; 4] {
    if pixel.iter().any(|value| !value.is_finite()) {
        return pixel;
    }
    pixel[3] = pixel[3].clamp(0.0, 1.0);
    for channel in 0..3 {
        pixel[channel] *= pixel[3];
    }
    pixel
}

pub(super) fn luminance(pixel: [f64; 4]) -> f64 {
    pixel[0] * 0.2126 + pixel[1] * 0.7152 + pixel[2] * 0.0722
}

pub(super) fn threshold(
    surface: &mut Surface,
    domain: Rect,
    value: f64,
    budget: &mut Budget,
    path: &str,
) -> Result<(), ValidationError> {
    pass(surface, domain, budget, path, |column, row, surface| {
        let pixel = surface.pixels[surface.index(column, row)];
        let luminance = luminance(straight(pixel));
        let factor = if luminance > value {
            1.0 - value / luminance
        } else {
            0.0
        };
        Ok(pixel.map(|value| f64::from(value) * factor))
    })
}

pub(super) fn noise(
    surface: &mut Surface,
    domain: Rect,
    seed: u64,
    amplitude: f64,
    origin: PixelRect,
    budget: &mut Budget,
    path: &str,
) -> Result<(), ValidationError> {
    pass(surface, domain, budget, path, |column, row, surface| {
        // Both seed words are folded; the WGSL twin hashes the same output-relative pixel.
        let hash = motion_core::noise::hash2(
            (i64::from(column) - i64::from(origin.left)) as i32,
            (i64::from(row) - i64::from(origin.top)) as i32,
            seed as u32 ^ (seed >> 32) as u32,
        );
        let delta = (f64::from(hash >> 8) / 16777216.0 * 2.0 - 1.0) * amplitude;
        let mut pixel = straight(surface.pixels[surface.index(column, row)]);
        for channel in &mut pixel[..3] {
            *channel += delta;
        }
        Ok(premultiplied(pixel))
    })
}
