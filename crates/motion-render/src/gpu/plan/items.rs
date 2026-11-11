//! Per-item planning: one method per prepared item, sharing the layer stack.

use super::*;
use motion_core::ValidationError;
use std::ops::Range;

/// Per-draw pattern words, including bitcast offsets into immutable mip texels.
const PATTERN_WORDS: usize = 8;

/// Four two-coordinate corners followed by four Oklab-plus-alpha colors.
const MESH_PATCH_WORDS: usize = 24;

/// Atmospheric seed, envelope kind, canvas, phase, recipe and tint words.
const ATMOSPHERE_WORDS: usize = 20;

/// Walks prepared items while tracking the open layer stack.
pub(super) struct ItemPlanner<'a> {
    pub(super) planner: Planner<'a>,
    pub(super) layers: Vec<Layer>,
}

impl<'a> ItemPlanner<'a> {
    /// Opens the root layer that every later item composes into.
    pub(super) fn new(mut planner: Planner<'a>, root: Layer) -> Result<Self, ValidationError> {
        let mut layers = Vec::new();
        planner.budget.push(&mut layers, root, "gpu.layers")?;
        Ok(Self { planner, layers })
    }

    pub(super) fn particles(
        &mut self,
        batch: &std::sync::Arc<dyn motion_core::instances::InstanceBatch>,
        transform: &motion_core::Transform,
    ) -> Result<(), ValidationError> {
        let frame = self.planner.frame;
        let layer = self.layers.last_mut().expect("validated root layer");
        let inverse = transform.inverse().ok_or_else(|| {
            ValidationError::invalid(
                "particles",
                "transform",
                "Particle instance matrix must be invertible",
            )
        })?;
        let coefficients = inverse.coefficients();
        let antialias = libm::hypot(coefficients[0], coefficients[1])
            .max(libm::hypot(coefficients[2], coefficients[3]));
        let mut parameters = GpuUniforms::new(frame, GpuOp::Particles, layer.bounds);
        let mut values = coefficients.to_vec();
        values.push(antialias);
        parameters.values(&values)?;
        self.planner
            .budget
            .bytes(batch.upload_bytes(), "gpu.particles")?;
        self.planner
            .budget
            .work(batch.upload_bytes() / 4, "gpu.particles")?;
        let upload = batch.upload()?.ok_or_else(|| {
            ValidationError::unsupported(
                "gpu.particles",
                "primitives",
                "Instance batch has no GPU representation",
            )
        })?;
        if upload.len() != batch.len() || upload.bytes() != batch.upload_bytes() {
            return Err(ValidationError::invalid(
                "particles",
                "upload",
                "Primitive count or byte charge mismatch",
            ));
        }
        let mut data = upload.into_words();
        let analytic = batch.analytic_upload();
        if let Some(program) = analytic {
            if program.len() != batch.len()
                || (data[3] as u32 & 31) != 12
                || (program.words()[3]
                    == motion_core::instances::InstanceKind::Sprite as u32 as f32
                    && (data[1] == 0.0 || data[2] == 0.0))
            {
                return Err(ValidationError::invalid(
                    "gpu.particles",
                    "analytic",
                    "Analytic program requires matching twelve-word primitives",
                ));
            }
            self.planner.admit_analytic_program(program)?;
            // Only compute-generated positions reach support preparation and composition.
            for record in data[4..4 + batch.len() * 12].as_chunks_mut::<12>().0 {
                record[..4].fill(0.0);
                record[9] = 0.0;
            }
        }
        let resource = self.planner.particle_resource(&mut data)?;
        self.planner
            .budget
            .work(batch.len() as u64 * 128, "gpu.particle_states")?;
        layer.image = self
            .planner
            .step(layer.image, 0, parameters, data, batch.work())?;
        if let Some(words) = resource {
            self.planner.attach_resource(layer.image, words)?;
        }
        if let Some(step) = self.planner.steps.last_mut() {
            step.analytic = analytic.cloned();
        }
        Ok(())
    }

    pub(super) fn push_filtered_layer(
        &mut self,
        descriptor: &crate::prepared::PreparedLayer,
        index: usize,
    ) -> Result<(), ValidationError> {
        let mut layer = self.planner.clear(descriptor.bounds, [0.0; 4])?;
        layer.filters = Some(index);
        self.planner
            .budget
            .push(&mut self.layers, layer, "gpu.layers")?;
        Ok(())
    }

    pub(super) fn push_layer(
        &mut self,
        descriptor: &crate::prepared::PreparedLayer,
    ) -> Result<(), ValidationError> {
        let frame = self.planner.frame;
        let crate::prepared::PreparedLayer {
            bounds,
            opacity,
            blend_mode,
            alpha_clip: clip,
            ..
        } = descriptor;
        let parent = self.layers.last().expect("validated root");
        let geometric = descriptor.geometric_clip.or(parent.geometric);
        let mut bounds = bounds.intersect(parent.bounds);
        for record in [*clip, geometric].into_iter().flatten() {
            bounds = bounds.intersect(frame.clips[record].bounds);
        }
        let mut layer = self.planner.clear(bounds, [0.0; 4])?;
        layer.opacity = *opacity;
        layer.mode = *blend_mode;
        layer.clip = *clip;
        layer.geometric = geometric;
        self.planner
            .budget
            .push(&mut self.layers, layer, "gpu.layers")?;
        Ok(())
    }

    pub(super) fn fill(
        &mut self,
        strips: &Range<usize>,
        spans: &Range<usize>,
        paint: &usize,
    ) -> Result<(), ValidationError> {
        let frame = self.planner.frame;
        let layer_bounds = self.layers.last().expect("validated root").bounds;
        let decal = frame.paint_sources[*paint].as_ref().is_some_and(
            |source| matches!(&source.paint, motion_core::Paint::Pattern(pattern) if pattern.decal),
        );
        let mut parameters = GpuUniforms::new(frame, GpuOp::Fill, layer_bounds);
        // Only drawn quads name their record; a compute pass would scan every record per pixel.
        let direct = !decal && !strips.is_empty() && parameters.operation.blend_space == 0;
        if !direct
            && frame
                .device_fills
                .iter()
                .any(|fill| strips.contains(&fill.strip))
        {
            return Err(ValidationError::unsupported(
                "gpu.device_coverage",
                "fill",
                "Device coverage requires direct fill coverage",
            ));
        }
        let mut data = if direct {
            self.planner
                .direct_coverage(strips.clone(), spans.clone(), layer_bounds)?
        } else {
            let sparse = if decal {
                None
            } else {
                self.planner
                    .sparse_coverage(strips.clone(), spans.clone(), layer_bounds)?
            };
            sparse.map_or_else(
                || self.planner.coverage(strips.clone(), spans.clone(), 0.0),
                Ok,
            )?
        };
        parameters.values(&frame.paints[*paint].linear_premultiplied.map(f64::from))?;
        let mut resource = None;
        if let Some(source) = &frame.paint_sources[*paint] {
            if let motion_core::Paint::Pattern(pattern) = &source.paint
                && pattern.decal
            {
                self.clip_decal_coverage(&mut data, pattern, source)?;
            }
            self.attach_spatial_paint(source, &mut data, &mut parameters)?;
            if let motion_core::Paint::Pattern(pattern) = &source.paint {
                let count = (0..pattern.image.levels())
                    .map(|level| {
                        let size = pattern.image.size(level);
                        size.0 as usize * size.1 as usize * 4
                    })
                    .sum();
                let mut words = self.planner.data(count, 0.0)?;
                words.clear();
                for level in 0..pattern.image.levels() {
                    pattern.image.extend_level(level, &mut words);
                }
                resource = Some(words);
            }
        }
        let work = frame.paint_sources[*paint]
            .as_ref()
            .map_or(1, |source| source.work_per_pixel());
        // A drawn quad names its own record, but a compute pass scans every direct record per pixel.
        let scan = (data[0].to_bits() == DIRECT_COVERAGE).then(|| Planner::clip_work(&data));
        let layer = self.layers.last_mut().expect("validated root");
        layer.image = self.planner.step(layer.image, 0, parameters, data, work)?;
        if let Some(words) = resource {
            self.planner.attach_resource(layer.image, words)?;
        }
        self.push_quads(strips, spans, layer_bounds)?;
        if let (Some(scan), Some(step)) = (scan, self.planner.steps.last())
            && !super::schedule::bounded_fill(step)
        {
            let pixels = step.parameters.image_bounds(2).pixels();
            self.planner.budget.work(
                pixels.checked_mul(scan).ok_or_else(|| {
                    ValidationError::limit("gpu.work", "max_work", "GPU work overflow")
                })?,
                "gpu.dispatch",
            )?;
        }
        Ok(())
    }

    /// Zeroes coverage outside the image for decal patterns, which do not repeat or extend.
    fn clip_decal_coverage(
        &mut self,
        data: &mut Vec<f32>,
        pattern: &motion_core::ImagePattern,
        source: &crate::raster::paint::Source,
    ) -> Result<(), ValidationError> {
        let bounds = self.planner.coverage_bounds(&*data);
        if data[0].to_bits() & (1 << 31) != 0 {
            let expanded = self.planner.coverage_packed(bounds, data[4])?;
            self.planner.budget.release(data.len() as u64 * 4);
            *data = expanded;
        }
        self.planner
            .budget
            .work(bounds.pixels() * 8, "gpu.image_decal")?;
        let inverse = pattern
            .transform
            .inverse()
            .expect("validated image transform");
        let width = (bounds.right - bounds.left) as usize;
        for index in 0..bounds.pixels() as usize {
            let column = (index % width) as u32 + bounds.left;
            let row = (index / width) as u32 + bounds.top;
            if self.planner.coverage_at(&*data, column, row) == 0.0 {
                continue;
            }
            let point = inverse.apply(source.inverse.apply(motion_core::Point::new(
                (index % width) as f64 + f64::from(bounds.left) + 0.5,
                (index / width) as f64 + f64::from(bounds.top) + 0.5,
            )));
            if point.x < 0.0
                || point.y < 0.0
                || point.x >= f64::from(pattern.image.width())
                || point.y >= f64::from(pattern.image.height())
            {
                Planner::set_coverage_byte(data, index, 0);
            }
        }
        Ok(())
    }

    /// Appends gradient or image-pattern payload to the coverage data and records the paint mapping.
    fn attach_spatial_paint(
        &mut self,
        source: &crate::raster::paint::Source,
        data: &mut Vec<f32>,
        parameters: &mut GpuUniforms,
    ) -> Result<(), ValidationError> {
        use motion_core::drawing::paint::Geometry;
        let additional = source.paint.gradient().map_or_else(
            || match &source.paint {
                motion_core::Paint::Pattern(_) => PATTERN_WORDS,
                motion_core::Paint::Mesh(_) => 1 + source.mesh_patches.len() * MESH_PATCH_WORDS,
                motion_core::Paint::Atmosphere(_) => ATMOSPHERE_WORDS,
                _ => 0,
            },
            |gradient| 1 + gradient.stops.len() + source.gradient_table.len() * 4,
        );
        self.planner
            .budget
            .bytes(additional as u64 * 4, "gpu.paint")?;
        self.planner.budget.work(additional as u64, "gpu.paint")?;
        data.try_reserve(additional).map_err(|_| {
            ValidationError::limit(
                "gpu.paint",
                "max_bytes",
                "Could not reserve spatial paint upload",
            )
        })?;
        let mut mapping = source.inverse;
        let mut geometry = [0.0; 4];
        let mut focus = [0.0; 2];
        if let Some(gradient) = source.paint.gradient() {
            let kind = match gradient.geometry {
                Geometry::Linear(first, second) => {
                    geometry = [first.x, first.y, second.x, second.y];
                    PaintKind::Linear
                }
                Geometry::Radial(center, radius) => {
                    geometry = [center.x, center.y, radius, 0.0];
                    PaintKind::Radial
                }
                Geometry::RadialFocal(center, radius, point) => {
                    geometry = [center.x, center.y, radius, 0.0];
                    focus = [point.x, point.y];
                    PaintKind::RadialFocal
                }
                Geometry::Sweep(center, start, end) => {
                    geometry = [center.x, center.y, start, end];
                    PaintKind::Sweep
                }
            };
            parameters.gradient(kind, gradient.units, gradient.extend);
            data.push(gradient.stops.len() as f32);
            data.extend(gradient.stops.iter().map(|(position, _)| *position as f32));
            data.extend(
                source
                    .gradient_table
                    .iter()
                    .flat_map(|color| color.iter().copied()),
            );
        } else if let motion_core::Paint::Pattern(pattern) = &source.paint {
            mapping = mapping.then(
                pattern
                    .transform
                    .inverse()
                    .expect("validated image transform"),
            );
            parameters.pattern(pattern.extend);
            geometry = [
                f64::from(pattern.image.width()),
                f64::from(pattern.image.height()),
                f64::from(pattern.decal),
                0.0,
            ];
            let first = pattern.image.size(source.mip.0);
            let second = pattern.image.size(source.mip.1);
            data.extend([
                first.0 as f32,
                first.1 as f32,
                second.0 as f32,
                second.1 as f32,
                source.mip.2,
                if pattern.sampling == motion_core::Sampling::Nearest {
                    1.0
                } else {
                    0.0
                },
            ]);
            for selected in [source.mip.0, source.mip.1] {
                let offset = (0..selected).try_fold(0u32, |offset, level| {
                    let size = pattern.image.size(level);
                    size.0
                        .checked_mul(size.1)
                        .and_then(|pixels| pixels.checked_mul(4))
                        .and_then(|words| offset.checked_add(words))
                        .ok_or_else(|| {
                            ValidationError::limit(
                                "gpu.paint",
                                "max_bytes",
                                "Image mip offsets exceed u32 storage",
                            )
                        })
                })?;
                data.push(f32::from_bits(offset));
            }
        } else if matches!(
            source.paint,
            motion_core::Paint::Mesh(_) | motion_core::Paint::Atmosphere(_)
        ) {
            parameters.gradient(
                if matches!(source.paint, motion_core::Paint::Mesh(_)) {
                    PaintKind::Mesh
                } else {
                    PaintKind::Atmosphere
                },
                motion_core::GradientUnits::ObjectBoundingBox,
                motion_core::GradientExtend::Pad,
            );
            if let motion_core::Paint::Atmosphere(atmosphere) = &source.paint {
                let recipe = atmosphere.parameters();
                let (kind, envelope) = match recipe.kind {
                    motion_core::AtmosphereKind::Fog { gradient } => {
                        (0.0, [gradient[0], gradient[1], 0.0, 0.0])
                    }
                    motion_core::AtmosphereKind::Dust { radius, coverage } => {
                        (1.0, [radius, coverage, 0.0, 0.0])
                    }
                    motion_core::AtmosphereKind::LightLeak { center, extent } => {
                        (2.0, [center[0], center[1], extent[0], extent[1]])
                    }
                };
                let phase = atmosphere.phase();
                data.push(f32::from_bits(recipe.seed));
                data.extend(
                    [
                        kind,
                        atmosphere.canvas().width as f64,
                        atmosphere.canvas().height as f64,
                        phase[0],
                        phase[1],
                        recipe.scale,
                        recipe.opacity,
                        recipe.intensity,
                        recipe.fractal.lacunarity,
                        recipe.fractal.gain,
                        f64::from(recipe.fractal.octaves),
                    ]
                    .map(|value| value as f32),
                );
                data.extend(
                    recipe
                        .color
                        .to_linear_premultiplied()
                        .map(|value| value as f32),
                );
                data.extend(envelope.map(|value| value as f32));
            } else {
                data.push(source.mesh_patches.len() as f32);
                for patch in source.mesh_patches.iter() {
                    for point in patch.positions() {
                        data.extend([point.x as f32, point.y as f32]);
                    }
                    data.extend(patch.colors().iter().flatten().map(|value| *value as f32));
                }
            }
        }
        let anchor = if parameters.is_pattern() {
            mapping
                .inverse()
                .map(|inverse| {
                    inverse.apply(motion_core::Point::new(
                        geometry[0] * 0.5,
                        geometry[1] * 0.5,
                    ))
                })
                .unwrap_or(motion_core::Point::ORIGIN)
        } else {
            motion_core::Point::ORIGIN
        };
        let [xx, yx, xy, yy, _, _] = mapping.coefficients();
        let origin = mapping.apply(anchor);
        let mut values = vec![xx, yx, xy, yy, origin.x, origin.y];
        values.extend([
            source.bounds.left,
            source.bounds.top,
            source.bounds.width(),
            source.bounds.height(),
        ]);
        values.extend(geometry);
        values.extend([anchor.x, anchor.y]);
        values.extend(focus);
        parameters.values(&values)?;
        Ok(())
    }

    /// Records the draw rectangles of a fill and admits their instance and staging storage.
    fn push_quads(
        &mut self,
        strips: &Range<usize>,
        spans: &Range<usize>,
        layer_bounds: PixelRect,
    ) -> Result<(), ValidationError> {
        let frame = self.planner.frame;
        let direct = self
            .planner
            .steps
            .last()
            .is_some_and(|step| step.data[0].to_bits() == DIRECT_COVERAGE);
        let mut quads = Vec::new();
        for strip in &frame.strips[strips.clone()] {
            let rect = PixelRect {
                left: strip.x,
                top: strip.y,
                right: strip.x + strip.width,
                bottom: strip.y + strip.height,
            }
            .intersect(layer_bounds);
            if direct || !rect.is_empty() {
                quads.push([
                    rect.left as f32,
                    rect.top as f32,
                    rect.right as f32,
                    rect.bottom as f32,
                ]);
            }
        }
        for span in &frame.spans[spans.clone()] {
            let rect = PixelRect {
                left: span.start_x,
                top: span.y,
                right: span.end_x,
                bottom: span.y + span.height,
            }
            .intersect(layer_bounds);
            if direct || !rect.is_empty() {
                quads.push([
                    rect.left as f32,
                    rect.top as f32,
                    rect.right as f32,
                    rect.bottom as f32,
                ]);
            }
        }
        if !quads.is_empty() {
            // Quads draw straight into the layer's working texture, which `gpu.working_textures`
            // already charges, so only the instance records themselves are admitted here.
            self.planner
                .budget
                .bytes(quads.len() as u64 * 16, "gpu.strip_instances")?;
        }
        self.planner.steps.last_mut().expect("fill operation").quads = quads;
        Ok(())
    }

    pub(super) fn begin_mask(
        &mut self,
        mode: &motion_core::drawing::MaskMode,
    ) -> Result<(), ValidationError> {
        let mut layer = self
            .planner
            .clear(self.layers.last().expect("mask owner").bounds, [0.0; 4])?;
        layer.mask = Some(*mode);
        self.planner
            .budget
            .push(&mut self.layers, layer, "gpu.layers")?;
        Ok(())
    }

    pub(super) fn end_mask(&mut self) -> Result<(), ValidationError> {
        let frame = self.planner.frame;
        let mask = self.layers.pop().expect("mask");
        self.planner.budget.release(size_of::<Layer>() as u64);
        let owner = self.layers.last_mut().expect("mask owner");
        let mut parameters = GpuUniforms::new(frame, GpuOp::Mask, owner.bounds);
        parameters.mask(mask.mask.expect("mask mode"));
        owner.image = self
            .planner
            .step(owner.image, mask.image, parameters, Vec::new(), 1)?;
        Ok(())
    }

    pub(super) fn pop_layer(&mut self) -> Result<(), ValidationError> {
        let frame = self.planner.frame;
        let mut layer = self.layers.pop().expect("layer");
        self.planner.budget.release(size_of::<Layer>() as u64);
        if let Some(index) = layer.filters {
            let PreparedItem::PushLayer { layer: descriptor } = &frame.items[index] else {
                unreachable!("filter layers are recorded at their push item")
            };
            let filters = descriptor.filters.as_ref().expect("source filter scope");
            layer.image =
                self.planner
                    .filters(layer.image, filters.source_bounds, &filters.operations)?;
        }
        let parent = self.layers.last_mut().expect("parent");
        let mut data = self.planner.clip(layer.clip)?;
        let mut work = 1 + Planner::clip_work(&data);
        if layer.geometric != parent.geometric {
            let child = self.planner.clip(layer.geometric)?;
            let ancestor = self.planner.clip(parent.geometric)?;
            work += Planner::clip_work(&child) + Planner::clip_work(&ancestor) + 2;
            data = self.planner.conditional_clip(data, child, ancestor)?;
        }
        let mut parameters = GpuUniforms::new(
            frame,
            GpuOp::Composite,
            layer.bounds.intersect(parent.bounds),
        );
        parameters.blend(layer.mode);
        parameters.values(&[layer.opacity])?;
        parent.image = self
            .planner
            .step(parent.image, layer.image, parameters, data, work)?;
        Ok(())
    }
}
