//! Portable visual-check metadata attached to project registrations.

use std::collections::BTreeSet;

use crate::render::Composition;
use crate::{Rect, ValidationError};

/// A named, integer-pixel region used for automated nonblank checks.
#[derive(Clone, Debug)]
pub struct VisualRegion {
    /// Human-readable label included in reports and failures.
    pub name: String,
    /// Output-pixel bounds; all edges must be integers inside the composition.
    pub bounds: Rect,
    /// Required pixels different from the output's top-left reference pixel.
    pub min_nonblank_pixels: usize,
}

impl VisualRegion {
    /// Stores region metadata without painting. Defaults to 101 nonblank pixels.
    pub fn new(name: impl Into<String>, bounds: Rect) -> Self {
        Self {
            name: name.into(),
            bounds,
            min_nonblank_pixels: 101,
        }
    }
}

/// Metadata for automated keyframe checks; not a human visual acceptance result.
/// Constructing or attaching this policy does not render frames or execute checks;
/// register it with [`ProjectBuilder::visual_check`](crate::ProjectBuilder::visual_check).
#[derive(Clone, Debug)]
pub struct VisualCheckPolicy {
    /// Between 2 and 1024 distinct indices inside the composition, in sampling order.
    /// Sorting is not required; the supplied first/last entries are compared for motion.
    pub frames: Vec<u32>,
    /// At most 64 named output-pixel regions whose content must be nonblank.
    pub regions: Vec<VisualRegion>,
    /// Allowed maximum per-channel CPU/GPU RGB difference.
    pub max_rgb_difference: u8,
    /// Minimum changed pixels between first and last sampled frames; zero permits static art.
    pub min_changed_pixels: usize,
}

impl VisualCheckPolicy {
    /// Stores bounded-check metadata; bounds are validated by [`Self::preflight`].
    /// Defaults to RGB tolerance 3 and at least 101 first/last changed pixels.
    pub fn new(frames: impl Into<Vec<u32>>, regions: impl Into<Vec<VisualRegion>>) -> Self {
        Self {
            frames: frames.into(),
            regions: regions.into(),
            max_rgb_difference: 3,
            min_changed_pixels: 101,
        }
    }

    /// Checks everything that does not depend on the composition: sampled frame count and
    /// uniqueness, and the region count. Called when the policy is registered.
    pub(crate) fn check_structure(&self) -> Result<(), ValidationError> {
        let invalid =
            |field: &str, message: &str| ValidationError::invalid("visual_check", field, message);
        if !(2..=1024).contains(&self.frames.len()) {
            return Err(invalid("frames", "Require 2..=1024 sampled frames"));
        }
        let unique: BTreeSet<_> = self.frames.iter().copied().collect();
        if unique.len() != self.frames.len() {
            return Err(invalid("frames", "Require distinct sampled frame indices"));
        }
        if self.regions.len() > 64 {
            return Err(invalid("regions", "At most 64 regions are allowed"));
        }
        Ok(())
    }

    /// Checks the policy against the composition it will sample, without rendering.
    pub fn preflight(&self, composition: &Composition) -> Result<(), ValidationError> {
        self.check_structure()?;
        let invalid =
            |field: &str, message: &str| ValidationError::invalid("visual_check", field, message);
        let size = composition.output_size();
        if self
            .frames
            .iter()
            .any(|index| u64::from(*index) >= composition.frame_count())
        {
            return Err(invalid(
                "frames",
                "Require every sampled index inside the composition",
            ));
        }
        for (index, region) in self.regions.iter().enumerate() {
            let bounds = region.bounds;
            if region.name.is_empty()
                || region.name.len() > 256
                || [bounds.left, bounds.top, bounds.right, bounds.bottom]
                    .iter()
                    .any(|value| !value.is_finite() || value.fract() != 0.0)
                || bounds.left < 0.0
                || bounds.top < 0.0
                || bounds.width() <= 0.0
                || bounds.height() <= 0.0
                || bounds.right > f64::from(size.width)
                || bounds.bottom > f64::from(size.height)
                || region.min_nonblank_pixels > bounds.width() as usize * bounds.height() as usize
            {
                return Err(invalid(
                    &format!("regions[{index}]"),
                    "Require a named integer-pixel region inside the output with an attainable nonblank threshold",
                ));
            }
        }
        if self.min_changed_pixels > size.width as usize * size.height as usize {
            return Err(invalid(
                "min_changed_pixels",
                "Changed-pixel threshold exceeds the output pixel count",
            ));
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn composition() -> Composition {
        Composition::new(16, 8, crate::render::FrameClock::new(crate::fps!(30), 7))
    }

    fn policy() -> VisualCheckPolicy {
        VisualCheckPolicy::new(
            vec![0, 6],
            vec![VisualRegion::new("art", Rect::new(0.0, 0.0, 16.0, 8.0))],
        )
    }

    #[test]
    fn policies_validate_frames_and_pixel_regions_against_the_composition() {
        assert!(policy().preflight(&composition()).is_ok());
        let mut invalid = policy();
        invalid.frames = vec![0, 7];
        assert_eq!(
            invalid.preflight(&composition()).unwrap_err().field,
            "frames"
        );
        invalid.frames = vec![0, 0];
        assert!(invalid.check_structure().is_err());
        let mut invalid = policy();
        invalid.regions[0].bounds.right = 17.0;
        assert_eq!(
            invalid.preflight(&composition()).unwrap_err().field,
            "regions[0]"
        );
        let mut fractional = policy();
        fractional.regions[0].bounds.left = 0.125;
        assert_eq!(
            fractional.preflight(&composition()).unwrap_err().field,
            "regions[0]"
        );
    }
}
