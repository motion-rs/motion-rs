//! Frame planning: turns a prepared frame into budgeted GPU steps, display parameters and packing stats.

use super::*;
use motion_core::{PixelSize, ValidationError};

mod coverage;
mod items;
mod schedule;
mod sparse;
mod steps;
mod textures;

/// Packed storage, mapped readback and owned CPU output coexist until completion.
const PACKED_PAYLOAD_COPIES: u64 = 3;

fn lut_word_count(lut: Option<&motion_core::Lut3d>) -> usize {
    17 + lut.map_or(0, |lut| {
        (lut.data().len() + lut.shaper().map_or(0, |shaper| shaper.data().len())) * 3
    })
}

fn write_lut(target: &mut [f32], lut: Option<&motion_core::Lut3d>, input: motion_core::LutInput) {
    let (encoding, minimum, maximum) = match input {
        motion_core::LutInput::Linear => (0.0, 0.0, 1.0),
        motion_core::LutInput::SrgbEncoded => (1.0, 0.0, 1.0),
        motion_core::LutInput::Log { minimum, maximum } => (2.0, minimum, maximum),
    };
    let min = lut.map_or([0.0; 3], motion_core::Lut3d::domain_min);
    let max = lut.map_or([1.0; 3], motion_core::Lut3d::domain_max);
    let shaper = lut.and_then(motion_core::Lut3d::shaper);
    let shaper_min = shaper.map_or([0.0; 3], motion_core::Lut1d::domain_min);
    let shaper_max = shaper.map_or([1.0; 3], motion_core::Lut1d::domain_max);
    target[..17].copy_from_slice(&[
        lut.map_or(0.0, |lut| f32::from(lut.size())),
        min[0],
        min[1],
        min[2],
        max[0],
        max[1],
        max[2],
        encoding,
        minimum,
        maximum,
        shaper.map_or(0.0, |shaper| shaper.data().len() as f32),
        shaper_min[0],
        shaper_min[1],
        shaper_min[2],
        shaper_max[0],
        shaper_max[1],
        shaper_max[2],
    ]);
    if let Some(lut) = lut {
        let samples = lut.data().iter().flatten().chain(
            shaper
                .into_iter()
                .flat_map(|shaper| shaper.data().iter().flatten()),
        );
        for (target, source) in target[17..].iter_mut().zip(samples) {
            *target = *source;
        }
    }
}

impl Plan {
    /// Builds all operations and accounts for uploads, captures, staging and output.
    /// Unsupported precision or exhausted logical budgets fail before device access.
    pub fn new(frame: &PreparedFrame, limits: RasterLimits) -> Result<Self, ValidationError> {
        Self::with_planning_limit(frame, limits, limits.max_bytes, None)
    }

    /// Plans a supported packed output without unused floating-point readback storage.
    /// Packed storage, mapped readback and CPU planes are admitted before optional batching.
    pub fn new_packed(
        frame: &PreparedFrame,
        limits: RasterLimits,
        format: crate::PixelFormat,
    ) -> Result<Self, ValidationError> {
        Self::with_planning_limit(frame, limits, limits.max_bytes, Some(format))
    }

    /// Bounds logical planning costs separately from the frame's submission allowance.
    /// Device/staging reservations are included conservatively in the planning charge.
    /// `packed` selects a packed output format; `None` plans floating-point readback.
    pub fn with_planning_limit(
        frame: &PreparedFrame,
        limits: RasterLimits,
        max_bytes: u64,
        packed: Option<crate::PixelFormat>,
    ) -> Result<Self, ValidationError> {
        let planning_limits = RasterLimits {
            max_bytes: limits.max_bytes.min(max_bytes),
            ..limits
        };
        let budget = Self::admit(frame, planning_limits, packed)?;
        let mut planner = Planner {
            frame,
            steps: Vec::new(),
            budget,
        };
        let full = PixelRect {
            left: 0,
            top: 0,
            right: frame.size.width,
            bottom: frame.size.height,
        };
        let root = planner.clear(
            full,
            frame
                .background
                .map_or([0.0; 4], |paint| paint.linear_premultiplied),
        )?;
        let mut walker = items::ItemPlanner::new(planner, root)?;
        for (index, item) in frame.items.iter().enumerate() {
            walker.planner.budget.work(1, "gpu.commands")?;
            match item {
                PreparedItem::ParticleInstances { batch, transform } => {
                    walker.particles(batch, transform)?
                }
                PreparedItem::PushLayer { layer } if layer.filters.is_some() => {
                    walker.push_filtered_layer(layer, index)?
                }
                PreparedItem::PushLayer { layer } => walker.push_layer(layer)?,
                PreparedItem::Fill {
                    strips,
                    spans,
                    paint,
                } => walker.fill(strips, spans, paint)?,
                PreparedItem::BeginMask { mode } => walker.begin_mask(mode)?,
                PreparedItem::EndMask => walker.end_mask()?,
                PreparedItem::PopLayer => walker.pop_layer()?,
            }
        }
        let items::ItemPlanner {
            planner,
            mut layers,
            ..
        } = walker;
        let output = layers.pop().expect("root").image;
        let mut plan = Self::finish(planner, output, frame.size, planning_limits)?;
        plan.packed_output = packed.map(|format| (frame.size(), format));
        plan.limits = limits;
        Ok(plan)
    }

    /// Sets output-only display processing, including its logical work and storage charge.
    pub fn with_display(
        mut self,
        display: &crate::DisplayTransform,
        frame: u64,
    ) -> Result<Self, ValidationError> {
        display.validate()?;
        self.display = display.gpu_words(frame);
        let append = |target: &mut Vec<f32>,
                      lut: Option<&motion_core::Lut3d>,
                      input: motion_core::LutInput| {
            let start = target.len();
            target.resize(start + lut_word_count(lut), 0.0);
            write_lut(&mut target[start..], lut, input);
        };
        self.lut_data.clear();
        let count = 34
            + display
                .look_lut
                .iter()
                .chain(&display.display_lut)
                .map(|lut| lut_word_count(Some(lut)) - 17)
                .sum::<usize>();
        self.stats.peak_bytes = self
            .stats
            .peak_bytes
            .checked_add(count as u64 * 8 + 128)
            .filter(|bytes| *bytes <= self.limits.max_bytes)
            .ok_or_else(|| {
                ValidationError::limit(
                    "display.lut",
                    "max_bytes",
                    "LUT storage exceeds the allowance",
                )
            })?;
        self.lut_data.try_reserve_exact(count).map_err(|_| {
            ValidationError::limit("display.lut", "max_bytes", "Could not reserve LUT upload")
        })?;
        append(
            &mut self.lut_data,
            display.look_lut.as_deref(),
            display.lut_input,
        );
        self.display[27] = self.lut_data.len() as u32;
        append(
            &mut self.lut_data,
            display.display_lut.as_deref(),
            motion_core::LutInput::SrgbEncoded,
        );
        let extra = if display.is_identity() {
            0
        } else {
            u64::from(self.size.width) * u64::from(self.size.height) * display.work_per_pixel()
        };
        self.stats.work = self
            .stats
            .work
            .checked_add(extra)
            .filter(|work| *work <= self.limits.max_work)
            .ok_or_else(|| {
                ValidationError::limit(
                    "display",
                    "max_work",
                    "Display processing exceeds the work allowance",
                )
            })?;
        Ok(self)
    }

    /// Admits the fixed costs of a frame (plan, staging, output, readback) and builds the budget.
    fn admit(
        frame: &PreparedFrame,
        limits: RasterLimits,
        packed: Option<crate::PixelFormat>,
    ) -> Result<Budget, ValidationError> {
        let count = u64::from(frame.size.width) * u64::from(frame.size.height);
        let mut budget = Budget::new(frame.size, limits)?;
        if count > u64::from(u32::MAX)
            || frame.size.width > i32::MAX as u32
            || frame.size.height > i32::MAX as u32
        {
            return Err(ValidationError::limit(
                "gpu.plan",
                "max_pixels",
                "GPU raster exceeds 32-bit shader addressing",
            ));
        }
        budget.bytes(
            frame.stats.retained_bytes + size_of::<Plan>() as u64,
            "gpu.plan",
        )?;
        if let Some(format) = packed {
            budget.bytes(8, "gpu.staging")?;
            let resources = packed_resources(frame.size(), format)?;
            budget.bytes(resources.peak_bytes, "gpu.pack")?;
            budget.work(resources.work, "gpu.pack")?;
        } else {
            budget.bytes(
                count.checked_mul(16).ok_or_else(|| {
                    ValidationError::limit("gpu.plan", "max_bytes", "GPU staging size overflow")
                })? + 8,
                "gpu.staging",
            )?;
            budget.bytes(
                frame.output_bounds.pixels() * 16 + size_of::<LinearFrame>() as u64,
                "gpu.output",
            )?;
            budget.work(count + frame.output_bounds.pixels(), "gpu.readback")?;
        }
        let _count = usize::try_from(count).map_err(|_| {
            ValidationError::limit(
                "gpu.plan",
                "max_pixels",
                "GPU raster exceeds addressable storage",
            )
        })?;
        Ok(budget)
    }

    /// Computes texture lifetimes and batch staging for the finished steps, then freezes the plan.
    fn finish(
        mut planner: Planner<'_>,
        output: usize,
        size: PixelSize,
        limits: RasterLimits,
    ) -> Result<Self, ValidationError> {
        let coverage = if planner
            .steps
            .iter()
            .any(|step| Planner::uses_direct_coverage(&step.data))
        {
            planner.shared_coverage()?
        } else {
            Vec::new()
        };
        planner.budget.bytes(
            (planner.steps.len() as u64 + 1) * (size_of::<usize>() * 3 + size_of::<u64>()) as u64,
            "gpu.lifetimes",
        )?;
        let last_uses = Self::last_uses(&planner.steps, output, &mut planner.budget)?;
        let (texture_blends, texture_bounded) =
            Self::texture_eligibility(&planner.steps, &mut planner.budget)?;
        let (texture_slots, texture_sizes) = Self::texture_lifetimes(
            &planner.steps,
            &last_uses,
            Some(&texture_blends),
            &mut planner.budget,
        )?;
        let (compute_texture_slots, compute_texture_sizes) =
            Self::texture_lifetimes(&planner.steps, &last_uses, None, &mut planner.budget)?;
        let texture_bytes = |sizes: &[PixelSize]| {
            sizes
                .iter()
                .map(|size| {
                    u64::from(size.width) * u64::from(size.height) * 16
                        + size_of::<wgpu::Texture>() as u64
                })
                .sum::<u64>()
        };
        planner.budget.bytes(
            texture_bytes(&texture_sizes).max(texture_bytes(&compute_texture_sizes)),
            "gpu.working_textures",
        )?;
        planner.budget.bytes(
            planner.steps.len() as u64
                * (256
                    + 32
                    + size_of::<(wgpu::BindGroup, wgpu::Buffer, u32)>() as u64
                    + size_of::<wgpu::BindGroup>() as u64)
                + 272
                + 32,
            "gpu.texture_batches",
        )?;
        let mut position = 0;
        while position < planner.steps.len() {
            if texture_blends[position] {
                let range = textures::run(&planner.steps, &last_uses, &texture_blends, position);
                planner.budget.bytes(
                    Self::batch_upload_bytes(&planner.steps[range.clone()]),
                    "gpu.batch_staging",
                )?;
                position = range.end;
            } else {
                position += 1;
            }
        }
        planner.budget.work(
            planner
                .steps
                .iter()
                .map(|step| image_bytes(step.parameters.image_bounds(2)) / 16)
                .sum(),
            "gpu.texture_backdrops",
        )?;
        planner.budget.bytes(
            image_bytes(planner.steps[output - 1].parameters.image_bounds(2)),
            "gpu.output_boundary",
        )?;
        planner.budget.work(
            image_size(planner.steps[output - 1].parameters.image_bounds(2)).width as u64
                * image_size(planner.steps[output - 1].parameters.image_bounds(2)).height as u64,
            "gpu.output_boundary",
        )?;
        planner.budget.bytes(
            (planner.steps.len() as u64 * 6 + 8) * size_of::<wgpu::Buffer>() as u64 + 32,
            "gpu.leases",
        )?;
        let texture_data_sizes = Self::admit_batch_data(
            &planner.steps,
            &last_uses,
            &texture_blends,
            &mut planner.budget,
        );
        let stats = WorkStats {
            peak_bytes: planner.budget.peak_bytes(),
            work: planner.budget.work_done(),
        };
        Ok(Self {
            size,
            display: [0; 32],
            lut_data: vec![0.0; 34],
            packed_output: None,
            limits,
            output,
            steps: planner.steps,
            coverage,
            last_uses,
            texture_slots,
            texture_sizes,
            compute_texture_slots,
            compute_texture_sizes,
            texture_blends,
            texture_bounded,
            texture_data_sizes,
            stats,
        })
    }

    /// Logical peak/work including packed output resources, checked before submission.
    pub fn packed_stats(
        &self,
        size: PixelSize,
        format: crate::PixelFormat,
    ) -> Result<WorkStats, ValidationError> {
        if size.width == 0
            || size.height == 0
            || size.width > self.size.width
            || size.height > self.size.height
        {
            return Err(ValidationError::invalid(
                "gpu.pack",
                "size",
                "Packed output must fit the prepared raster",
            ));
        }
        if !Gpu::supports_packed_output(size, format) {
            return Err(ValidationError::invalid(
                "gpu.pack",
                "format",
                "Output size/format requires linear fallback",
            ));
        }
        let resources = if let Some(output) = self.packed_output {
            if output != (size, format) {
                return Err(ValidationError::invalid(
                    "gpu.pack",
                    "format",
                    "Packed plan output does not match the requested size/format",
                ));
            }
            WorkStats::default()
        } else {
            packed_resources(size, format)?
        };
        let peak_bytes = self
            .stats
            .peak_bytes
            .checked_add(resources.peak_bytes)
            .filter(|bytes| *bytes <= self.limits.max_bytes)
            .ok_or_else(|| {
                ValidationError::limit(
                    "gpu.pack",
                    "max_bytes",
                    "Packed output resources exceed frame allowance",
                )
            })?;
        let work = self
            .stats
            .work
            .checked_add(resources.work)
            .filter(|work| *work <= self.limits.max_work)
            .ok_or_else(|| {
                ValidationError::limit(
                    "gpu.pack",
                    "max_work",
                    "Packed output exceeds shared frame work allowance",
                )
            })?;
        Ok(WorkStats { peak_bytes, work })
    }
}

fn packed_resources(
    size: PixelSize,
    format: crate::PixelFormat,
) -> Result<WorkStats, ValidationError> {
    if size.width == 0 || size.height == 0 || !Gpu::supports_packed_output(size, format) {
        return Err(ValidationError::invalid(
            "gpu.pack",
            "format",
            "Output size/format requires linear fallback",
        ));
    }
    let count = u64::from(size.width) * u64::from(size.height);
    let packed_bytes = match format {
        crate::PixelFormat::Yuv420 { bit_depth: 8 } => (count * 3 / 2).next_multiple_of(4),
        crate::PixelFormat::Yuv420 { bit_depth: 10 } => count * 3,
        _ => count * 4,
    };
    Ok(WorkStats {
        peak_bytes: packed_bytes * PACKED_PAYLOAD_COPIES
            + (size_of::<PendingOutputFrame>()
                + size_of::<crate::FrameBuffer>()
                + 3 * size_of::<crate::PixelPlane>()
                + 32) as u64,
        work: crate::FrameBuffer::conversion_work(size, format),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use motion_core::{Cdl, ColorWheels, Lut1d, LutInput};

    #[test]
    fn lut_protocol_carries_both_domains_and_appended_shaper_samples() {
        let cube = ColorWheels::default().bake(2, 1.0, Cdl::default()).unwrap();
        assert_eq!(lut_word_count(Some(&cube)), 41);
        let shaped = cube.clone().with_shaper(
            Lut1d::new([0.0; 3], [16.0; 3], vec![[0.0; 3], [0.8; 3], [1.0; 3]]).unwrap(),
        );
        let mut words = vec![0.0; lut_word_count(Some(&shaped))];
        write_lut(
            &mut words,
            Some(&shaped),
            LutInput::Log {
                minimum: -8.0,
                maximum: 4.0,
            },
        );
        assert_eq!(&words[7..11], &[2.0, -8.0, 4.0, 3.0]);
        assert_eq!(&words[14..17], &[16.0; 3]);
        assert_eq!(&words[41..], &[0.0, 0.0, 0.0, 0.8, 0.8, 0.8, 1.0, 1.0, 1.0]);
        assert_eq!(words.len() - lut_word_count(Some(&cube)), 9);
        let mut absent = [1.0; 17];
        write_lut(&mut absent, None, LutInput::Linear);
        assert_eq!(absent[0], 0.0);
        assert_eq!(absent[10], 0.0);
    }
}
