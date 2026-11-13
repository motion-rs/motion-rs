//! GPU command steps and filter expansion.

use super::*;
use motion_core::{Rect, ValidationError};

/// Device evaluation work units per analytic initial state.
const ANALYTIC_WORK_PER_INSTANCE: u64 = 256;

/// Simultaneous analytic staging and device payload copies; host facts remain in the admitted IR.
const ANALYTIC_DEVICE_COPIES: u64 = 2;

/// Keep the bitmap path bounded; larger layouts use exact-size ordered indices.
const MAX_PARTICLE_MASK_BYTES: u64 = 32 * 1024 * 1024;

impl Planner<'_> {
    pub(in crate::gpu) fn admit_analytic_program(
        &mut self,
        program: &motion_core::instances::AnalyticInstanceUpload,
    ) -> Result<(), ValidationError> {
        self.budget.bytes(
            program.bytes() * ANALYTIC_DEVICE_COPIES
                + size_of::<wgpu::Buffer>() as u64
                + size_of::<wgpu::BindGroup>() as u64,
            "gpu.particles.analytic",
        )?;
        self.budget.work(
            program.len() as u64 * ANALYTIC_WORK_PER_INSTANCE + program.words().len() as u64,
            "gpu.particles.analytic",
        )
    }

    pub(super) fn data(&mut self, count: usize, value: f32) -> Result<Vec<f32>, ValidationError> {
        self.budget.bytes(count as u64 * 4, "gpu.data")?;
        self.budget.work(count as u64, "gpu.data")?;
        let mut data = Vec::new();
        data.try_reserve_exact(count).map_err(|_| {
            ValidationError::limit("gpu.data", "max_bytes", "Could not reserve GPU upload data")
        })?;
        data.resize(count, value);
        Ok(data)
    }

    pub(super) fn step(
        &mut self,
        first: usize,
        second: usize,
        mut parameters: GpuUniforms,
        mut data: Vec<f32>,
        factor: u64,
    ) -> Result<usize, ValidationError> {
        if data.is_empty() {
            data = self.data(1, 1.0)?;
        }
        let input_bounds = |image: usize| {
            image.checked_sub(1).map_or(
                PixelRect {
                    left: 0,
                    top: 0,
                    right: 0,
                    bottom: 0,
                },
                |index| self.steps[index].parameters.image_bounds(2),
            )
        };
        let first_bounds = input_bounds(first);
        let second_bounds = input_bounds(second);
        let mut bounds = PixelRect {
            left: parameters.bounds.left,
            top: parameters.bounds.top,
            right: parameters.bounds.right,
            bottom: parameters.bounds.bottom,
        };
        if [
            GpuOp::Fill as u32,
            GpuOp::Composite as u32,
            GpuOp::Mask as u32,
            GpuOp::Particles as u32,
        ]
        .contains(&parameters.operation.code)
            && first_bounds.pixels() != 0
        {
            if bounds.pixels() == 0 {
                bounds = first_bounds;
            } else {
                bounds = PixelRect {
                    left: bounds.left.min(first_bounds.left),
                    top: bounds.top.min(first_bounds.top),
                    right: bounds.right.max(first_bounds.right),
                    bottom: bounds.bottom.max(first_bounds.bottom),
                };
            }
        }
        parameters.images([first_bounds, second_bounds, bounds]);
        let tiled = parameters.operation.code == GpuOp::Particles as u32 && data[0] >= 256.0;
        if parameters.operation.code == GpuOp::Particles as u32 {
            let layout = ParticleTiles::new(bounds, data[0] as u32)?;
            if tiled {
                let size = image_size(bounds);
                let pixels = u64::from(size.width) * u64::from(size.height);
                let mask_bytes = layout.mask_bytes();
                let tile_work = u64::from(layout.tiles) * (u64::from(layout.count) * 32 + 64);
                let mask_work = layout
                    .mask_composition_work(pixels)
                    .and_then(|work| layout.mask_build_work()?.checked_add(work));
                let step_bytes = data.len() as u64 * 4 + size_of::<GpuUniforms>() as u64;
                let required_bytes = layout
                    .state_bytes()
                    .checked_add(mask_bytes.unwrap_or(u64::MAX))
                    .and_then(|bytes| bytes.checked_add(step_bytes))
                    .and_then(|bytes| bytes.checked_add(self.budget.live_bytes()));
                let required_work = mask_work
                    .and_then(|work| work.checked_add(pixels))
                    .and_then(|work| work.checked_add(self.budget.work_done()));
                let bitset = matches!(
                    (mask_bytes, mask_work, required_bytes, required_work),
                    (
                        Some(bytes),
                        Some(_),
                        Some(required_bytes),
                        Some(required_work)
                    ) if bytes <= MAX_PARTICLE_MASK_BYTES
                        && required_bytes <= self.budget.limits.max_bytes
                        && required_work <= self.budget.limits.max_work
                );
                if bitset && let (Some(mask_bytes), Some(mask_work)) = (mask_bytes, mask_work) {
                    self.budget
                        .bytes(layout.state_bytes() + mask_bytes, "gpu.particle_tiles")?;
                    self.budget.work(mask_work, "gpu.particle_tiles")?;
                    parameters.flags.particle_tiles = PARTICLE_TILE_BITSET;
                } else {
                    self.budget
                        .bytes(layout.bytes * 2 + 16, "gpu.particle_tiles")?;
                    self.budget.work(tile_work, "gpu.particle_tiles")?;
                    parameters.flags.particle_tiles = 1;
                }
            } else {
                self.budget.bytes(
                    (u64::from(layout.count) * 48).max(16),
                    "gpu.particle_states",
                )?;
            }
        }
        let factor = if tiled { 1 } else { factor };
        let size = image_size(bounds);
        self.budget.work(
            (u64::from(size.width) * u64::from(size.height))
                .checked_mul(factor)
                .ok_or_else(|| {
                    ValidationError::limit("gpu.work", "max_work", "GPU work overflow")
                })?,
            "gpu.dispatch",
        )?;
        self.budget.bytes(
            data.len() as u64 * 4 + size_of::<GpuUniforms>() as u64,
            "gpu.buffers",
        )?;
        self.budget.push(
            &mut self.steps,
            Step {
                first,
                second,
                parameters,
                data,
                resource: None,
                quads: Vec::new(),
                analytic: None,
            },
            "gpu.steps",
        )?;
        Ok(self.steps.len())
    }

    pub(super) fn clear(
        &mut self,
        bounds: PixelRect,
        color: [f32; 4],
    ) -> Result<Layer, ValidationError> {
        let mut parameters = GpuUniforms::new(self.frame, GpuOp::Clear, bounds);
        parameters.values(&color.map(f64::from))?;
        let image = self.step(0, 0, parameters, Vec::new(), 1)?;
        Ok(Layer {
            image,
            bounds,
            opacity: 1.0,
            mode: BlendMode::SourceOver,
            clip: None,
            geometric: None,
            mask: None,
            filters: None,
        })
    }

    pub(super) fn pass(
        &mut self,
        image: usize,
        domain: Rect,
        operation: GpuOp,
        values: &[f64],
    ) -> Result<usize, ValidationError> {
        let mut parameters =
            GpuUniforms::new(self.frame, operation, pixel_bounds(domain, self.frame.size));
        parameters.values(values)?;
        self.step(image, 0, parameters, Vec::new(), 1)
    }

    pub(super) fn composite(
        &mut self,
        backdrop: usize,
        source: usize,
        domain: Rect,
        blend: BlendMode,
    ) -> Result<usize, ValidationError> {
        let mut parameters = GpuUniforms::new(
            self.frame,
            GpuOp::Composite,
            pixel_bounds(domain, self.frame.size),
        );
        parameters.blend(blend);
        parameters.values(&[1.0])?;
        let data = self.data(1, 1.0)?;
        self.step(backdrop, source, parameters, data, 1)
    }

    pub(super) fn gaussian(
        &mut self,
        mut image: usize,
        domain: &mut Rect,
        sigma: f64,
    ) -> Result<usize, ValidationError> {
        if sigma == 0.0 {
            return Ok(image);
        }
        let output = motion_core::FilterOperation::Blur { sigma }.bounds(*domain, *domain);
        let mut working = *domain;
        for radius in motion_core::filter::quality_box_radii(sigma, self.frame.quality)? {
            if radius == 0 {
                continue;
            }
            let count = radius * 2 + 1;
            let next = Rect::new(
                working.left - f64::from(radius),
                working.top - f64::from(radius),
                working.right + f64::from(radius),
                working.bottom + f64::from(radius),
            );
            for horizontal in [true, false] {
                let bounds = if horizontal {
                    Rect::new(next.left, working.top, next.right, working.bottom)
                } else {
                    next
                };
                let mut kernel = self.data(count as usize, 0.0)?;
                self.budget.work(u64::from(count) * 2, "gpu.kernel")?;
                for (index, weight) in kernel.iter_mut().enumerate() {
                    *weight = motion_core::filter::box_weight(sigma, radius as usize, index) as f32;
                }
                let mut parameters = GpuUniforms::new(
                    self.frame,
                    GpuOp::Blur,
                    pixel_bounds(bounds, self.frame.size),
                );
                parameters.blur(horizontal, radius);
                image = self.step(image, 0, parameters, kernel, u64::from(count) * 4)?;
            }
            working = next;
        }
        *domain = output;
        Ok(image)
    }

    pub(super) fn primitive(
        &mut self,
        image: usize,
        domain: &mut Rect,
        source: usize,
        source_bounds: Rect,
        operation: &motion_core::FilterOperation,
    ) -> Result<usize, ValidationError> {
        use motion_core::FilterOperation as Operation;
        self.budget.work(1, "gpu.primitives")?;
        match operation {
            Operation::Transform(transform) => {
                *domain = operation.bounds(*domain, source_bounds);
                let inverse = transform.inverse().ok_or_else(|| {
                    ValidationError::invalid(
                        "filter.transform",
                        "matrix",
                        "Require invertible transform",
                    )
                })?;
                let mut parameters = [0.0; 32];
                parameters[..6].copy_from_slice(&inverse.coefficients());
                parameters[31] = 1.0;
                self.pass(image, *domain, GpuOp::Offset, &parameters)
            }
            Operation::BlurXY { sigma_x, sigma_y } => {
                let output = operation.bounds(*domain, source_bounds);
                let mut working = *domain;
                let mut result = image;
                for (axis, sigma) in [*sigma_x, *sigma_y].into_iter().enumerate() {
                    for radius in motion_core::filter::quality_box_radii(sigma, self.frame.quality)?
                    {
                        if radius == 0 {
                            continue;
                        }
                        let next = Rect::new(
                            working.left - if axis == 0 { f64::from(radius) } else { 0.0 },
                            working.top - if axis == 1 { f64::from(radius) } else { 0.0 },
                            working.right + if axis == 0 { f64::from(radius) } else { 0.0 },
                            working.bottom + if axis == 1 { f64::from(radius) } else { 0.0 },
                        );
                        let count = radius * 2 + 1;
                        let mut kernel = self.data(count as usize, 0.0)?;
                        for (index, weight) in kernel.iter_mut().enumerate() {
                            *weight = motion_core::filter::box_weight(sigma, radius as usize, index)
                                as f32;
                        }
                        let mut parameters = GpuUniforms::new(
                            self.frame,
                            GpuOp::Blur,
                            pixel_bounds(next, self.frame.size),
                        );
                        parameters.blur(axis == 0, radius);
                        result = self.step(result, 0, parameters, kernel, u64::from(count) * 4)?;
                        working = next;
                    }
                }
                *domain = output;
                Ok(result)
            }
            Operation::Blur { sigma } => self.gaussian(image, domain, *sigma),
            Operation::Offset { x, y } => {
                *domain = operation.bounds(*domain, source_bounds);
                self.pass(image, *domain, GpuOp::Offset, &[*x, *y])
            }
            Operation::ColorMatrix(matrix) => self.pass(image, *domain, GpuOp::ColorMatrix, matrix),
            Operation::Threshold { value } => {
                self.pass(image, *domain, GpuOp::Threshold, &[*value])
            }
            Operation::Noise { seed, amplitude } => {
                let mut parameters = GpuUniforms::new(
                    self.frame,
                    GpuOp::Noise,
                    pixel_bounds(*domain, self.frame.size),
                );
                parameters.noise(*seed);
                parameters.values(&[*amplitude])?;
                self.step(image, 0, parameters, Vec::new(), 1)
            }
            Operation::ChannelOffset { red, green, blue } => {
                *domain = operation.bounds(*domain, source_bounds);
                self.pass(
                    image,
                    *domain,
                    GpuOp::ChannelOffset,
                    &[red.x, red.y, green.x, green.y, blue.x, blue.y],
                )
            }
            Operation::CompositeWithSource { mode } => {
                *domain = operation.bounds(*domain, source_bounds);
                self.composite(source, image, *domain, *mode)
            }
        }
    }

    pub(super) fn filters(
        &mut self,
        mut image: usize,
        domain: Rect,
        filters: &[motion_core::Filter],
    ) -> Result<usize, ValidationError> {
        use motion_core::{Filter, FilterOperation as Operation};
        if domain.width() <= 0.0 || domain.height() <= 0.0 {
            return Ok(image);
        }
        let mut domain = motion_core::filter::raster_bounds(domain);
        for filter in filters {
            self.budget.work(1, "gpu.filters")?;
            let source = image;
            let source_bounds = domain;
            match filter {
                Filter::Optics(
                    optics @ motion_core::Optics::Anamorphic {
                        threshold,
                        tint,
                        intensity,
                        ..
                    },
                ) => {
                    let [sigma_x, sigma_y] = optics.streak_sigmas().expect("anamorphic sigmas");
                    image = self.pass(image, domain, GpuOp::Threshold, &[*threshold])?;
                    image = self.primitive(
                        image,
                        &mut domain,
                        source,
                        source_bounds,
                        &Operation::BlurXY { sigma_x, sigma_y },
                    )?;
                    let color = tint.to_linear_premultiplied();
                    let mut matrix = [0.0; 20];
                    for channel in 0..3 {
                        matrix[channel * 5 + channel] = if color[3] > 0.0 {
                            color[channel] / color[3]
                        } else {
                            0.0
                        };
                    }
                    matrix[18] = 1.0;
                    image = self.pass(image, domain, GpuOp::ColorMatrix, &matrix)?;
                    image = self.pass(image, domain, GpuOp::Scale, &[*intensity])?;
                    image = self.composite(source, image, domain, BlendMode::Plus)?;
                }
                Filter::Optics(optics) => {
                    let (values, kernel) = optics.parameters(domain);
                    domain = optics.bounds(domain);
                    let mut parameters = GpuUniforms::new(
                        self.frame,
                        GpuOp::Optics,
                        pixel_bounds(domain, self.frame.size),
                    );
                    parameters.values(&values)?;
                    let data = if matches!(optics, motion_core::Optics::Bokeh { .. }) {
                        let mut data = self.data(128, 0.0)?;
                        for (target, value) in data.iter_mut().zip(kernel.iter().flatten()) {
                            *target = *value;
                        }
                        data
                    } else {
                        Vec::new()
                    };
                    let factor = match optics {
                        motion_core::Optics::Bokeh { .. } => values[3] as u64 * 4,
                        motion_core::Optics::LensFlare { ghosts, .. } => u64::from(*ghosts) * 4,
                        motion_core::Optics::RadialBlur { samples, .. } => u64::from(*samples) * 4,
                        motion_core::Optics::Relight { .. } => 40,
                        _ => 12,
                    };
                    image = self.step(image, 0, parameters, data, factor)?;
                }
                Filter::Cdl(cdl) => {
                    let mut parameters = [0.0; 13];
                    for (index, values) in
                        [cdl.slope, cdl.offset, cdl.power].into_iter().enumerate()
                    {
                        for channel in 0..3 {
                            parameters[index * 4 + channel] = f64::from(values[channel]);
                        }
                    }
                    parameters[12] = f64::from(cdl.saturation);
                    image = self.pass(image, domain, GpuOp::Cdl, &parameters)?;
                }
                Filter::Lut { lut, input } => {
                    let mut data = self.data(lut_word_count(Some(lut)), 0.0)?;
                    write_lut(&mut data, Some(lut), *input);
                    let parameters = GpuUniforms::new(
                        self.frame,
                        GpuOp::Lut,
                        pixel_bounds(domain, self.frame.size),
                    );
                    image = self.step(
                        image,
                        0,
                        parameters,
                        data,
                        4 + u64::from(lut.shaper().is_some()) * 6,
                    )?;
                }
                Filter::Bloom {
                    threshold,
                    knee,
                    radius,
                    levels,
                    falloff,
                    intensity,
                    tint,
                } => {
                    let highlights = self.pass(
                        source,
                        source_bounds,
                        GpuOp::Threshold,
                        &[*threshold, *knee, 1.0],
                    )?;
                    let weights: [f64; 8] = std::array::from_fn(|level| {
                        if level < usize::from(*levels) {
                            libm::pow(*falloff, level as f64)
                        } else {
                            0.0
                        }
                    });
                    let total = weights.iter().sum::<f64>();
                    let factors = weights.map(|weight| weight / total * intensity);
                    let count = usize::from(*levels);
                    // The pyramid follows the CPU contract: downsample every level from its
                    // unblurred parent, blur each by `radius` level pixels, then sum coarse to fine.
                    let mut pyramid = [(highlights, source_bounds); 8];
                    for level in 1..count {
                        let (parent, parent_domain) = pyramid[level - 1];
                        let level_domain = motion_core::filter::bloom_down(parent_domain);
                        let parameters = GpuUniforms::new(
                            self.frame,
                            GpuOp::Downsample,
                            pixel_bounds(level_domain, self.frame.size),
                        );
                        pyramid[level] = (
                            self.step(parent, 0, parameters, Vec::new(), 4)?,
                            level_domain,
                        );
                    }
                    for (image, bounds) in &mut pyramid[..count] {
                        *image = self.gaussian(*image, bounds, *radius)?;
                    }
                    // Upsample without a second source only weights the coarsest level; unlike
                    // Scale it leaves alpha unclamped until the final tint, as on the CPU.
                    let (mut summed, mut region) = (0, Rect::default());
                    for level in (0..count).rev() {
                        let (blurred, bounds) = pyramid[level];
                        region = if summed == 0 {
                            bounds
                        } else {
                            motion_core::filter::union(
                                motion_core::filter::bloom_up(region),
                                bounds,
                            )
                        };
                        let mut parameters = GpuUniforms::new(
                            self.frame,
                            GpuOp::Upsample,
                            pixel_bounds(region, self.frame.size),
                        );
                        parameters.values(&[factors[level]])?;
                        summed = self.step(blurred, summed, parameters, Vec::new(), 5)?;
                    }
                    let color = tint.to_linear_premultiplied();
                    let mut matrix = [0.0; 20];
                    for channel in 0..3 {
                        matrix[channel * 5 + channel] = if color[3] > 0.0 {
                            color[channel] / color[3]
                        } else {
                            0.0
                        };
                    }
                    matrix[18] = color[3];
                    let tinted = self.pass(summed, region, GpuOp::ColorMatrix, &matrix)?;
                    domain = motion_core::filter::union(domain, region);
                    image = self.composite(image, tinted, domain, BlendMode::Plus)?;
                }
                Filter::Blur { sigma } => image = self.gaussian(image, &mut domain, *sigma)?,
                Filter::DropShadow {
                    offset,
                    sigma,
                    color,
                } => {
                    image = self.pass(
                        image,
                        domain,
                        GpuOp::Colorize,
                        &color.to_linear_premultiplied(),
                    )?;
                    image = self.gaussian(image, &mut domain, *sigma)?;
                    image = self.primitive(
                        image,
                        &mut domain,
                        source,
                        source_bounds,
                        &Operation::Offset {
                            x: offset.x,
                            y: offset.y,
                        },
                    )?;
                    domain = motion_core::filter::union(domain, source_bounds);
                    image = self.composite(image, source, domain, BlendMode::SourceOver)?;
                }
                Filter::Glow {
                    threshold,
                    sigma,
                    intensity,
                } => {
                    image = self.pass(image, domain, GpuOp::Threshold, &[*threshold])?;
                    image = self.gaussian(image, &mut domain, *sigma)?;
                    image = self.pass(image, domain, GpuOp::Scale, &[*intensity])?;
                    image = self.composite(source, image, domain, BlendMode::Plus)?;
                }
                Filter::ColorAdjust {
                    exposure,
                    contrast,
                    saturation,
                    hue_degrees,
                } => {
                    let radians = hue_degrees.rem_euclid(360.0).to_radians();
                    let sine = libm::sin(radians);
                    let cosine = libm::cos(radians);
                    image = self.pass(
                        image,
                        domain,
                        GpuOp::ColorAdjust,
                        &[
                            libm::exp2(*exposure),
                            *contrast,
                            *saturation,
                            cosine,
                            sine / libm::sqrt(3.0),
                        ],
                    )?;
                }
                Filter::Custom(operations) => {
                    for operation in operations {
                        image =
                            self.primitive(image, &mut domain, source, source_bounds, operation)?;
                    }
                }
            }
        }
        Ok(image)
    }
}
