//! Bounded scalar reference for source-only linear-light filters.
//!
//! This module owns the shared pass, sampling, scratch and budget helpers and applies each
//! filter in order; the blur, optics and color families live in submodules.

use super::*;
use blur::{gaussian, gaussian_xy, shift};
use color::{luminance, noise, premultiplied, straight, threshold};
use motion_core::{Filter, FilterOperation, PixelSize, Rect, ValidationError, Vector2};
use optics::{ZeroFootprint, channels, optical_pixel};

mod bloom;
mod blur;
mod color;
mod optics;
#[cfg(test)]
mod tests;

pub(crate) fn storage_pooled(
    count: usize,
    budget: &mut Budget,
    path: &str,
    pool: &pool::SurfacePool,
) -> Result<Vec<[f32; 4]>, ValidationError> {
    budget.work(count as u64, path)?;
    budget.bytes(count as u64 * size_of::<[f32; 4]>() as u64, path)?;
    let mut pixels = pool.take(count).unwrap_or_default();
    pixels.clear();
    pixels.try_reserve_exact(count).map_err(|_| {
        ValidationError::limit(path, "max_bytes", "Could not reserve filter scratch")
    })?;
    pixels.resize(count, [0.0; 4]);
    Ok(pixels)
}

/// Like `storage_pooled`, but a reused buffer keeps its previous contents: the caller must
/// write or clear every pixel.
fn storage_overwritten(
    count: usize,
    budget: &mut Budget,
    path: &str,
    pool: &pool::SurfacePool,
) -> Result<Vec<[f32; 4]>, ValidationError> {
    budget.work(count as u64, path)?;
    budget.bytes(count as u64 * size_of::<[f32; 4]>() as u64, path)?;
    let mut pixels = pool.take(count).unwrap_or_default();
    if pixels.len() != count {
        pixels.clear();
        pixels.try_reserve_exact(count).map_err(|_| {
            ValidationError::limit(path, "max_bytes", "Could not reserve filter scratch")
        })?;
        pixels.resize(count, [0.0; 4]);
    }
    Ok(pixels)
}

/// Zeroes the pixels of a buffer laid out over `outer` that lie outside `inner`.
fn clear_outside(pixels: &mut [[f32; 4]], outer: PixelRect, inner: PixelRect) {
    let width = (outer.right - outer.left) as usize;
    if inner.left >= inner.right || inner.top >= inner.bottom {
        pixels.fill([0.0; 4]);
        return;
    }
    let row = |y: u32| (y - outer.top) as usize * width;
    pixels[..row(inner.top)].fill([0.0; 4]);
    pixels[row(inner.bottom.min(outer.bottom))..].fill([0.0; 4]);
    for y in inner.top..inner.bottom {
        let start = row(y);
        pixels[start..start + (inner.left - outer.left) as usize].fill([0.0; 4]);
        pixels[start + (inner.right - outer.left) as usize..start + width].fill([0.0; 4]);
    }
}

fn release(pixels: Vec<[f32; 4]>, budget: &mut Budget, pool: &pool::SurfacePool) {
    budget.release(pixels.len() as u64 * size_of::<[f32; 4]>() as u64);
    pool.put(pixels);
}

/// Narrows a filtered pixel to working storage. Fully transparent pixels drop their color.
///
/// The finiteness tests use `&` so the common case is a straight-line sequence the compiler can
/// keep in registers; only failures take the (cold) error path.
#[inline]
fn checked(pixel: [f64; 4], path: &str) -> Result<[f32; 4], ValidationError> {
    if !(pixel[0].is_finite() & pixel[1].is_finite() & pixel[2].is_finite() & pixel[3].is_finite())
    {
        return Err(unrepresentable(
            path,
            "Filter arithmetic must remain finite",
        ));
    }
    if pixel[3] == 0.0 {
        return Ok([0.0; 4]);
    }
    let result = [
        pixel[0] as f32,
        pixel[1] as f32,
        pixel[2] as f32,
        pixel[3] as f32,
    ];
    if !(result[0].is_finite()
        & result[1].is_finite()
        & result[2].is_finite()
        & result[3].is_finite())
    {
        return Err(unrepresentable(
            path,
            "Filter working color must remain finite and representable",
        ));
    }
    Ok(result)
}

#[cold]
fn unrepresentable(path: &str, message: &'static str) -> ValidationError {
    ValidationError::invalid(path, "filters", message)
}

fn pass(
    surface: &mut Surface,
    domain: Rect,
    budget: &mut Budget,
    path: &str,
    mut operation: impl FnMut(u32, u32, &Surface) -> Result<[f64; 4], ValidationError>,
) -> Result<(), ValidationError> {
    let mut pixels = storage_pooled(surface.pixels.len(), budget, path, &surface.pool)?;
    let bounds = surface.bounds.intersect(pixel_bounds(
        domain,
        PixelSize::new(surface.bounds.right, surface.bounds.bottom),
    ));
    for row in bounds.top..bounds.bottom {
        for column in bounds.left..bounds.right {
            pixels[surface.index(column, row)] = checked(operation(column, row, surface)?, path)?;
        }
    }
    release(
        std::mem::replace(&mut surface.pixels, pixels),
        budget,
        &surface.pool,
    );
    Ok(())
}

fn sample(surface: &Surface, x: f64, y: f64) -> [f64; 4] {
    let left = x.floor();
    let top = y.floor();
    let bounds = surface.bounds;
    // Interior samples skip the per-tap bounds tests but keep the tap order and the
    // `value * factor_x * factor_y` association of the general path, so results are identical.
    if left >= f64::from(bounds.left)
        && left + 1.0 < f64::from(bounds.right)
        && top >= f64::from(bounds.top)
        && top + 1.0 < f64::from(bounds.bottom)
    {
        let first = surface.index(left as u32, top as u32);
        let below = first + (bounds.right - bounds.left) as usize;
        let mut result = [0.0; 4];
        for (index, factor_x, factor_y) in [
            (first, 1.0 - (x - left), 1.0 - (y - top)),
            (below, 1.0 - (x - left), y - top),
            (first + 1, x - left, 1.0 - (y - top)),
            (below + 1, x - left, y - top),
        ] {
            let pixel = surface.pixels[index];
            for channel in 0..4 {
                result[channel] += f64::from(pixel[channel]) * factor_x * factor_y;
            }
        }
        return result;
    }
    let mut result = [0.0; 4];
    for (column, factor_x) in [(left, 1.0 - (x - left)), (left + 1.0, x - left)] {
        for (row, factor_y) in [(top, 1.0 - (y - top)), (top + 1.0, y - top)] {
            if column >= f64::from(surface.bounds.left)
                && column < f64::from(surface.bounds.right)
                && row >= f64::from(surface.bounds.top)
                && row < f64::from(surface.bounds.bottom)
            {
                let pixel = surface.pixels[surface.index(column as u32, row as u32)];
                for channel in 0..4 {
                    result[channel] += f64::from(pixel[channel]) * factor_x * factor_y;
                }
            }
        }
    }
    result
}

fn admitted(
    surface: &Surface,
    factor: u64,
    budget: &mut Budget,
    path: &str,
) -> Result<(), ValidationError> {
    budget.work(
        (surface.pixels.len() as u64)
            .checked_mul(factor)
            .ok_or_else(|| ValidationError::limit(path, "max_work", "Filter work overflow"))?,
        path,
    )
}

pub(crate) fn apply(
    surface: &mut Surface,
    mut domain: Rect,
    filters: &[Filter],
    origin: PixelRect,
    budget: &mut Budget,
    path: &str,
    frame: &FrameRecords,
) -> Result<(), ValidationError> {
    let blend_space = frame.blend_space;
    if domain.right <= domain.left || domain.bottom <= domain.top {
        return Ok(());
    }
    domain = motion_core::filter::raster_bounds(domain);
    for filter in filters {
        budget.work(1, path)?;
        let needs_source = match filter {
            Filter::DropShadow { .. } | Filter::Glow { .. } | Filter::Bloom { .. } => true,
            Filter::Optics(optics) => optics.streak_sigmas().is_some(),
            Filter::Custom(operations) => operations
                .iter()
                .any(|operation| matches!(operation, FilterOperation::CompositeWithSource { .. })),
            _ => false,
        };
        let original = if needs_source {
            let mut original = storage_pooled(surface.pixels.len(), budget, path, &surface.pool)?;
            original.copy_from_slice(&surface.pixels);
            Some(original)
        } else {
            None
        };
        let source_bounds = domain;
        match filter {
            Filter::Optics(
                optics @ motion_core::Optics::Anamorphic {
                    threshold: cutoff,
                    tint,
                    intensity,
                    ..
                },
            ) => {
                let sigmas = optics.streak_sigmas().expect("anamorphic sigmas");
                threshold(surface, domain, *cutoff, budget, path)?;
                domain = gaussian_xy(surface, domain, sigmas, budget, path, frame.quality)?;
                let color = tint.to_linear_premultiplied();
                let scale = [color[0], color[1], color[2]].map(|channel| {
                    if color[3] > 0.0 {
                        channel / color[3]
                    } else {
                        0.0
                    }
                });
                admitted(surface, 1, budget, path)?;
                pass(surface, domain, budget, path, |column, row, surface| {
                    let index = surface.index(column, row);
                    let original = original.as_ref().expect("streak source");
                    let streak = surface.pixels[index];
                    if streak == [0.0; 4] {
                        return Ok(original[index].map(f64::from));
                    }
                    let glow = [
                        f64::from(streak[0]) * scale[0] * intensity,
                        f64::from(streak[1]) * scale[1] * intensity,
                        f64::from(streak[2]) * scale[2] * intensity,
                        (f64::from(streak[3]) * intensity).clamp(0.0, 1.0),
                    ];
                    let mut destination = original[index];
                    blending::composite_in_space(
                        &mut destination,
                        checked(glow, path)?,
                        motion_core::drawing::BlendMode::Plus,
                        blend_space,
                        path,
                    )?;
                    Ok(destination.map(f64::from))
                })?;
            }
            Filter::Optics(optics) => {
                let (values, kernel) = optics.parameters(domain);
                let output = optics.bounds(domain);
                admitted(
                    surface,
                    match optics {
                        motion_core::Optics::Bokeh { .. } => values[3] as u64 * 4,
                        motion_core::Optics::LensFlare { ghosts, .. } => u64::from(*ghosts) * 4,
                        motion_core::Optics::RadialBlur { samples, .. } => u64::from(*samples) * 4,
                        motion_core::Optics::Relight { .. } => 40,
                        _ => 12,
                    },
                    budget,
                    path,
                )?;
                let zero = match optics {
                    motion_core::Optics::Bokeh { radius, .. } if *radius != 0.0 => {
                        ZeroFootprint::new(surface, &kernel, budget, path)?
                    }
                    _ => None,
                };
                pass(surface, output, budget, path, |column, row, surface| {
                    if zero
                        .as_ref()
                        .is_some_and(|zero| zero.contains(surface, column, row))
                    {
                        return Ok([0.0; 4]);
                    }
                    Ok(optical_pixel(
                        surface,
                        [f64::from(column), f64::from(row)],
                        &values,
                        &kernel,
                    ))
                })?;
                if let Some(zero) = zero {
                    zero.release(budget);
                }
                domain = output;
            }
            Filter::Cdl(cdl) => {
                pass(surface, domain, budget, path, |column, row, surface| {
                    let mut pixel = straight(surface.pixels[surface.index(column, row)]);
                    let rgb = cdl.apply([pixel[0] as f32, pixel[1] as f32, pixel[2] as f32]);
                    for channel in 0..3 {
                        pixel[channel] = f64::from(rgb[channel]);
                    }
                    Ok(premultiplied(pixel))
                })?;
            }
            Filter::Lut { lut, input } => {
                admitted(
                    surface,
                    4 + u64::from(lut.shaper().is_some()) * 6,
                    budget,
                    path,
                )?;
                pass(surface, domain, budget, path, |column, row, surface| {
                    let mut pixel = straight(surface.pixels[surface.index(column, row)]);
                    let rgb =
                        lut.apply([pixel[0] as f32, pixel[1] as f32, pixel[2] as f32], *input);
                    for channel in 0..3 {
                        pixel[channel] = f64::from(rgb[channel]);
                    }
                    Ok(premultiplied(pixel))
                })?;
            }
            Filter::Bloom {
                threshold: cutoff,
                knee,
                radius,
                levels,
                falloff,
                intensity,
                tint,
            } => {
                pass(surface, domain, budget, path, |column, row, surface| {
                    let pixel = surface.pixels[surface.index(column, row)];
                    if pixel == [0.0; 4] {
                        return Ok([0.0; 4]);
                    }
                    let luma = luminance(straight(pixel)).max(0.0);
                    let limited = luma.min(1000.0);
                    let soft = (limited - cutoff + knee).clamp(0.0, 2.0 * knee);
                    let contribution = (soft * soft / (4.0 * knee + 1e-6))
                        .max(limited - cutoff)
                        .max(0.0)
                        / luma.max(1e-6);
                    Ok(pixel.map(|value| f64::from(value) * contribution))
                })?;
                let mut accumulated =
                    storage_pooled(surface.pixels.len(), budget, path, &surface.pool)?;
                if let Some(original) = &original {
                    accumulated.copy_from_slice(original);
                }
                let weights: [f64; 8] = std::array::from_fn(|level| {
                    if level < usize::from(*levels) {
                        libm::pow(*falloff, level as f64)
                    } else {
                        0.0
                    }
                });
                let total = weights.iter().sum::<f64>();
                let factors = weights.map(|weight| weight / total * intensity);
                let glow = bloom::pyramid(
                    surface,
                    source_bounds,
                    *radius,
                    &factors[..usize::from(*levels)],
                    budget,
                    path,
                    frame.quality,
                )?;
                let output = motion_core::filter::union(source_bounds, glow);
                admitted(surface, 1, budget, path)?;
                let color = tint.to_linear_premultiplied();
                let linear = blend_space == motion_core::drawing::BlendSpace::LinearRgb;
                for (target, pixel) in accumulated.iter_mut().zip(&surface.pixels) {
                    // A transparent pixel scales to a zero glow, which composites to nothing.
                    if *pixel == [0.0; 4] {
                        continue;
                    }
                    let mut glow = checked(
                        std::array::from_fn(|channel| f64::from(pixel[channel]) * color[channel]),
                        path,
                    )?;
                    glow[3] = glow[3].clamp(0.0, 1.0);
                    if linear {
                        blending::plus_linear(target, glow, path)?;
                    } else {
                        blending::composite_in_space(
                            target,
                            glow,
                            motion_core::drawing::BlendMode::Plus,
                            blend_space,
                            path,
                        )?;
                    }
                }
                release(
                    std::mem::replace(&mut surface.pixels, accumulated),
                    budget,
                    &surface.pool,
                );
                domain = output;
            }
            Filter::Blur { sigma } => {
                domain = gaussian(surface, domain, *sigma, budget, path, frame.quality)?
            }
            Filter::DropShadow {
                offset,
                sigma,
                color,
            } => {
                let color = color.to_linear_premultiplied();
                pass(surface, domain, budget, path, |column, row, surface| {
                    Ok(color.map(|channel| {
                        channel * f64::from(surface.pixels[surface.index(column, row)][3])
                    }))
                })?;
                domain = gaussian(surface, domain, *sigma, budget, path, frame.quality)?;
                domain = shift(surface, domain, *offset, budget, path)?;
                domain = motion_core::filter::union(domain, source_bounds);
                pass(surface, domain, budget, path, |column, row, surface| {
                    let index = surface.index(column, row);
                    let mut destination = surface.pixels[index];
                    blending::composite_in_space(
                        &mut destination,
                        original.as_ref().expect("shadow source")[index],
                        motion_core::drawing::BlendMode::SourceOver,
                        blend_space,
                        path,
                    )?;
                    Ok(destination.map(f64::from))
                })?;
            }
            Filter::Glow {
                threshold: cutoff,
                sigma,
                intensity,
            } => {
                threshold(surface, domain, *cutoff, budget, path)?;
                domain = gaussian(surface, domain, *sigma, budget, path, frame.quality)?;
                pass(surface, domain, budget, path, |column, row, surface| {
                    let index = surface.index(column, row);
                    let original = original.as_ref().expect("glow source");
                    // No glow leaves the source pixel as it was; skip the scaling and blending.
                    if surface.pixels[index] == [0.0; 4] {
                        return Ok(original[index].map(f64::from));
                    }
                    let mut glow = surface.pixels[index].map(|value| f64::from(value) * intensity);
                    glow[3] = glow[3].clamp(0.0, 1.0);
                    let mut destination = original[index];
                    blending::composite_in_space(
                        &mut destination,
                        checked(glow, path)?,
                        motion_core::drawing::BlendMode::Plus,
                        blend_space,
                        path,
                    )?;
                    Ok(destination.map(f64::from))
                })?;
            }
            Filter::ColorAdjust {
                exposure,
                contrast,
                saturation,
                hue_degrees,
            } => {
                let radians = hue_degrees.rem_euclid(360.0).to_radians();
                let sine = libm::sin(radians) / libm::sqrt(3.0);
                let cosine = libm::cos(radians);
                let gain = libm::exp2(*exposure);
                pass(surface, domain, budget, path, |column, row, surface| {
                    let mut pixel = straight(surface.pixels[surface.index(column, row)]);
                    for channel in &mut pixel[..3] {
                        *channel = (*channel * gain - 0.18) * contrast + 0.18;
                    }
                    let luminance = luminance(pixel);
                    for channel in &mut pixel[..3] {
                        *channel = luminance + (*channel - luminance) * saturation;
                    }
                    let gray = (pixel[0] + pixel[1] + pixel[2]) / 3.0;
                    let rgb = [pixel[0], pixel[1], pixel[2]];
                    pixel[0] = rgb[0] * cosine + (rgb[2] - rgb[1]) * sine + gray * (1.0 - cosine);
                    pixel[1] = rgb[1] * cosine + (rgb[0] - rgb[2]) * sine + gray * (1.0 - cosine);
                    pixel[2] = rgb[2] * cosine + (rgb[1] - rgb[0]) * sine + gray * (1.0 - cosine);
                    Ok(premultiplied(pixel))
                })?;
            }
            Filter::Custom(operations) => {
                for operation in operations {
                    budget.work(1, path)?;
                    match operation {
                        FilterOperation::BlurXY { sigma_x, sigma_y } => {
                            domain = gaussian_xy(
                                surface,
                                domain,
                                [*sigma_x, *sigma_y],
                                budget,
                                path,
                                frame.quality,
                            )?;
                        }
                        FilterOperation::Transform(transform) => {
                            let inverse = transform.inverse().ok_or_else(|| {
                                unrepresentable(path, "Filter transform must be invertible")
                            })?;
                            domain = operation.bounds(domain, source_bounds);
                            admitted(surface, 4, budget, path)?;
                            pass(surface, domain, budget, path, |column, row, surface| {
                                let point = inverse.apply(motion_core::Point::new(
                                    f64::from(column) + 0.5,
                                    f64::from(row) + 0.5,
                                ));
                                Ok(sample(surface, point.x - 0.5, point.y - 0.5))
                            })?;
                        }
                        FilterOperation::Blur { sigma } => {
                            domain = gaussian(surface, domain, *sigma, budget, path, frame.quality)?
                        }
                        FilterOperation::Offset { x, y } => {
                            domain = shift(surface, domain, Vector2::new(*x, *y), budget, path)?
                        }
                        FilterOperation::ColorMatrix(matrix) => {
                            pass(surface, domain, budget, path, |column, row, surface| {
                                let input = straight(surface.pixels[surface.index(column, row)]);
                                let mut pixel = [0.0; 4];
                                for channel in 0..4 {
                                    pixel[channel] = matrix[channel * 5 + 4];
                                    for component in 0..4 {
                                        pixel[channel] +=
                                            matrix[channel * 5 + component] * input[component];
                                    }
                                }
                                Ok(premultiplied(pixel))
                            })?
                        }
                        FilterOperation::Threshold { value } => {
                            threshold(surface, domain, *value, budget, path)?
                        }
                        FilterOperation::Noise { seed, amplitude } => {
                            noise(surface, domain, *seed, *amplitude, origin, budget, path)?
                        }
                        FilterOperation::ChannelOffset { red, green, blue } => {
                            domain = channels(surface, domain, [*red, *green, *blue], budget, path)?
                        }
                        FilterOperation::CompositeWithSource { mode } => {
                            domain = motion_core::filter::union(domain, source_bounds);
                            pass(surface, domain, budget, path, |column, row, surface| {
                                let index = surface.index(column, row);
                                let mut destination =
                                    original.as_ref().expect("filter source")[index];
                                blending::composite_in_space(
                                    &mut destination,
                                    surface.pixels[index],
                                    *mode,
                                    blend_space,
                                    path,
                                )?;
                                Ok(destination.map(f64::from))
                            })?;
                        }
                    }
                }
            }
        }
        if let Some(original) = original {
            release(original, budget, &surface.pool);
        }
    }
    Ok(())
}
