//! Brush material baking: paper textures, paint sampling and tiled distance-field material.

use super::super::{BrushStroke, Prepared, invalid};
use super::geometry::{outline, rotated, samples};
use crate::ImageAsset;
use motion_core::{GradientExtend, ImagePattern, Paint, Point, Rect, Transform, ValidationError};
use std::sync::Arc;

/// Segments per culling block in the material distance search.
const MATERIAL_BLOCK: usize = 16;

/// Repeating immutable paper height map sampled in local design coordinates.
#[derive(Clone, Debug)]
pub struct Paper {
    /// Already resolved image; no implicit file loading during rendering.
    pub texture: Arc<ImageAsset>,
    /// Local design units per texel.
    pub grain_scale: f64,
    /// Coverage attenuation by the paper height, in 0..=1.
    pub absorbency: f64,
}

fn image_sample(pattern: &ImagePattern, point: Point) -> Result<[f64; 4], ValidationError> {
    let inverse = pattern
        .transform
        .inverse()
        .ok_or_else(|| invalid("paint", "Noninvertible paint transform"))?;
    let point = inverse.apply(point);
    let width = pattern.image.width() as usize;
    let height = pattern.image.height() as usize;
    if pattern.decal
        && (point.x < 0.0 || point.y < 0.0 || point.x >= width as f64 || point.y >= height as f64)
    {
        return Ok([0.0; 4]);
    }
    let horizontal = point.x - 0.5;
    let vertical = point.y - 0.5;
    let column = horizontal.floor();
    let row = vertical.floor();
    let fraction = [horizontal - column, vertical - row];
    let address = |index: i64, length: usize| match pattern.extend {
        GradientExtend::Pad => index.clamp(0, length as i64 - 1) as usize,
        GradientExtend::Repeat => index.rem_euclid(length as i64) as usize,
        GradientExtend::Reflect => {
            let index = index.rem_euclid(length as i64 * 2) as usize;
            if index >= length {
                length * 2 - index - 1
            } else {
                index
            }
        }
    };
    let mut result = [0.0; 4];
    for vertical in 0..2 {
        for horizontal in 0..2 {
            let weight = (if horizontal == 0 {
                1.0 - fraction[0]
            } else {
                fraction[0]
            }) * (if vertical == 0 {
                1.0 - fraction[1]
            } else {
                fraction[1]
            });
            let texel = pattern.image.texel(
                0,
                address(column as i64 + horizontal, width),
                address(row as i64 + vertical, height),
            );
            for channel in 0..4 {
                result[channel] += f64::from(texel[channel]) * weight;
            }
        }
    }
    Ok(result)
}

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
        _ => return Err(invalid("paint", "Unsupported brush paint")),
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

pub(super) fn material(
    stroke: &BrushStroke,
    bounds: Rect,
) -> Result<Vec<Prepared>, ValidationError> {
    let options = &stroke.advanced;
    let groups = samples(stroke, 1.0)?;
    let path = Arc::new(outline(stroke, &groups)?);
    let segments: Vec<_> = groups
        .iter()
        .flat_map(|group| group.windows(2).map(|pair| (pair[0], pair[1])))
        .collect();
    // Boxes widened by the widest radius bound the distance from below, so blocks of
    // far segments are skipped without changing the nearest result or the overlap count.
    let boxes: Vec<_> = segments
        .iter()
        .map(|(first, second)| {
            (
                [
                    first.point.x.min(second.point.x),
                    first.point.y.min(second.point.y),
                ],
                [
                    first.point.x.max(second.point.x),
                    first.point.y.max(second.point.y),
                ],
                first.radius.max(second.radius),
            )
        })
        .collect();
    let blocks: Vec<_> = boxes
        .chunks(MATERIAL_BLOCK)
        .map(|chunk| {
            chunk.iter().skip(1).fold(chunk[0], |total, item| {
                (
                    [total.0[0].min(item.0[0]), total.0[1].min(item.0[1])],
                    [total.1[0].max(item.1[0]), total.1[1].max(item.1[1])],
                    total.2.max(item.2),
                )
            })
        })
        .collect();
    let length = stroke.path.try_length()?;
    let width = bounds.width().ceil().max(1.0);
    let height = bounds.height().ceil().max(1.0);
    if width > 16384.0
        || height > 16384.0
        || width * height > 16_777_216.0
        || (width * height) as u64 * segments.len() as u64 > stroke.material_work_limit
    {
        return Err(invalid(
            "material",
            "Extended material exceeds pixel or total evaluation allowance",
        ));
    }
    let width = width as u32;
    let height = height as u32;
    let columns = width.div_ceil(options.tile_size);
    let rows = height.div_ceil(options.tile_size);
    if u64::from(columns) * u64::from(rows) > 256 {
        return Err(invalid("material_tiles", "At most 256 material tiles"));
    }
    let paint = options.paint.clone().unwrap_or_else(|| stroke.color.into());
    let ink = stroke.ink.unwrap_or_default();
    let paper = options.paper.as_ref().map(|paper| {
        ImagePattern::from_asset(paper.texture.clone())
            .transform(Transform::scale(paper.grain_scale))
            .extend(GradientExtend::Repeat)
    });
    let mut tiles = Vec::new();
    for row in 0..rows {
        for column in 0..columns {
            let first = column * options.tile_size;
            let top = row * options.tile_size;
            let tile_width = (width - first).min(options.tile_size);
            let tile_height = (height - top).min(options.tile_size);
            let mut pixels = vec![[0.0; 4]; tile_width as usize * tile_height as usize];
            let evaluate = |start: usize, pixels: &mut [[f64; 4]]| -> Result<(), ValidationError> {
                for (offset, pixel) in pixels.iter_mut().enumerate() {
                    let index = start + offset;
                    let point = Point::new(
                        bounds.left + f64::from(first) + (index % tile_width as usize) as f64 + 0.5,
                        bounds.top + f64::from(top) + (index / tile_width as usize) as f64 + 0.5,
                    );
                    let lower_bound = |bounds: &([f64; 2], [f64; 2], f64)| {
                        let horizontal =
                            (bounds.0[0] - point.x).max(point.x - bounds.1[0]).max(0.0);
                        let vertical = (bounds.0[1] - point.y).max(point.y - bounds.1[1]).max(0.0);
                        libm::hypot(horizontal, vertical) - bounds.2
                    };
                    let measure = |index: usize| {
                        let (first, second) = &segments[index];
                        let delta = second.point - first.point;
                        let square = delta.x * delta.x + delta.y * delta.y;
                        let offset = point - first.point;
                        let amount = if square > 0.0 {
                            ((offset.x * delta.x + offset.y * delta.y) / square).clamp(0.0, 1.0)
                        } else {
                            0.0
                        };
                        let center = first.point + delta * amount;
                        let offset = point - center;
                        let angle = first.angle + (second.angle - first.angle) * amount;
                        let local = rotated(offset.x, offset.y, -angle);
                        let radius = first.radius + (second.radius - first.radius) * amount;
                        let distance = libm::sqrt(
                            local.x * local.x + local.y * local.y / (stroke.aspect * stroke.aspect),
                        ) - radius;
                        (
                            distance,
                            first.arc + (second.arc - first.arc) * amount,
                            offset.x * first.tangent.y - offset.y * first.tangent.x,
                            radius,
                        )
                    };
                    // The best block seeds the nearest distance, so ties still resolve to the
                    // lowest segment index and skipping becomes effective immediately.
                    let mut nearest = (f64::INFINITY, 0.0, 0.0, 0.0);
                    let mut nearest_index = usize::MAX;
                    if let Some((block, _)) = blocks
                        .iter()
                        .enumerate()
                        .map(|(block, bounds)| (block, lower_bound(bounds)))
                        .min_by(|left, right| left.1.total_cmp(&right.1))
                    {
                        for index in block * MATERIAL_BLOCK
                            ..((block + 1) * MATERIAL_BLOCK).min(segments.len())
                        {
                            let candidate = measure(index);
                            if candidate.0 < nearest.0 {
                                nearest = candidate;
                                nearest_index = index;
                            }
                        }
                    }
                    let mut overlaps = 0usize;
                    let mut previous_overlap = None;
                    for (block, bounds) in blocks.iter().enumerate() {
                        let lower = lower_bound(bounds);
                        if lower > 1e-6 && lower > nearest.0 + 1e-6 {
                            continue;
                        }
                        for index in block * MATERIAL_BLOCK
                            ..((block + 1) * MATERIAL_BLOCK).min(segments.len())
                        {
                            let candidate = measure(index);
                            if candidate.0 < 0.0
                                && previous_overlap.is_none_or(|previous| index > previous + 1)
                            {
                                overlaps += 1;
                                previous_overlap = Some(index);
                            }
                            if candidate.0 < nearest.0
                                || (candidate.0 == nearest.0 && index < nearest_index)
                            {
                                nearest = candidate;
                                nearest_index = index;
                            }
                        }
                    }
                    let paper_point = options.paper_transform.apply(point);
                    let material = ink.sample((point.x, point.y), (paper_point.x, paper_point.y));
                    let load = stroke.load[0]
                        + (stroke.load[1] - stroke.load[0])
                            * if length > 0.0 {
                                nearest.1 / length
                            } else {
                                0.0
                            };
                    let mut alpha = material
                        .shade(
                            material.coverage(nearest.0, 1.0),
                            nearest.0,
                            (nearest.1, nearest.2 * options.anisotropy),
                            load,
                        )
                        .clamp(0.0, 1.0);
                    if let (Some(paper), Some(parameters)) = (&paper, &options.paper) {
                        let sample = image_sample(paper, paper_point)?;
                        let height = if sample[3] > 0.0 {
                            (sample[0] * 0.2126 + sample[1] * 0.7152 + sample[2] * 0.0722)
                                / sample[3]
                        } else {
                            0.0
                        };
                        alpha *= 1.0 - parameters.absorbency * height.clamp(0.0, 1.0);
                    }
                    if overlaps > 1 && options.build_up > 0.0 {
                        alpha = 1.0
                            - libm::pow(
                                1.0 - alpha,
                                1.0 + options.build_up * (overlaps - 1) as f64,
                            );
                    }
                    let mut color = paint_sample(&paint, point, stroke.path.bounding_box())?;
                    let rim = (1.0 - nearest.0.abs() / 2.0).clamp(0.0, 1.0);
                    let darken = 1.0
                        - options.edge_darkening * rim
                        - options.build_up * (1.0 - 1.0 / overlaps.max(1) as f64) * 0.5;
                    for channel in &mut color[..3] {
                        *channel *= darken;
                    }
                    if options.height > 0.0 {
                        let height = options.height
                            * libm::sqrt((-nearest.0 / nearest.3.max(1e-12)).clamp(0.0, 1.0))
                            * load
                            * (1.0 + options.build_up * overlaps.saturating_sub(1) as f64);
                        for channel in &mut color[..3] {
                            *channel = motion_core::color::linear_to_srgb(height);
                        }
                    }
                    color[3] *= alpha;
                    *pixel = color;
                }
                Ok(())
            };
            if options.parallel {
                let threads = std::thread::available_parallelism()
                    .map_or(1, usize::from)
                    .min(8);
                let chunk = pixels.len().div_ceil(threads).max(1);
                std::thread::scope(|scope| {
                    let handles: Vec<_> = pixels
                        .chunks_mut(chunk)
                        .enumerate()
                        .map(|(index, pixels)| {
                            let evaluate = &evaluate;
                            scope.spawn(move || evaluate(index * chunk, pixels))
                        })
                        .collect();
                    for handle in handles {
                        handle.join().map_err(|_| {
                            invalid("material_thread", "Material worker panicked")
                        })??;
                    }
                    Ok::<_, ValidationError>(())
                })?;
            } else {
                evaluate(0, &mut pixels)?;
            }
            let image = ImageAsset::from_rgba_float(tile_width, tile_height, pixels)
                .map_err(|error| invalid("material", &error.to_string()))?;
            let mut pattern = ImagePattern::from_asset(Arc::new(image))
                .transform(Transform::translate(
                    bounds.left + f64::from(first),
                    bounds.top + f64::from(top),
                ))
                .extend(GradientExtend::Pad)
                .sampling(motion_core::Sampling::Bilinear);
            pattern.decal = true;
            tiles.push(Prepared {
                path: path.clone(),
                paint: pattern.into(),
                bounds,
            });
        }
    }
    Ok(tiles)
}
