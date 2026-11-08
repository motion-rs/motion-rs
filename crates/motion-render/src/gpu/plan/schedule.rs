//! Lifetime analysis and shared GPU image-slot allocation.

use super::*;
use motion_core::ValidationError;

pub(in crate::gpu) fn bounded_fill(step: &Step) -> bool {
    if step.parameters.operation.code != GpuOp::Fill as u32
        || step.parameters.operation.blend_space != 0
        || step.quads.is_empty()
    {
        return false;
    }
    let bounded = |value: f32| value.is_finite() && value.abs() <= 65536.0;
    if step.parameters.operation.kind == 0 {
        return step.parameters.values[..4]
            .iter()
            .all(|word| bounded(*word));
    }
    if matches!(step.data[0].to_bits(), SPARSE_COVERAGE | DIRECT_COVERAGE) {
        return step.data[step.data[1].to_bits() as usize..]
            .iter()
            .copied()
            .all(bounded);
    }
    let first = step.data[0].to_bits();
    let second = step.data[1].to_bits();
    let count = u64::from(step.data[2].to_bits() - (first & 0x7fffffff))
        * u64::from(step.data[3].to_bits() - (second & 0x7fffffff));
    let coverage_words = if first & 0x80000000 != 0 {
        5
    } else if second & 0x80000000 != 0 {
        4 + count.div_ceil(4) as usize
    } else {
        4 + count as usize
    };
    step.data[coverage_words..].iter().copied().all(bounded)
}

impl Plan {
    pub(in crate::gpu) fn last_uses(
        steps: &[Step],
        output: usize,
        budget: &mut Budget,
    ) -> Result<Vec<usize>, ValidationError> {
        budget.work(steps.len() as u64 * 4 + 1, "gpu.lifetimes")?;
        let mut last = Vec::new();
        last.try_reserve_exact(steps.len() + 1).map_err(|_| {
            ValidationError::limit(
                "gpu.lifetimes",
                "max_bytes",
                "Could not reserve GPU image schedule",
            )
        })?;
        last.extend((0..=steps.len()).map(|image| image.saturating_sub(1)));
        last[0] = steps.len();
        last[output] = steps.len();
        for (position, step) in steps.iter().enumerate() {
            last[step.first] = last[step.first].max(position);
            last[step.second] = last[step.second].max(position);
        }
        Ok(last)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fill_batches_respect_color_space_hdr_saved_inputs_and_admission() -> motion_core::Result {
        let node = motion_core::Node::from(
            motion_core::Rect::new(0.25, 0.5, 60.0, 62.0).fill(motion_core::Color::WHITE),
        );
        let limits = RasterLimits::default();
        let frame = motion_core::Node::group([node.clone(), node.clone(), node])
            .compile(
                motion_core::Size::new(64.0, 64.0),
                PixelSize::new(64, 64),
                motion_core::ValidationLimits::default(),
            )?
            .prepare(motion_core::ValidationLimits::default(), limits)?;
        let mut plan = Plan::new(&frame, limits)?;
        let start = plan
            .steps
            .iter()
            .position(|step| step.parameters.operation.code == GpuOp::Fill as u32)
            .unwrap();
        assert_eq!(plan.texture_run(start).len(), 3);
        // Every image of a chain of bounded fills is known to be finite, so no check pass is needed.
        assert!(
            plan.texture_bounded[start..=start + 3]
                .iter()
                .all(|bounded| *bounded)
        );
        let saved = plan.last_uses[start + 1];
        plan.last_uses[start + 1] += 1;
        assert_eq!(plan.texture_run(start).len(), 1);
        plan.last_uses[start + 1] = saved;
        plan.steps[start + 1].parameters.operation.blend_space = 1;
        (plan.texture_blends, plan.texture_bounded) =
            Plan::texture_eligibility(&plan.steps, &mut Budget::new(frame.size, limits)?)?;
        assert_eq!(plan.texture_run(start).len(), 1);
        plan.steps[start + 1].parameters.operation.blend_space = 0;
        plan.steps[start + 1].parameters.values[0] = 1e30_f32;
        (plan.texture_blends, plan.texture_bounded) =
            Plan::texture_eligibility(&plan.steps, &mut Budget::new(frame.size, limits)?)?;
        assert_eq!(plan.texture_run(start).len(), 1);
        // An unbounded fill makes its output, and everything built on it, unproven.
        assert!(plan.texture_bounded[start]);
        assert!(
            !plan.texture_bounded[start + 2..=start + 3]
                .iter()
                .any(|bounded| *bounded)
        );
        let constrained = RasterLimits {
            max_bytes: plan.stats.peak_bytes - 1,
            ..limits
        };
        assert!(Plan::new(&frame, constrained).is_err());
        Ok(())
    }
}
