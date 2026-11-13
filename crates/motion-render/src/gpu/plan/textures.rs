//! Two-dimensional capacities and non-aliasing texture lifetimes.

use super::*;
use motion_core::{PixelSize, ValidationError};

pub(super) fn run(steps: &[Step], last: &[usize], blends: &[bool], start: usize) -> Range<usize> {
    let mut end = start + 1;
    if blends[start] {
        while end < steps.len()
            && blends[end]
            && steps[end].first == end
            && last[end] == end
            && !(start < steps[end].second && steps[end].second <= end)
            && steps[end].parameters.image_bounds(2) == steps[start].parameters.image_bounds(2)
        {
            end += 1;
        }
    }
    start..end
}

fn accumulation_slot(
    steps: &[Step],
    last: &[usize],
    slots: &[usize],
    deferred: &[bool],
    range: &Range<usize>,
    hardware: bool,
) -> Option<usize> {
    let step = &steps[range.start];
    let first = step.first;
    (hardware
        && first != 0
        && last[first] == range.start
        && !deferred[first]
        && steps[first - 1].parameters.image_bounds(2) == step.parameters.image_bounds(2)
        && steps[range.clone()].iter().all(|step| step.second != first))
    .then(|| slots[first])
}

impl Plan {
    /// Canonical byte alignment for shared storage records; stricter devices use per-draw data.
    pub(in crate::gpu) const BATCH_DATA_ALIGNMENT: u32 = 256;

    /// Minimum draw count for aggregation to remove at least two data buffers and copies.
    pub(in crate::gpu) const BATCH_DATA_MIN_DRAWS: usize = 3;

    pub(in crate::gpu) fn admit_batch_data(
        steps: &[Step],
        last: &[usize],
        blends: &[bool],
        budget: &mut Budget,
    ) -> Vec<u64> {
        let mut admitted = Budget::resume(
            budget.limits,
            budget.live_bytes(),
            budget.peak_bytes(),
            budget.work_done(),
        );
        let mut sizes = Vec::new();
        if admitted
            .bytes(
                steps.len() as u64 * size_of::<u64>() as u64,
                "gpu.batch_data_layout",
            )
            .is_err()
            || admitted
                .work(steps.len() as u64 * 4, "gpu.batch_data_layout")
                .is_err()
            || sizes.try_reserve_exact(steps.len()).is_err()
        {
            return sizes;
        }
        sizes.resize(steps.len(), 0);
        let mut position = 0;
        while position < steps.len() {
            let range = run(steps, last, blends, position);
            if blends[position]
                && range.len() >= Self::BATCH_DATA_MIN_DRAWS
                && let Some(bytes) = Self::batch_data_bytes(
                    &steps[range.clone()],
                    wgpu::Limits::default().max_buffer_size,
                    Self::BATCH_DATA_ALIGNMENT,
                )
            {
                let payload = steps[range.clone()]
                    .iter()
                    .map(|step| step.data.len() as u64 * size_of::<f32>() as u64)
                    .sum::<u64>();
                let padding = bytes - payload;
                let mut trial = Budget::resume(
                    admitted.limits,
                    admitted.live_bytes(),
                    admitted.peak_bytes(),
                    admitted.work_done(),
                );
                if trial.bytes(padding * 2, "gpu.batch_data_padding").is_ok()
                    && trial
                        .work(
                            padding / size_of::<f32>() as u64 * 2,
                            "gpu.batch_data_padding",
                        )
                        .is_ok()
                {
                    sizes[position] = bytes;
                    admitted = trial;
                }
            }
            position = range.end;
        }
        *budget = admitted;
        sizes
    }

    pub(in crate::gpu) fn texture_data_bytes(
        &self,
        position: usize,
        maximum: u64,
        alignment: u32,
    ) -> Option<u64> {
        if alignment == 0 || !Self::BATCH_DATA_ALIGNMENT.is_multiple_of(alignment) {
            return None;
        }
        self.texture_data_sizes
            .get(position)
            .copied()
            .filter(|bytes| *bytes != 0 && *bytes <= maximum)
    }

    pub(in crate::gpu) fn batch_data_bytes(
        steps: &[Step],
        maximum: u64,
        alignment: u32,
    ) -> Option<u64> {
        if steps.len() < 2 || alignment < 4 || !alignment.is_power_of_two() {
            return None;
        }
        let alignment = u64::from(alignment);
        steps
            .iter()
            .try_fold(0u64, |total, step| {
                let bytes = (step.data.len() as u64).checked_mul(size_of::<f32>() as u64)?;
                if bytes == 0 {
                    return None;
                }
                let start = total.checked_add(alignment - 1)? / alignment * alignment;
                start.checked_add(bytes)
            })
            .filter(|bytes| *bytes <= maximum)
    }

    pub(in crate::gpu) fn batch_uniform_bytes(
        steps: &[Step],
        maximum: u64,
        alignment: u32,
    ) -> Option<u64> {
        let stride = size_of::<GpuUniforms>() as u64;
        if alignment == 0 || !stride.is_multiple_of(u64::from(alignment)) {
            return None;
        }
        (steps.len() as u64)
            .checked_mul(stride)
            .filter(|bytes| !steps.is_empty() && *bytes <= maximum)
    }

    pub(in crate::gpu) fn batch_vertex_bytes(steps: &[Step], maximum: u64) -> Option<u64> {
        let stride = size_of::<[f32; 4]>() as u64;
        steps
            .iter()
            .try_fold(0u64, |total, step| {
                total.checked_add((step.quads.len().max(1) as u64).checked_mul(stride)?)
            })
            .filter(|bytes| !steps.is_empty() && *bytes <= maximum)
    }

    pub(in crate::gpu) fn batch_upload_bytes(steps: &[Step]) -> u64 {
        272 + steps
            .iter()
            .map(|step| 256 + step.data.len() as u64 * 4 + step.quads.len().max(1) as u64 * 16)
            .sum::<u64>()
    }

    pub(in crate::gpu) fn texture_run(&self, start: usize) -> Range<usize> {
        run(&self.steps, &self.last_uses, &self.texture_blends, start)
    }

    pub(in crate::gpu) fn texture_blend(&self, position: usize) -> bool {
        self.texture_blends[position]
    }

    /// Per step, whether hardware blending applies, and per image, whether every value is known
    /// to be finite and at most 65536 in magnitude from the plan alone.
    pub(in crate::gpu) fn texture_eligibility(
        steps: &[Step],
        budget: &mut Budget,
    ) -> Result<(Vec<bool>, Vec<bool>), ValidationError> {
        budget.bytes((steps.len() as u64 + 1) * 2, "gpu.blend_eligibility")?;
        budget.work(
            steps.iter().map(|step| step.data.len() as u64 + 4).sum(),
            "gpu.blend_eligibility",
        )?;
        let mut bounded = Vec::new();
        let mut blends = Vec::new();
        for values in [&mut bounded, &mut blends] {
            values.try_reserve_exact(steps.len() + 1).map_err(|_| {
                ValidationError::limit(
                    "gpu.blend_eligibility",
                    "max_bytes",
                    "Could not reserve blend classification",
                )
            })?;
        }
        bounded.push(true);
        for step in steps {
            let fill = super::schedule::bounded_fill(step);
            blends.push(
                step.parameters.operation.blend_space == 0
                    && (fill
                        || step.parameters.operation.code == GpuOp::Composite as u32
                            && matches!(step.parameters.operation.kind, 0 | 1 | 17)
                            && bounded[step.second]),
            );
            bounded.push(if step.parameters.operation.code == GpuOp::Clear as u32 {
                step.parameters.values[..4]
                    .iter()
                    .all(|word| word.abs() <= 65536.0)
            } else {
                fill && bounded[step.first]
            });
        }
        Ok((blends, bounded))
    }

    pub(in crate::gpu) fn texture_lifetimes(
        steps: &[Step],
        last: &[usize],
        blends: Option<&[bool]>,
        budget: &mut Budget,
    ) -> Result<(Vec<usize>, Vec<PixelSize>), ValidationError> {
        budget.bytes(
            (steps.len() as u64 + 1) * (size_of::<usize>() * 2 + size_of::<PixelSize>() + 1) as u64,
            "gpu.texture_lifetimes",
        )?;
        let mut slots = Vec::new();
        let mut sizes = Vec::new();
        let mut available = Vec::new();
        let mut deferred = Vec::new();
        deferred.try_reserve_exact(steps.len() + 1).map_err(|_| {
            ValidationError::limit(
                "gpu.texture_lifetimes",
                "max_bytes",
                "Could not reserve deferred alpha schedule",
            )
        })?;
        deferred.resize(steps.len() + 1, false);
        for values in [&mut slots, &mut available] {
            values.try_reserve_exact(steps.len() + 1).map_err(|_| {
                ValidationError::limit(
                    "gpu.texture_lifetimes",
                    "max_bytes",
                    "Could not reserve texture schedule",
                )
            })?;
        }
        sizes.try_reserve_exact(steps.len() + 1).map_err(|_| {
            ValidationError::limit(
                "gpu.texture_lifetimes",
                "max_bytes",
                "Could not reserve texture capacities",
            )
        })?;
        slots.push(0);
        sizes.push(PixelSize::new(1, 1));
        let mut position = 0;
        while position < steps.len() {
            let step = &steps[position];
            let range = blends.map_or(position..position + 1, |blends| {
                run(steps, last, blends, position)
            });
            budget.work(available.len() as u64 * 4 + 4, "gpu.texture_lifetimes")?;
            let size = image_size(step.parameters.image_bounds(2));
            let candidate = available
                .iter()
                .enumerate()
                .min_by_key(|(_, slot)| {
                    let capacity: PixelSize = sizes[**slot];
                    u64::from(capacity.width.max(size.width))
                        * u64::from(capacity.height.max(size.height))
                })
                .map(|(index, _)| index);
            let hardware = blends.is_some_and(|blends| blends[position]);
            let slot = if let Some(slot) =
                accumulation_slot(steps, last, &slots, &deferred, &range, hardware)
            {
                slot
            } else if let Some(index) = candidate {
                let slot = available.swap_remove(index);
                sizes[slot].width = sizes[slot].width.max(size.width);
                sizes[slot].height = sizes[slot].height.max(size.height);
                slot
            } else {
                sizes.push(size);
                sizes.len() - 1
            };
            slots.extend(std::iter::repeat_n(slot, range.len()));
            deferred[range.end] = hardware
                && steps[range.clone()].iter().any(|step| {
                    step.parameters.operation.code == GpuOp::Composite as u32
                        && step.parameters.operation.kind != 0
                });
            for index in range.clone() {
                for image in [steps[index].first, steps[index].second] {
                    if image != 0
                        && image <= position
                        && last[image] == index
                        && slots[image] != slot
                        && !available.contains(&slots[image])
                    {
                        available.push(slots[image]);
                    }
                }
            }
            if last[range.end] == range.end - 1 {
                available.push(slot);
            }
            position = range.end;
        }
        Ok((slots, sizes))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use motion_core::Shape;

    #[test]
    fn batch_data_admission_keeps_pairs_separate_and_shares_triples() -> motion_core::Result {
        for (count, packed, peak, work) in [(2, 0, 16, 8), (3, 552, 888, 228)] {
            let steps = (0..count)
                .map(|index| Step {
                    first: index,
                    second: 0,
                    parameters: GpuUniforms::default(),
                    data: vec![1.0; 10],
                    quads: Vec::new(),
                    analytic: None,
                    resource: None,
                })
                .collect::<Vec<_>>();
            let mut budget = Budget::new(PixelSize::new(1, 1), RasterLimits::default())?;
            let sizes = Plan::admit_batch_data(
                &steps,
                &(0..=count).collect::<Vec<_>>(),
                &vec![true; count],
                &mut budget,
            );
            assert_eq!(sizes[0], packed);
            assert!(sizes[1..].iter().all(|bytes| *bytes == 0));
            assert_eq!(budget.peak_bytes(), peak);
            assert_eq!(budget.work_done(), work);
        }
        Ok(())
    }

    #[test]
    fn batch_data_admission_falls_back_without_partial_padding_charges() -> motion_core::Result {
        let steps = [10, 1, 3]
            .into_iter()
            .enumerate()
            .map(|(index, words)| Step {
                first: index,
                second: 0,
                parameters: GpuUniforms::default(),
                data: vec![1.0; words],
                quads: Vec::new(),
                analytic: None,
                resource: None,
            })
            .collect::<Vec<_>>();
        for (maximum, work, expected, peak) in
            [(960, 246, 524, 960), (959, 246, 0, 24), (960, 245, 0, 24)]
        {
            let mut budget = Budget::new(
                PixelSize::new(1, 1),
                RasterLimits {
                    max_bytes: maximum,
                    max_work: work,
                    ..RasterLimits::default()
                },
            )?;
            let sizes = Plan::admit_batch_data(&steps, &[0, 1, 2, 3], &[true; 3], &mut budget);
            assert_eq!(sizes, [expected, 0, 0]);
            assert_eq!(budget.peak_bytes(), peak);
            assert_eq!(budget.work_done(), if expected == 0 { 12 } else { 246 });
        }
        let mut budget = Budget::new(
            PixelSize::new(1, 1),
            RasterLimits {
                max_bytes: 23,
                ..RasterLimits::default()
            },
        )?;
        assert!(Plan::admit_batch_data(&steps, &[0, 1, 2, 3], &[true; 3], &mut budget).is_empty());
        assert_eq!(budget.peak_bytes(), 0);
        Ok(())
    }

    #[test]
    fn batch_data_layout_preserves_record_lengths_and_limits() {
        let mut steps = [10, 1, 3].map(|words| Step {
            first: 0,
            second: 0,
            parameters: GpuUniforms::default(),
            data: vec![1.0; words],
            quads: Vec::new(),
            analytic: None,
            resource: None,
        });
        for (alignment, bytes) in [(4, 56), (16, 76), (64, 140), (256, 524)] {
            assert_eq!(
                Plan::batch_data_bytes(&steps, bytes, alignment),
                Some(bytes)
            );
            assert_eq!(Plan::batch_data_bytes(&steps, bytes - 1, alignment), None);
        }
        assert_eq!(Plan::BATCH_DATA_ALIGNMENT, 256);
        for alignment in [0, 1, 2, 3, 12] {
            assert_eq!(Plan::batch_data_bytes(&steps, u64::MAX, alignment), None);
        }
        assert_eq!(Plan::batch_data_bytes(&[], u64::MAX, 256), None);
        assert_eq!(Plan::batch_data_bytes(&steps[..1], u64::MAX, 256), None);
        steps[1].data.clear();
        assert_eq!(Plan::batch_data_bytes(&steps, u64::MAX, 256), None);
    }

    #[test]
    fn accumulation_rejects_retained_sources_deferred_alpha_and_domain_changes()
    -> motion_core::Result {
        let limits = RasterLimits::default();
        let frame = motion_core::Node::from(
            motion_core::Rect::new(0.25, 0.5, 31.5, 63.5).fill(motion_core::Color::WHITE),
        )
        .compile(
            motion_core::Size::new(32.0, 64.0),
            PixelSize::new(32, 64),
            motion_core::ValidationLimits::default(),
        )?
        .prepare(motion_core::ValidationLimits::default(), limits)?;
        let mut plan = Plan::new(&frame, limits)?;
        let start = plan.texture_blends.iter().position(|blend| *blend).unwrap();
        let range = plan.texture_run(start);
        let first = plan.steps[start].first;
        let mut deferred = vec![false; plan.steps.len() + 1];
        let eligible = |plan: &Plan, deferred: &[bool], hardware| {
            accumulation_slot(
                &plan.steps,
                &plan.last_uses,
                &plan.texture_slots,
                deferred,
                &range,
                hardware,
            )
        };
        assert_eq!(
            eligible(&plan, &deferred, true),
            Some(plan.texture_slots[first])
        );
        assert!(eligible(&plan, &deferred, false).is_none());
        deferred[first] = true;
        assert!(eligible(&plan, &deferred, true).is_none());
        deferred[first] = false;
        plan.last_uses[first] += 1;
        assert!(eligible(&plan, &deferred, true).is_none());
        plan.last_uses[first] -= 1;
        let second = plan.steps[start].second;
        plan.steps[start].second = first;
        assert!(eligible(&plan, &deferred, true).is_none());
        plan.steps[start].second = second;
        plan.steps[start].parameters.sources[2].right -= 1;
        assert!(eligible(&plan, &deferred, true).is_none());
        Ok(())
    }

    #[test]
    fn texture_slots_preserve_saved_sources_and_distinct_compute_outputs() -> motion_core::Result {
        use motion_core::NodeModifiers;
        let rectangle = motion_core::Rect::new(0.25, 0.5, 31.5, 63.5);
        let node = motion_core::Node::group([
            motion_core::Node::group(
                (0..12).map(|_| rectangle.fill(motion_core::Color::WHITE.with_alpha(0.2))),
            ),
            motion_core::Node::from(rectangle.fill(motion_core::Color::rgb(30, 100, 220)))
                .filter(motion_core::Filter::Custom(vec![
                    motion_core::FilterOperation::CompositeWithSource {
                        mode: motion_core::BlendMode::Difference,
                    },
                ]))
                .mask(
                    rectangle.fill(motion_core::Color::WHITE),
                    motion_core::MaskMode::Alpha,
                ),
        ]);
        let limits = RasterLimits::default();
        let frame = node
            .compile(
                motion_core::Size::new(32.0, 64.0),
                PixelSize::new(32, 64),
                motion_core::ValidationLimits::default(),
            )?
            .prepare(motion_core::ValidationLimits::default(), limits)?;
        let plan = Plan::new(&frame, limits)?;
        let mut position = 0;
        let mut batches = 0;
        while position < plan.steps.len() {
            let range = plan.texture_run(position);
            let output = plan.texture_slots[range.end];
            for index in range.clone() {
                let step = &plan.steps[index];
                if step.first <= position {
                    if output == plan.texture_slots[step.first] {
                        assert!(plan.texture_blend(position));
                        assert_eq!(plan.last_uses[step.first], position);
                        assert_eq!(
                            plan.steps[step.first - 1].parameters.image_bounds(2),
                            step.parameters.image_bounds(2)
                        );
                    } else {
                        assert_ne!(output, plan.texture_slots[step.first]);
                    }
                }
                assert_ne!(output, plan.texture_slots[step.second]);
                assert_eq!(output, plan.texture_slots[index + 1]);
                let size = image_size(step.parameters.image_bounds(2));
                assert!(
                    plan.texture_sizes[output].width >= size.width
                        && plan.texture_sizes[output].height >= size.height
                );
            }
            batches += usize::from(range.len() > 1);
            position = range.end;
        }
        assert!(batches > 0);
        for (index, step) in plan.steps.iter().enumerate() {
            let output = plan.compute_texture_slots[index + 1];
            assert_ne!(output, plan.compute_texture_slots[step.first]);
            assert_ne!(output, plan.compute_texture_slots[step.second]);
        }
        Ok(())
    }
}
