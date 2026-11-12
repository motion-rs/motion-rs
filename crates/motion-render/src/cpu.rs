//! Scalar linear-premultiplied composition of immutable coverage.

use crate::linear::{LinearFrame, WorkStats};
use crate::prepared::{ClipRecord, PixelRect, PreparedItem, PreparedLayer, PreparedScene};
use crate::raster::prepare::pixel_bounds;
use motion_core::budget::Budget;
use motion_core::instances::{InstanceBlend, InstanceComposite};
use motion_core::{PixelSize, RasterLimits, ValidationError};
use std::mem::size_of;

mod blending;
mod clipping;
mod filter;
mod masking;
pub(crate) mod output;
mod particles;
pub(crate) mod pool;
#[cfg(test)]
mod tests;
mod walk;

#[cfg(test)]
use crate::prepared::{PreparedFrame, RenderPrepare};

pub(crate) fn particle_compositor(blend: InstanceBlend) -> InstanceComposite {
    fn composite_with_mode<const MODE: u8>(
        destination: &mut [f32; 4],
        source: [f32; 4],
        space: motion_core::BlendSpace,
    ) -> Result<(), ValidationError> {
        use motion_core::BlendMode;
        let mode = match MODE {
            1 => BlendMode::SourceOver,
            2 => BlendMode::LinearPlus,
            3 => BlendMode::Screen,
            _ => BlendMode::Multiply,
        };
        blending::composite_in_space(destination, source, mode, space, "particles.composite")
    }
    match blend {
        InstanceBlend::SourceOver => composite_with_mode::<1>,
        InstanceBlend::Plus => composite_with_mode::<2>,
        InstanceBlend::Screen => composite_with_mode::<3>,
        InstanceBlend::Multiply => composite_with_mode::<4>,
    }
}

impl PreparedScene {
    /// Composes coverage with scalar linear-premultiplied layer blending.
    ///
    /// Independent render limits admit root pixels and every live capture before
    /// allocation. The byte peak includes this prepared frame's logical storage;
    /// copies/source lists, vector excess capacity and allocator overhead do not.
    /// Commands/cleared/blended pixels consume work even under zero opacity.
    /// Group opacity applies once, in f64 before narrowing the product to f32.
    /// Fills use source-over within captures; each layer's mode applies once at
    /// PopLayer, against its immediate parent's accumulated backdrop.
    /// Content masks render against transparent black and scale completed
    /// content before path clipping, opacity and blending. Alpha is extracted
    /// directly; luminance uses premultiplied linear Rec.709 RGB without another
    /// alpha multiplication. Inverted modes complement clamped coverage, so
    /// transparent mask regions preserve only existing source pixels.
    /// Mask captures share the live byte/work budget.
    /// Shape and rectangle regions intersect during preparation. Captures
    /// use conditional child/ancestor coverage (zero under a zero ancestor),
    /// so nested clipping does not reapply a shared antialiased boundary.
    /// Mask artwork has an independent geometric domain. Path coverage masks
    /// still multiply independently; clip_shape instead intersects filled regions.
    /// Nonfinite working results fail instead of clipping to an invented color.
    /// Returned output is transparent when there is no background or coverage.
    pub(crate) fn compose_linear(
        &self,
        limits: RasterLimits,
    ) -> Result<LinearFrame, ValidationError> {
        self.render_linear_pooled(limits, &mut pool::SurfacePool::default())
    }

    /// Composes the frame on the CPU and converts the result into `format`.
    ///
    /// The linear composition follows the rules above; its working storage and the
    /// converted buffer are admitted against `limits`. No display transform applies.
    pub fn render_buffer(
        &self,
        format: crate::PixelFormat,
        limits: RasterLimits,
    ) -> Result<crate::FrameBuffer, ValidationError> {
        crate::FrameBuffer::from_linear(&self.compose_linear(limits)?, format, limits)
    }

    /// Composes the frame into unquantized [`crate::PixelFormat::LinearRgba32Float`] output;
    /// read it with [`crate::FrameBuffer::linear_pixel`].
    pub fn render_linear(
        &self,
        limits: RasterLimits,
    ) -> Result<crate::FrameBuffer, ValidationError> {
        self.render_buffer(crate::PixelFormat::LinearRgba32Float, limits)
    }

    pub(crate) fn render_linear_pooled(
        &self,
        limits: RasterLimits,
        pool: &mut pool::SurfacePool,
    ) -> Result<LinearFrame, ValidationError> {
        self.render_linear_bounds(
            limits,
            pool,
            self.output_bounds,
            PixelRect {
                left: 0,
                top: 0,
                right: self.size.width,
                bottom: self.size.height,
            },
        )
    }

    pub(crate) fn render_linear_bounds(
        &self,
        limits: RasterLimits,
        pool: &mut pool::SurfacePool,
        output_bounds: PixelRect,
        bounds: PixelRect,
    ) -> Result<LinearFrame, ValidationError> {
        self.render_linear_bounds_counted(
            limits,
            pool,
            output_bounds,
            bounds,
            &mut WorkStats {
                peak_bytes: 0,
                work: 0,
            },
        )
    }

    pub(crate) fn render_linear_bounds_counted(
        &self,
        limits: RasterLimits,
        pool: &mut pool::SurfacePool,
        output_bounds: PixelRect,
        bounds: PixelRect,
        stats: &mut WorkStats,
    ) -> Result<LinearFrame, ValidationError> {
        self.render_linear_bounds_mode::<true>(limits, pool, output_bounds, bounds, stats)
    }

    pub(crate) fn render_linear_bounds_mode<const OPTIMIZE_LAYERS: bool>(
        &self,
        limits: RasterLimits,
        pool: &mut pool::SurfacePool,
        output_bounds: PixelRect,
        bounds: PixelRect,
        stats: &mut WorkStats,
    ) -> Result<LinearFrame, ValidationError> {
        self.render_linear_selected::<OPTIMIZE_LAYERS>(
            limits,
            pool,
            output_bounds,
            bounds,
            stats,
            None,
        )
    }

    fn render_linear_selected<const OPTIMIZE_LAYERS: bool>(
        &self,
        limits: RasterLimits,
        pool: &mut pool::SurfacePool,
        output_bounds: PixelRect,
        bounds: PixelRect,
        stats: &mut WorkStats,
        fills: Option<&[usize]>,
    ) -> Result<LinearFrame, ValidationError> {
        pool.trim(limits.max_bytes / 4);
        let mut budget = Budget::new(self.size, limits)?;
        let result = (|| {
            budget.bytes(self.stats.retained_bytes, "linear.prepared")?;
            budget.bytes(size_of::<LinearFrame>() as u64, "linear.frame")?;
            let root = Surface::new_pooled(
                bounds,
                1.0,
                motion_core::drawing::BlendMode::SourceOver,
                self.background
                    .map_or([0.0; 4], |paint| paint.linear_premultiplied),
                &mut budget,
                "linear.root",
                Some(&mut *pool),
            )?;
            let mut walk = walk::LinearWalk::new(self, pool, &mut budget, root, bounds)?;
            let mut index = 0;
            while index < self.items.len() {
                let path = format!("prepared.items[{index}]");
                if OPTIMIZE_LAYERS
                    && let [
                        PreparedItem::PushLayer { layer },
                        PreparedItem::Fill {
                            strips,
                            spans,
                            paint,
                        },
                        PreparedItem::PopLayer,
                        ..,
                    ] = &self.items[index..]
                    && layer.filters.is_none()
                    && layer.alpha_clip.is_none()
                    && layer.geometric_clip.is_none()
                    && walk
                        .surfaces
                        .last()
                        .expect("validated parent")
                        .rect_clip
                        .is_none()
                {
                    walk.budget.work(3, &path)?;
                    if fills.is_none_or(|fills| fills.binary_search(&(index + 1)).is_ok()) {
                        walk.fill_mode(strips, spans, paint, &path, Some(layer))?;
                    }
                    index += 3;
                    continue;
                }
                walk.budget.work(1, &path)?;
                let item = &self.items[index];
                match item {
                    PreparedItem::PushLayer { layer } if layer.filters.is_some() => {
                        walk.push_filtered_layer(index, layer, &path)?
                    }
                    PreparedItem::PushLayer { layer } => walk.push_layer(layer, &path)?,
                    PreparedItem::BeginMask { mode } => walk.begin_mask(mode, &path)?,
                    PreparedItem::EndMask => walk.end_mask(&path)?,
                    PreparedItem::PopLayer => walk.pop_layer(&path)?,
                    PreparedItem::ParticleInstances { batch, transform } => {
                        walk.particles(batch, transform, &path)?
                    }
                    PreparedItem::Fill {
                        strips,
                        spans,
                        paint,
                    } => {
                        if fills.is_none_or(|fills| fills.binary_search(&index).is_ok()) {
                            walk.fill(strips, spans, paint, &path)?;
                        }
                    }
                }
                index += 1;
            }
            let walk::LinearWalk {
                pool,
                budget,
                mut surfaces,
                peak_bytes,
                ..
            } = walk;
            let root = surfaces.pop().expect("validated root surface");
            let pixels = if root.bounds == output_bounds {
                root.pixels
            } else {
                let mut pixels = filter::storage_pooled(
                    output_bounds.pixels() as usize,
                    &mut *budget,
                    "linear.output",
                    pool,
                )?;
                let width = (output_bounds.right - output_bounds.left) as usize;
                for row in output_bounds.top..output_bounds.bottom {
                    let start = root.index(output_bounds.left, row);
                    let target = (row - output_bounds.top) as usize * width;
                    pixels[target..target + width]
                        .copy_from_slice(&root.pixels[start..start + width]);
                }
                pool.put(root.pixels);
                pixels
            };
            Ok(LinearFrame::from_pixels_with_stats(
                PixelSize::new(
                    output_bounds.right - output_bounds.left,
                    output_bounds.bottom - output_bounds.top,
                ),
                pixels,
                WorkStats {
                    peak_bytes: peak_bytes.max(budget.peak_bytes()),
                    work: budget.work_done(),
                },
            ))
        })();
        *stats = WorkStats {
            peak_bytes: budget.peak_bytes(),
            work: budget.work_done(),
        };
        result
    }
}

struct Surface {
    bounds: PixelRect,
    opacity: f64,
    blend_mode: motion_core::drawing::BlendMode,
    clip: Option<clipping::ClipMask>,
    rect_clip: Option<usize>,
    rectangle: Option<clipping::ClipMask>,
    mask_mode: Option<motion_core::drawing::MaskMode>,
    filter_item: Option<usize>,
    pool: pool::SurfacePool,
    pixels: Vec<[f32; 4]>,
}

impl Surface {
    #[cfg(test)]
    fn new(
        bounds: PixelRect,
        opacity: f64,
        blend_mode: motion_core::drawing::BlendMode,
        clear: [f32; 4],
        budget: &mut Budget,
        path: &str,
    ) -> Result<Self, ValidationError> {
        Self::new_pooled(bounds, opacity, blend_mode, clear, budget, path, None)
    }

    fn new_pooled(
        bounds: PixelRect,
        opacity: f64,
        blend_mode: motion_core::drawing::BlendMode,
        clear: [f32; 4],
        budget: &mut Budget,
        path: &str,
        pool: Option<&mut pool::SurfacePool>,
    ) -> Result<Self, ValidationError> {
        let count = bounds.pixels();
        budget.work(count, path)?;
        let bytes = count
            .checked_mul(size_of::<[f32; 4]>() as u64)
            .ok_or_else(|| {
                ValidationError::limit(path, "max_bytes", "pixel storage size overflow")
            })?;
        budget.bytes(bytes, path)?;
        let count = usize::try_from(count).map_err(|_| {
            ValidationError::limit(path, "max_bytes", "surface exceeds addressable storage")
        })?;
        let pool = pool.map(|pool| pool.clone()).unwrap_or_default();
        let mut pixels = pool.take(count).unwrap_or_default();
        pixels.clear();
        pixels.try_reserve_exact(count).map_err(|_| {
            ValidationError::limit(path, "max_bytes", "Could not reserve surface storage")
        })?;
        pixels.resize(count, clear);
        Ok(Self {
            bounds,
            opacity,
            blend_mode,
            clip: None,
            rect_clip: None,
            rectangle: None,
            mask_mode: None,
            filter_item: None,
            pool,
            pixels,
        })
    }

    fn index(&self, x: u32, y: u32) -> usize {
        (y - self.bounds.top) as usize * (self.bounds.right - self.bounds.left) as usize
            + (x - self.bounds.left) as usize
    }
}

fn blend(
    destination: &mut [f32; 4],
    source: [f32; 4],
    coverage: f32,
    path: &str,
) -> Result<(), ValidationError> {
    let remaining = 1.0 - source[3] * coverage;
    store_blended(
        destination,
        std::array::from_fn(|channel| {
            source[channel] * coverage + destination[channel] * remaining
        }),
        path,
    )
}

fn blend_span(
    pixels: &mut [[f32; 4]],
    source: [f32; 4],
    path: &str,
) -> Result<(), ValidationError> {
    let remaining = 1.0 - source[3];
    for destination in pixels {
        store_blended(
            destination,
            std::array::from_fn(|channel| source[channel] + destination[channel] * remaining),
            path,
        )?;
    }
    Ok(())
}

fn store_blended(
    destination: &mut [f32; 4],
    result: [f32; 4],
    path: &str,
) -> Result<(), ValidationError> {
    if result.iter().any(|value| !value.is_finite()) {
        return Err(ValidationError::invalid(
            path,
            "color",
            "linear composition overflowed finite f32 working storage",
        ));
    }
    *destination = result;
    Ok(())
}
