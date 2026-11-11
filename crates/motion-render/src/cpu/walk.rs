//! Item-by-item linear composition: one method per prepared item over a stack of surfaces.

use super::*;

/// The live state of one linear render: budget, surface stack and peak storage.
pub(super) struct LinearWalk<'a> {
    pub(super) frame: &'a PreparedScene,
    pub(super) pool: &'a mut pool::SurfacePool,
    pub(super) budget: &'a mut Budget,
    pub(super) surfaces: Vec<Surface>,
    pub(super) peak_bytes: u64,
    bounds: PixelRect,
}

impl<'a> LinearWalk<'a> {
    pub(super) fn new(
        frame: &'a PreparedScene,
        pool: &'a mut pool::SurfacePool,
        budget: &'a mut Budget,
        root: Surface,
        bounds: PixelRect,
    ) -> Result<Self, ValidationError> {
        let mut walk = Self {
            frame,
            pool,
            budget,
            surfaces: Vec::new(),
            peak_bytes: 0,
            bounds,
        };
        walk.budget.push(&mut walk.surfaces, root, "linear.root")?;
        walk.peak_bytes = walk.budget.live_bytes();
        Ok(walk)
    }

    pub(super) fn push_filtered_layer(
        &mut self,
        index: usize,
        descriptor: &PreparedLayer,
        path: &str,
    ) -> Result<(), ValidationError> {
        let mut layer = Surface::new_pooled(
            descriptor.bounds.intersect(self.bounds),
            1.0,
            motion_core::drawing::BlendMode::SourceOver,
            [0.0; 4],
            self.budget,
            path,
            Some(&mut *self.pool),
        )?;
        layer.filter_item = Some(index);
        self.budget.push(&mut self.surfaces, layer, path)?;
        Ok(())
    }

    pub(super) fn push_layer(
        &mut self,
        descriptor: &PreparedLayer,
        path: &str,
    ) -> Result<(), ValidationError> {
        let frame = self.frame;
        let crate::prepared::PreparedLayer {
            opacity,
            bounds,
            blend_mode,
            alpha_clip: clip,
            ..
        } = descriptor;
        let parent = self.surfaces.last().expect("validated root surface");
        let rect_clip = descriptor.geometric_clip.or(parent.rect_clip);
        let mut bounds = bounds.intersect(parent.bounds);
        if let Some(index) = rect_clip {
            bounds = bounds.intersect(frame.clips[index].bounds);
        }
        if let Some(index) = clip {
            bounds = bounds.intersect(frame.clips[*index].bounds);
        }
        let mut layer = Surface::new_pooled(
            bounds,
            *opacity,
            *blend_mode,
            [0.0; 4],
            self.budget,
            path,
            Some(&mut *self.pool),
        )?;
        if let Some(index) = clip {
            layer.clip = Some(clipping::ClipMask::new(
                frame,
                &frame.clips[*index],
                bounds,
                self.budget,
                path,
            )?);
        }
        layer.rect_clip = rect_clip;
        if let Some(index) = rect_clip {
            layer.rectangle = Some(clipping::ClipMask::new(
                frame,
                &frame.clips[index],
                bounds,
                self.budget,
                path,
            )?);
        }
        self.budget.push(&mut self.surfaces, layer, path)?;
        self.peak_bytes = self.peak_bytes.max(self.budget.live_bytes());
        Ok(())
    }

    pub(super) fn begin_mask(
        &mut self,
        mode: &motion_core::drawing::MaskMode,
        path: &str,
    ) -> Result<(), ValidationError> {
        let bounds = self.surfaces.last().expect("validated mask owner").bounds;
        let mut mask = Surface::new_pooled(
            bounds,
            1.0,
            motion_core::drawing::BlendMode::SourceOver,
            [0.0; 4],
            self.budget,
            path,
            Some(&mut *self.pool),
        )?;
        mask.mask_mode = Some(*mode);
        self.budget.push(&mut self.surfaces, mask, path)?;
        self.peak_bytes = self.peak_bytes.max(self.budget.live_bytes());
        Ok(())
    }

    pub(super) fn end_mask(&mut self, path: &str) -> Result<(), ValidationError> {
        let mask = self.surfaces.pop().expect("validated mask phase");
        let mode = mask.mask_mode.expect("validated mask capture");
        let target = self.surfaces.last_mut().expect("validated mask owner");
        self.budget.work(target.bounds.pixels(), path)?;
        for (destination, pixel) in target.pixels.iter_mut().zip(&mask.pixels) {
            let coverage = masking::coverage(*pixel, mode);
            *destination = destination.map(|value| (f64::from(value) * coverage) as f32);
        }
        self.budget.release(
            mask.pixels.len() as u64 * size_of::<[f32; 4]>() as u64 + size_of::<Surface>() as u64,
        );
        self.pool.put(mask.pixels);
        Ok(())
    }

    pub(super) fn pop_layer(&mut self, path: &str) -> Result<(), ValidationError> {
        let frame = self.frame;
        let mut layer = self.surfaces.pop().expect("prepared layer balance");
        if let Some(index) = layer.filter_item {
            let PreparedItem::PushLayer { layer: descriptor } = &frame.items[index] else {
                unreachable!("validated filter layer")
            };
            let filters = descriptor.filters.as_ref().expect("source filter scope");
            filter::apply(
                &mut layer,
                filters.source_bounds,
                &filters.operations,
                frame.output_bounds,
                self.budget,
                path,
                frame,
            )?;
        }
        let parent = self
            .surfaces
            .last_mut()
            .expect("prepared layer has a parent");
        let output = layer.bounds.intersect(parent.bounds);
        self.budget.work(output.pixels(), path)?;
        let masked = layer.clip.is_some() || layer.rect_clip != parent.rect_clip;
        for row in output.top..output.bottom {
            if !masked {
                // Without masks both coverage factors are exactly 1.0, so only opacity scales.
                let width = (output.right - output.left) as usize;
                let from = layer.index(output.left, row);
                let to = parent.index(output.left, row);
                let sources = &layer.pixels[from..from + width];
                for (destination, pixel) in parent.pixels[to..to + width].iter_mut().zip(sources) {
                    if *pixel == [0.0; 4] {
                        continue;
                    }
                    let source = pixel.map(|value| (f64::from(value) * layer.opacity) as f32);
                    blending::composite_in_space(
                        destination,
                        source,
                        layer.blend_mode,
                        frame.blend_space,
                        path,
                    )?;
                }
                continue;
            }
            for column in output.left..output.right {
                let pixel = layer.pixels[layer.index(column, row)];
                // Transparent black scaled by any finite factor is still transparent black,
                // which `composite_in_space` ignores, so skip the mask lookups and scaling.
                if pixel == [0.0; 4] {
                    continue;
                }
                let coverage = layer
                    .clip
                    .as_ref()
                    .map_or(1.0, |mask| f64::from(mask.alpha(column, row)) / 255.0);
                let rectangle = if layer.rect_clip == parent.rect_clip {
                    1.0
                } else {
                    let numerator = layer
                        .rectangle
                        .as_ref()
                        .map_or(255, |mask| mask.alpha(column, row));
                    let denominator = parent
                        .rectangle
                        .as_ref()
                        .map_or(255, |mask| mask.alpha(column, row));
                    if denominator == 0 {
                        0.0
                    } else {
                        (f64::from(numerator) / f64::from(denominator)).min(1.0)
                    }
                };
                let source = pixel
                    .map(|value| (f64::from(value) * layer.opacity * coverage * rectangle) as f32);
                let destination = parent.index(column, row);
                blending::composite_in_space(
                    &mut parent.pixels[destination],
                    source,
                    layer.blend_mode,
                    frame.blend_space,
                    path,
                )?;
            }
        }
        self.budget.release(
            layer.pixels.len() as u64 * size_of::<[f32; 4]>() as u64
                + size_of::<Surface>() as u64
                + layer.clip.as_ref().map_or(0, |mask| mask.bytes())
                + layer.rectangle.as_ref().map_or(0, |mask| mask.bytes()),
        );
        self.pool.put(layer.pixels);
        Ok(())
    }

    pub(super) fn particles(
        &mut self,
        batch: &std::sync::Arc<dyn motion_core::instances::InstanceBatch>,
        transform: &motion_core::Transform,
        path: &str,
    ) -> Result<(), ValidationError> {
        let frame = self.frame;
        let target = self.surfaces.last_mut().expect("validated root surface");
        if particles::tiled(
            batch.as_ref(),
            *transform,
            target,
            frame.blend_space,
            self.budget,
            path,
        )? {
            self.peak_bytes = self.peak_bytes.max(self.budget.peak_bytes());
            return Ok(());
        }
        let inverse = transform.inverse().ok_or_else(|| {
            ValidationError::invalid(
                path,
                "transform",
                "Particle instance matrix must be invertible",
            )
        })?;
        let coefficients = inverse.coefficients();
        let antialias = libm::hypot(coefficients[0], coefficients[1])
            .max(libm::hypot(coefficients[2], coefficients[3]));
        self.budget.work(
            target
                .bounds
                .pixels()
                .checked_mul(batch.work())
                .ok_or_else(|| {
                    ValidationError::limit(path, "max_work", "Particle work overflow")
                })?,
            path,
        )?;
        for row in target.bounds.top..target.bounds.bottom {
            for column in target.bounds.left..target.bounds.right {
                let destination = target.index(column, row);
                let point = inverse.apply(motion_core::Point::new(
                    f64::from(column) + 0.5,
                    f64::from(row) + 0.5,
                ));
                target.pixels[destination] = batch.sample_with_zero_identity(
                    point,
                    antialias,
                    target.pixels[destination],
                    frame.blend_space,
                    particle_compositor(batch.blend()),
                )?;
            }
        }
        Ok(())
    }

    pub(super) fn fill(
        &mut self,
        strips: &std::ops::Range<usize>,
        spans: &std::ops::Range<usize>,
        paint: &usize,
        path: &str,
    ) -> Result<(), ValidationError> {
        self.fill_mode(strips, spans, paint, path, None)
    }

    pub(super) fn fill_mode(
        &mut self,
        strips: &std::ops::Range<usize>,
        spans: &std::ops::Range<usize>,
        paint: &usize,
        path: &str,
        layer: Option<&PreparedLayer>,
    ) -> Result<(), ValidationError> {
        let frame = self.frame;
        let space = frame.blend_space;
        let covered = if space == motion_core::drawing::BlendSpace::LinearRgb {
            super::blend
        } else {
            |destination: &mut [f32; 4], source, coverage, path: &str| {
                blending::covered(
                    destination,
                    source,
                    coverage,
                    motion_core::drawing::BlendSpace::Srgb,
                    path,
                )
            }
        };
        if let Some(layer) = layer {
            self.fill_rows(
                strips,
                spans,
                paint,
                path,
                Some(layer.bounds),
                |destination, source, coverage| {
                    let mut isolated = [0.0; 4];
                    covered(&mut isolated, source, coverage, path)?;
                    let source =
                        isolated.map(|value| (f64::from(value) * layer.opacity * 1.0 * 1.0) as f32);
                    blending::composite_in_space(destination, source, layer.blend_mode, space, path)
                },
            )
        } else {
            self.fill_rows(
                strips,
                spans,
                paint,
                path,
                None,
                |destination, source, coverage| covered(destination, source, coverage, path),
            )
        }
    }

    fn fill_rows(
        &mut self,
        strips: &std::ops::Range<usize>,
        spans: &std::ops::Range<usize>,
        paint: &usize,
        path: &str,
        bounds: Option<PixelRect>,
        composite: impl Fn(&mut [f32; 4], [f32; 4], f32) -> Result<(), ValidationError>,
    ) -> Result<(), ValidationError> {
        let frame = self.frame;
        let target = self.surfaces.last_mut().expect("validated root surface");
        let target_bounds = bounds.map_or(target.bounds, |bounds| target.bounds.intersect(bounds));
        let paint_index = *paint;
        let sampling_work = frame.paint_sources[paint_index]
            .as_ref()
            .map_or(1, |source| source.work_per_pixel())
            .checked_add(u64::from(bounds.is_some()))
            .ok_or_else(|| ValidationError::limit(path, "max_work", "Paint work overflow"))?;
        let solid = (bounds.is_none()
            && frame.blend_space == motion_core::drawing::BlendSpace::LinearRgb
            && frame.paint_sources[paint_index].is_none())
        .then_some(frame.paints[paint_index].linear_premultiplied);
        for strip in &frame.strips[strips.clone()] {
            let bounds = (PixelRect {
                left: strip.x,
                top: strip.y,
                right: strip.x + strip.width,
                bottom: strip.y + strip.height,
            })
            .intersect(target_bounds);
            self.budget.work(
                bounds.pixels().checked_mul(sampling_work).ok_or_else(|| {
                    ValidationError::limit(path, "max_work", "Paint work overflow")
                })?,
                path,
            )?;
            let width = (bounds.right - bounds.left) as usize;
            if width == 0 {
                continue;
            }
            for row in bounds.top..bounds.bottom {
                let mut sampling = frame.paint_row(paint_index, bounds.left, row);
                let alpha_start = strip.alpha_offset
                    + (row - strip.y) as usize * strip.width as usize
                    + (bounds.left - strip.x) as usize;
                let start = target.index(bounds.left, row);
                let pixels = &mut target.pixels[start..start + width];
                let alphas = &frame.alphas[alpha_start..alpha_start + width];
                for ((column, destination), alpha) in
                    (bounds.left..bounds.right).zip(pixels).zip(alphas)
                {
                    if *alpha == 0 {
                        continue;
                    }
                    composite(
                        destination,
                        sampling.sample(column)?,
                        f32::from(*alpha) / 255.0,
                    )?;
                }
            }
        }
        for span in &frame.spans[spans.clone()] {
            let bounds = (PixelRect {
                left: span.start_x,
                top: span.y,
                right: span.end_x,
                bottom: span.y + span.height,
            })
            .intersect(target_bounds);
            self.budget.work(
                bounds.pixels().checked_mul(sampling_work).ok_or_else(|| {
                    ValidationError::limit(path, "max_work", "Paint work overflow")
                })?,
                path,
            )?;
            let width = (bounds.right - bounds.left) as usize;
            if width == 0 {
                continue;
            }
            for row in bounds.top..bounds.bottom {
                let mut sampling = frame.paint_row(paint_index, bounds.left, row);
                let start = target.index(bounds.left, row);
                let pixels = &mut target.pixels[start..start + width];
                if let Some(source) = solid {
                    super::blend_span(pixels, source, path)?;
                    continue;
                }
                for (column, destination) in (bounds.left..bounds.right).zip(pixels) {
                    composite(destination, sampling.sample(column)?, 1.0)?;
                }
            }
        }
        Ok(())
    }
}
