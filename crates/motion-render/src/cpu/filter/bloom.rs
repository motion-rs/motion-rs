//! Bloom highlight pyramid for the scalar filter reference: 2x2 downsampling, a per-level blur
//! and bilinear coarse-to-fine sums.
//!
//! Level `k` pixel `(x, y)` averages the 2x2 block at `(2x, 2y)` of level `k - 1`, in absolute
//! device coordinates halved `k` times. Every level is blurred by the same `radius` in its own
//! pixels, which spans `radius * 2^k` device pixels. Starting from the coarsest level, each finer
//! level adds its weighted blur to the coarser sum sampled bilinearly at
//! `((x + 0.5) / 2 - 0.5, (y + 0.5) / 2 - 0.5)`. The GPU plan follows the same contract.

use super::blur::gaussian;
use super::{admitted, pass, sample};
use crate::PixelRect;
use crate::cpu::Surface;
use motion_core::budget::Budget;
use motion_core::filter::{bloom_down, bloom_up, union};
use motion_core::{Rect, ValidationError};

/// Replaces the highlights in `surface` over `domain` with `sum(factors[k] * level_k)` and
/// returns the summed region. `factors` holds one weight per level, at most eight.
pub(super) fn pyramid(
    surface: &mut Surface,
    domain: Rect,
    radius: f64,
    factors: &[f64],
    budget: &mut Budget,
    path: &str,
    quality: motion_core::RenderQuality,
) -> Result<Rect, ValidationError> {
    // Downsample every level from its unblurred parent before any level is blurred.
    let mut levels: Vec<(Surface, Rect)> = Vec::new();
    let mut pool = surface.pool.clone();
    for _ in 1..factors.len() {
        let (parent, parent_domain) = levels
            .last()
            .map_or((&*surface, domain), |(level, domain)| (level, *domain));
        let bounds = PixelRect {
            left: parent.bounds.left / 2,
            top: parent.bounds.top / 2,
            right: parent.bounds.right.div_ceil(2),
            bottom: parent.bounds.bottom.div_ceil(2),
        };
        let level_domain = bloom_down(parent_domain);
        let mut level = Surface::new_pooled(
            bounds,
            1.0,
            motion_core::drawing::BlendMode::SourceOver,
            [0.0; 4],
            budget,
            path,
            Some(&mut pool),
        )?;
        admitted(&level, 4, budget, path)?;
        pass(&mut level, level_domain, budget, path, |column, row, _| {
            let mut total = [0.0_f64; 4];
            for (x, y) in [(0, 0), (1, 0), (0, 1), (1, 1)] {
                let (x, y) = (column * 2 + x, row * 2 + y);
                if x >= parent.bounds.left
                    && x < parent.bounds.right
                    && y >= parent.bounds.top
                    && y < parent.bounds.bottom
                {
                    let pixel = parent.pixels[parent.index(x, y)];
                    for channel in 0..4 {
                        total[channel] += f64::from(pixel[channel]);
                    }
                }
            }
            Ok(total.map(|value| value * 0.25))
        })?;
        levels.push((level, level_domain));
    }
    let mut blurred = vec![gaussian(surface, domain, radius, budget, path, quality)?];
    for (level, level_domain) in &mut levels {
        blurred.push(gaussian(
            level,
            *level_domain,
            radius,
            budget,
            path,
            quality,
        )?);
    }
    // Sum coarse to fine; the coarsest level only takes its weight.
    let mut coarser: Option<(Surface, Rect)> = None;
    for (index, factor) in factors.iter().enumerate().rev() {
        let mut target = if index == 0 {
            None
        } else {
            Some(levels.pop().map(|(level, _)| level).ok_or_else(|| {
                ValidationError::invalid(path, "levels", "Missing Bloom pyramid level")
            })?)
        };
        let region = coarser.as_ref().map_or(blurred[index], |(_, region)| {
            union(bloom_up(*region), blurred[index])
        });
        let level = target.as_mut().unwrap_or(&mut *surface);
        admitted(level, 5, budget, path)?;
        pass(level, region, budget, path, |column, row, level| {
            let pixel = level.pixels[level.index(column, row)];
            let mut value = pixel.map(|channel| f64::from(channel) * factor);
            if let Some((coarse, _)) = &coarser {
                let sampled = sample(
                    coarse,
                    (f64::from(column) + 0.5) * 0.5 - 0.5,
                    (f64::from(row) + 0.5) * 0.5 - 0.5,
                );
                for channel in 0..4 {
                    value[channel] += sampled[channel];
                }
            }
            Ok(value)
        })?;
        if let Some((coarse, _)) = coarser.take() {
            super::release(coarse.pixels, budget, &pool);
        }
        coarser = target.map(|level| (level, region));
        if index == 0 {
            return Ok(region);
        }
    }
    Ok(domain)
}
