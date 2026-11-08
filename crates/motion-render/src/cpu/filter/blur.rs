//! Separable box and Gaussian blurs plus fractional offsets for the scalar filter reference.
//!
//! Uniform boxes use integer taps for radii 1/2. Larger radii slide a window of 2r+1 samples
//! as the sum of a suffix and a prefix of two adjacent aligned blocks, so a sample is only
//! ever added, never subtracted again: a large HDR value that leaves the window cannot erase
//! the smaller values that remain. Blocks that only see transparent black are written without
//! summing, which cannot change any result.

use super::{admitted, checked, clear_outside, pass, release, sample, storage_overwritten};
use crate::cpu::Surface;
use crate::raster::prepare::pixel_bounds;
use motion_core::budget::Budget;
use motion_core::{FilterOperation, PixelSize, Rect, ValidationError, Vector2};

/// Target size of the suffix table; wider boxes process fewer lanes at a time. It is small
/// enough to stay in the L1 cache and to give a lane group a fair chance of being clear.
pub(super) const BOX_SCRATCH_BYTES: usize = 32 * 1024;

/// A strided view of the source lines being blurred. Position `p` of lane `l` is
/// `source[origin + p * stride + l]`; positions outside `0..length` read as transparent.
pub(super) struct Lines<'a> {
    pub(super) source: &'a [[f32; 4]],
    pub(super) origin: usize,
    pub(super) stride: usize,
    pub(super) lanes: usize,
    pub(super) length: i64,
}

impl Lines<'_> {
    pub(super) fn element(&self, position: i64, lane: usize) -> [f64; 4] {
        if position < 0 || position >= self.length {
            return [0.0; 4];
        }
        self.source[self.origin + position as usize * self.stride + lane].map(f64::from)
    }

    /// First position at or after `from` where any lane holds a bit pattern other than `+0.0`,
    /// or `length` when none does. A negative zero counts as set, which only makes callers
    /// skip less. The scan ORs the raw bits so it is branch-free and vectorizes.
    fn next_set(&self, from: i64) -> i64 {
        let set = |pixels: &[[f32; 4]]| {
            pixels.iter().fold(0_u32, |bits, pixel| {
                bits | pixel[0].to_bits()
                    | pixel[1].to_bits()
                    | pixel[2].to_bits()
                    | pixel[3].to_bits()
            }) != 0
        };
        let mut position = from.max(0);
        if self.lanes == 1 && self.stride == 1 {
            // One contiguous line: clear 32 positions per test, then locate the first set one.
            while position < self.length {
                let count = (self.length - position).min(32) as usize;
                let start = self.origin + position as usize;
                if set(&self.source[start..start + count]) {
                    return position
                        + self.source[start..start + count]
                            .iter()
                            .position(|pixel| set(std::slice::from_ref(pixel)))
                            .unwrap_or(0) as i64;
                }
                position += count as i64;
            }
            return self.length;
        }
        while position < self.length {
            let start = self.origin + position as usize * self.stride;
            if set(&self.source[start..start + self.lanes]) {
                return position;
            }
            position += 1;
        }
        self.length
    }
}

/// Reusable sums for one sliding box pass over `lanes` lines at once.
pub(super) struct BoxSums {
    pub(super) suffix: Vec<[f64; 4]>,
    pub(super) prefix: Vec<[f64; 4]>,
    pub(super) running: Vec<[f64; 4]>,
}

fn box_lanes(radius: i64, lanes: usize) -> usize {
    let width = (radius * 2 + 1) as usize;
    (BOX_SCRATCH_BYTES / (width * size_of::<[f64; 4]>())).clamp(1, lanes.max(1))
}

/// One block of `slide_box` over a single contiguous line whose windows all lie inside it. A
/// horizontal pass sums one line at a time; vertical passes keep the general path, whose
/// lane loops over `lanes` interleaved columns are already as fast as slicing them.
///
/// Sums are taken in the same order as the general path (the suffix from the block's end
/// downward, the prefix upward), so the result is identical; only the per-sample edge test,
/// lane loops and index arithmetic are gone.
#[allow(clippy::too_many_arguments)]
fn contiguous_block(
    lines: &Lines<'_>,
    radius: i64,
    outputs: std::ops::Range<i64>,
    end: i64,
    weight: f64,
    sums: &mut BoxSums,
    destination: &mut [[f32; 4]],
    path: &str,
) -> Result<(), ValidationError> {
    let width = (radius * 2 + 1) as usize;
    let base = outputs.start;
    let steps = (end - base) as usize;
    let first = lines.origin + (base - radius) as usize;
    // Padded coordinate `base - radius + i` is `window[i]`; the last needed is `end - 1 + radius`.
    let window = &lines.source[first..first + steps + width - 1];
    let mut running = [0.0_f64; 4];
    for (step, pixel) in window[..steps].iter().enumerate().rev() {
        for channel in 0..4 {
            running[channel] += f64::from(pixel[channel]);
        }
        sums.suffix[step] = running;
    }
    let mut prefix = [0.0_f64; 4];
    for pixel in &window[steps..width] {
        for channel in 0..4 {
            prefix[channel] += f64::from(pixel[channel]);
        }
    }
    let target = &mut destination[lines.origin + base as usize..][..(outputs.end - base) as usize];
    for (offset, output) in target.iter_mut().enumerate() {
        if offset != 0 {
            let pixel = window[width - 1 + offset];
            for channel in 0..4 {
                prefix[channel] += f64::from(pixel[channel]);
            }
        }
        let suffix = sums.suffix[offset];
        let mut mean = [0.0; 4];
        for channel in 0..4 {
            mean[channel] = (suffix[channel] + prefix[channel]) * weight;
        }
        *output = checked(mean, path)?;
    }
    Ok(())
}

/// One block of `slide_box` over several interleaved lanes whose windows all lie inside the
/// lines. Each position is then a contiguous slice of `lanes` pixels, so the per-element edge
/// test and index arithmetic of the general path disappear; sums are taken in the same order,
/// so results are identical.
#[allow(clippy::too_many_arguments)]
fn interior_lanes_block(
    lines: &Lines<'_>,
    radius: i64,
    outputs: std::ops::Range<i64>,
    end: i64,
    weight: f64,
    sums: &mut BoxSums,
    destination: &mut [[f32; 4]],
    path: &str,
) -> Result<(), ValidationError> {
    let lanes = lines.lanes;
    let base = outputs.start;
    let row = |position: i64| {
        let start = lines.origin + position as usize * lines.stride;
        &lines.source[start..start + lanes]
    };
    let running = &mut sums.running[..lanes];
    running.fill([0.0; 4]);
    for step in (0..(end - base) as usize).rev() {
        let suffix = &mut sums.suffix[step * lanes..][..lanes];
        for ((running, pixel), suffix) in running
            .iter_mut()
            .zip(row(base + step as i64 - radius))
            .zip(suffix)
        {
            for channel in 0..4 {
                running[channel] += f64::from(pixel[channel]);
            }
            *suffix = *running;
        }
    }
    let prefix = &mut sums.prefix[..lanes];
    prefix.fill([0.0; 4]);
    for padded in end..base + radius * 2 + 1 {
        for (prefix, pixel) in prefix.iter_mut().zip(row(padded - radius)) {
            for channel in 0..4 {
                prefix[channel] += f64::from(pixel[channel]);
            }
        }
    }
    for position in base..outputs.end {
        if position != base {
            for (prefix, pixel) in prefix.iter_mut().zip(row(position + radius)) {
                for channel in 0..4 {
                    prefix[channel] += f64::from(pixel[channel]);
                }
            }
        }
        let suffix = &sums.suffix[(position - base) as usize * lanes..][..lanes];
        let start = lines.origin + position as usize * lines.stride;
        for ((target, suffix), prefix) in destination[start..start + lanes]
            .iter_mut()
            .zip(suffix)
            .zip(prefix.iter())
        {
            let mut mean = [0.0; 4];
            for channel in 0..4 {
                mean[channel] = (suffix[channel] + prefix[channel]) * weight;
            }
            *target = checked(mean, path)?;
        }
    }
    Ok(())
}

/// Writes the mean of the `2r+1` samples centered on each output position. The window
/// `[p, p + 2r + 1)` in padded coordinates is a suffix of one block of that width plus a
/// prefix of the next, so it needs three additions per sample and no subtraction.
///
/// A block whose windows all lie in transparent black is written without summing. Every sum
/// would be zero and `checked` turns it into `[0.0; 4]`, so the result is the same bit for bit;
/// sparse layers such as bokeh discs or a thresholded glow are mostly such blocks.
pub(super) fn slide_box(
    lines: &Lines<'_>,
    radius: i64,
    outputs: std::ops::Range<i64>,
    weight: f64,
    sums: &mut BoxSums,
    destination: &mut [[f32; 4]],
    path: &str,
) -> Result<(), ValidationError> {
    let width = radius * 2 + 1;
    let lanes = lines.lanes;
    let mut position = outputs.start;
    // First set position at or after the last lookup point; positions only move forward, so
    // the line is scanned once however many blocks consult it.
    let mut next_set = i64::MIN;
    while position < outputs.end {
        let base = position;
        let end = (position / width + 1) * width;
        let last = end.min(outputs.end);
        let first_needed = (base - radius).max(0);
        if next_set < first_needed {
            next_set = lines.next_set(first_needed);
        }
        if next_set > (last - 1 + radius).min(lines.length - 1) {
            for cleared in base..last {
                let start = lines.origin + cleared as usize * lines.stride;
                destination[start..start + lanes].fill([0.0; 4]);
            }
            position = last;
            continue;
        }
        if lanes == 1 && lines.stride == 1 && base - radius >= 0 && end - 1 + radius < lines.length
        {
            // Every sample of the block's windows exists, so none needs the edge handling.
            contiguous_block(
                lines,
                radius,
                base..last,
                end,
                weight,
                sums,
                destination,
                path,
            )?;
            position = last;
            continue;
        }
        if base - radius >= 0 && end - 1 + radius < lines.length {
            interior_lanes_block(
                lines,
                radius,
                base..last,
                end,
                weight,
                sums,
                destination,
                path,
            )?;
            position = last;
            continue;
        }
        sums.running[..lanes].fill([0.0; 4]);
        for step in (0..(end - base) as usize).rev() {
            for lane in 0..lanes {
                let value = lines.element(base + step as i64 - radius, lane);
                let running = &mut sums.running[lane];
                for (running, value) in running.iter_mut().zip(value) {
                    *running += value;
                }
                sums.suffix[step * lanes + lane] = *running;
            }
        }
        sums.prefix[..lanes].fill([0.0; 4]);
        for padded in end..base + width {
            for lane in 0..lanes {
                let value = lines.element(padded - radius, lane);
                for (prefix, value) in sums.prefix[lane].iter_mut().zip(value) {
                    *prefix += value;
                }
            }
        }
        for position in base..last {
            if position != base {
                for lane in 0..lanes {
                    let value = lines.element(position + radius, lane);
                    for (prefix, value) in sums.prefix[lane].iter_mut().zip(value) {
                        *prefix += value;
                    }
                }
            }
            let suffix = &sums.suffix[(position - base) as usize * lanes..][..lanes];
            for lane in 0..lanes {
                let mut mean = [0.0; 4];
                for channel in 0..4 {
                    mean[channel] = (suffix[lane][channel] + sums.prefix[lane][channel]) * weight;
                }
                destination[lines.origin + position as usize * lines.stride + lane] =
                    checked(mean, path)?;
            }
        }
        position = last;
    }
    Ok(())
}

pub(super) fn box_pass(
    surface: &mut Surface,
    domain: Rect,
    radius: usize,
    horizontal: bool,
    budget: &mut Budget,
    path: &str,
) -> Result<(), ValidationError> {
    let mut pixels = storage_overwritten(surface.pixels.len(), budget, path, &surface.pool)?;
    let bounds = surface.bounds.intersect(pixel_bounds(
        domain,
        PixelSize::new(surface.bounds.right, surface.bounds.bottom),
    ));
    clear_outside(&mut pixels, surface.bounds, bounds);
    let width = (surface.bounds.right - surface.bounds.left) as usize;
    let height = (surface.bounds.bottom - surface.bounds.top) as i64;
    let radius = radius as i64;
    let weight = 1.0 / (radius * 2 + 1) as f64;
    if radius <= 2 {
        let (start, end, lines, source_start, source_end) = if horizontal {
            (
                bounds.left,
                bounds.right,
                bounds.top..bounds.bottom,
                surface.bounds.left,
                surface.bounds.right,
            )
        } else {
            (
                bounds.top,
                bounds.bottom,
                bounds.left..bounds.right,
                surface.bounds.top,
                surface.bounds.bottom,
            )
        };
        for line in lines {
            let index = |position| {
                if horizontal {
                    surface.index(position, line)
                } else {
                    surface.index(line, position)
                }
            };
            for position in start..end {
                let first = (i64::from(position) - radius).max(i64::from(source_start));
                let last = (i64::from(position) + radius + 1).min(i64::from(source_end));
                let mut pixel = [0.0; 4];
                for tap in first..last {
                    let sampled = surface.pixels[index(tap as u32)];
                    for channel in 0..4 {
                        pixel[channel] += f64::from(sampled[channel]) * weight;
                    }
                }
                pixels[index(position)] = checked(pixel, path)?;
            }
        }
    } else if bounds.left < bounds.right && bounds.top < bounds.bottom {
        let columns = (bounds.right - bounds.left) as usize;
        let lanes = if horizontal {
            1
        } else {
            box_lanes(radius, columns)
        };
        let table = (radius * 2 + 1) as usize * lanes + 2 * lanes;
        let scratch = table as u64 * size_of::<[f64; 4]>() as u64;
        budget.bytes(scratch, path)?;
        let mut sums = BoxSums {
            suffix: Vec::new(),
            prefix: Vec::new(),
            running: Vec::new(),
        };
        for (values, count) in [
            (&mut sums.suffix, (radius * 2 + 1) as usize * lanes),
            (&mut sums.prefix, lanes),
            (&mut sums.running, lanes),
        ] {
            values.try_reserve_exact(count).map_err(|_| {
                ValidationError::limit(path, "max_bytes", "Could not reserve box filter sums")
            })?;
            values.resize(count, [0.0; 4]);
        }
        if horizontal {
            let outputs = i64::from(bounds.left - surface.bounds.left)
                ..i64::from(bounds.right - surface.bounds.left);
            for row in bounds.top..bounds.bottom {
                let lines = Lines {
                    source: &surface.pixels,
                    origin: (row - surface.bounds.top) as usize * width,
                    stride: 1,
                    lanes: 1,
                    length: width as i64,
                };
                slide_box(
                    &lines,
                    radius,
                    outputs.clone(),
                    weight,
                    &mut sums,
                    &mut pixels,
                    path,
                )?;
            }
        } else {
            let outputs = i64::from(bounds.top - surface.bounds.top)
                ..i64::from(bounds.bottom - surface.bounds.top);
            let first = (bounds.left - surface.bounds.left) as usize;
            for start in (0..columns).step_by(lanes) {
                let lines = Lines {
                    source: &surface.pixels,
                    origin: first + start,
                    stride: width,
                    lanes: lanes.min(columns - start),
                    length: height,
                };
                slide_box(
                    &lines,
                    radius,
                    outputs.clone(),
                    weight,
                    &mut sums,
                    &mut pixels,
                    path,
                )?;
            }
        }
        budget.release(scratch);
    }
    release(
        std::mem::replace(&mut surface.pixels, pixels),
        budget,
        &surface.pool,
    );
    Ok(())
}

pub(super) fn gaussian(
    surface: &mut Surface,
    domain: Rect,
    sigma: f64,
    budget: &mut Budget,
    path: &str,
    quality: motion_core::RenderQuality,
) -> Result<Rect, ValidationError> {
    let output = FilterOperation::Blur { sigma }.bounds(domain, domain);
    if sigma == 0.0 || surface.pixels.is_empty() {
        return Ok(output);
    }
    let mut working = domain;
    for radius in motion_core::filter::quality_box_radii(sigma, quality)? {
        if radius == 0 {
            continue;
        }
        let radius = radius as usize;
        let next = Rect::new(
            working.left - radius as f64,
            working.top - radius as f64,
            working.right + radius as f64,
            working.bottom + radius as f64,
        );
        if sigma * sigma > 1.0 / 3.0 {
            let work = if radius <= 2 { (radius * 2 + 1) * 2 } else { 6 };
            admitted(surface, work as u64, budget, path)?;
            box_pass(
                surface,
                Rect::new(next.left, working.top, next.right, working.bottom),
                radius,
                true,
                budget,
                path,
            )?;
            box_pass(surface, next, radius, false, budget, path)?;
            working = next;
            continue;
        }
        let count = radius * 2 + 1;
        admitted(
            surface,
            (count as u64).checked_mul(2).ok_or_else(|| {
                ValidationError::limit(path, "max_work", "Gaussian work overflow")
            })?,
            budget,
            path,
        )?;
        budget.work(count as u64, path)?;
        let bytes = (count as u64)
            .checked_mul(size_of::<f64>() as u64)
            .ok_or_else(|| {
                ValidationError::limit(path, "max_bytes", "Gaussian kernel size overflow")
            })?;
        budget.bytes(bytes, path)?;
        let mut kernel = Vec::new();
        kernel.try_reserve_exact(count).map_err(|_| {
            ValidationError::limit(path, "max_bytes", "Could not reserve Gaussian kernel")
        })?;
        kernel
            .extend((0..count).map(|index| motion_core::filter::box_weight(sigma, radius, index)));
        for horizontal in [true, false] {
            let target = if horizontal {
                Rect::new(next.left, working.top, next.right, working.bottom)
            } else {
                next
            };
            pass(surface, target, budget, path, |column, row, surface| {
                let mut pixel = [0.0; 4];
                for (index, weight) in kernel.iter().enumerate() {
                    let displacement = index as f64 - radius as f64;
                    let sampled = sample(
                        surface,
                        f64::from(column) + if horizontal { displacement } else { 0.0 },
                        f64::from(row) + if horizontal { 0.0 } else { displacement },
                    );
                    for channel in 0..4 {
                        pixel[channel] += sampled[channel] * weight;
                    }
                }
                Ok(pixel)
            })?;
        }
        budget.release(bytes);
        working = next;
    }
    Ok(output)
}

pub(super) fn shift(
    surface: &mut Surface,
    domain: Rect,
    offset: Vector2,
    budget: &mut Budget,
    path: &str,
) -> Result<Rect, ValidationError> {
    let output = FilterOperation::Offset {
        x: offset.x,
        y: offset.y,
    }
    .bounds(domain, domain);
    admitted(surface, 4, budget, path)?;
    pass(surface, output, budget, path, |column, row, surface| {
        Ok(sample(
            surface,
            f64::from(column) - offset.x,
            f64::from(row) - offset.y,
        ))
    })?;
    Ok(output)
}

pub(super) fn gaussian_xy(
    surface: &mut Surface,
    domain: Rect,
    sigmas: [f64; 2],
    budget: &mut Budget,
    path: &str,
    quality: motion_core::RenderQuality,
) -> Result<Rect, ValidationError> {
    let output = FilterOperation::BlurXY {
        sigma_x: sigmas[0],
        sigma_y: sigmas[1],
    }
    .bounds(domain, domain);
    let mut working = domain;
    for (axis, sigma) in sigmas.into_iter().enumerate() {
        for radius in motion_core::filter::quality_box_radii(sigma, quality)? {
            if radius == 0 {
                continue;
            }
            let horizontal = axis == 0;
            let radius = radius as usize;
            let next = Rect::new(
                working.left - if horizontal { radius as f64 } else { 0.0 },
                working.top - if horizontal { 0.0 } else { radius as f64 },
                working.right + if horizontal { radius as f64 } else { 0.0 },
                working.bottom + if horizontal { 0.0 } else { radius as f64 },
            );
            admitted(
                surface,
                if radius <= 2 {
                    (radius * 2 + 1) as u64
                } else {
                    6
                },
                budget,
                path,
            )?;
            if sigma * sigma > 1.0 / 3.0 {
                box_pass(surface, next, radius, horizontal, budget, path)?;
            } else {
                pass(surface, next, budget, path, |column, row, surface| {
                    let mut pixel = [0.0; 4];
                    for tap in 0..3 {
                        let displacement = tap as f64 - 1.0;
                        let sampled = sample(
                            surface,
                            f64::from(column) + if horizontal { displacement } else { 0.0 },
                            f64::from(row) + if horizontal { 0.0 } else { displacement },
                        );
                        let weight = motion_core::filter::box_weight(sigma, radius, tap);
                        for channel in 0..4 {
                            pixel[channel] += sampled[channel] * weight;
                        }
                    }
                    Ok(pixel)
                })?;
            }
            working = next;
        }
    }
    Ok(output)
}
