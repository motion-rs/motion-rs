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
