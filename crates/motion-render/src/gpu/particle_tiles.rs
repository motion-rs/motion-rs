//! Checked particle state, tile metadata and ordered index storage layout.

use super::*;
use motion_core::ValidationError;

/// Width and height of an ordered particle tile, in output pixels.
const PARTICLE_TILE_SIZE: u32 = 16;

/// Number of particle membership bits stored in one mask word.
const PARTICLES_PER_MASK_WORD: u32 = 32;

/// Conservative work units for converting one particle support to device bounds.
const PARTICLE_MASK_SETUP_WORK_PER_INSTANCE: u64 = 128;

/// Uniform mode selecting ordered particle-index bitsets.
pub(super) const PARTICLE_TILE_BITSET: u32 = 2;

pub(super) struct ParticleTiles {
    pub(super) count: u32,
    pub(super) tiles: u32,
    pub(super) summary: u64,
    pub(super) bytes: u64,
}

impl ParticleTiles {
    pub(super) fn new(bounds: PixelRect, count: u32) -> Result<Self, ValidationError> {
        if count as usize > motion_core::instances::MAX_INSTANCE_COUNT {
            return Err(ValidationError::limit(
                "gpu.particle_tiles",
                "instances",
                "Particle tile count exceeds the instance protocol",
            ));
        }
        let columns = (bounds.right - bounds.left).div_ceil(PARTICLE_TILE_SIZE);
        let rows = (bounds.bottom - bounds.top).div_ceil(PARTICLE_TILE_SIZE);
        let tiles = columns.checked_mul(rows).ok_or_else(|| {
            ValidationError::limit(
                "gpu.particle_tiles",
                "max_bytes",
                "Particle tile count overflow",
            )
        })?;
        let summary = (u64::from(count) * 3 + u64::from(tiles)) * 16;
        Ok(Self {
            count,
            tiles,
            summary,
            bytes: summary + 16,
        })
    }

    pub(super) fn state_bytes(&self) -> u64 {
        (u64::from(self.count) * 48).max(16)
    }

    pub(super) fn mask_bytes(&self) -> Option<u64> {
        let words = u64::from(self.count.div_ceil(PARTICLES_PER_MASK_WORD));
        u64::from(self.tiles)
            .checked_mul(words)?
            .checked_mul(4)
            .map(|bytes| bytes.max(4))
    }

    pub(super) fn mask_composition_work(&self, pixels: u64) -> Option<u64> {
        let words = u64::from(self.count.div_ceil(PARTICLES_PER_MASK_WORD));
        let per_pixel = u64::from(self.count).checked_mul(64)?.checked_add(words)?;
        pixels.checked_mul(per_pixel)
    }

    pub(super) fn mask_build_work(&self) -> Option<u64> {
        let overlap_work = u64::from(self.tiles).checked_mul(u64::from(self.count) * 32 + 64)?;
        let setup_work =
            u64::from(self.count).checked_mul(PARTICLE_MASK_SETUP_WORK_PER_INSTANCE)?;
        overlap_work.checked_add(setup_work)
    }

    pub(super) fn exceeds_device_limits(&self, limits: &wgpu::Limits, bitset: bool) -> bool {
        let storage_limit = limits
            .max_storage_buffer_binding_size
            .min(limits.max_buffer_size);
        let state_bytes = if bitset {
            self.state_bytes()
        } else {
            self.bytes
        };
        let tile_groups = if bitset {
            1
        } else {
            self.tiles.div_ceil(64).max(1)
        };
        state_bytes > storage_limit
            || (bitset && self.mask_bytes().is_none_or(|bytes| bytes > storage_limit))
            || tile_groups > limits.max_compute_workgroups_per_dimension
            || self.count.div_ceil(64) > limits.max_compute_workgroups_per_dimension
    }

    pub(super) fn admit(
        &self,
        summary: [u32; 4],
        budget: &mut Budget,
        device_limit: u64,
    ) -> Result<u64, ValidationError> {
        if summary[3] != 0 {
            return Err(ValidationError::limit(
                "gpu.particle_tiles",
                "max_bytes",
                "Particle index prefix overflow",
            ));
        }
        let references = u64::from(summary[0]) * 16;
        let bytes = self
            .bytes
            .checked_add(references)
            .filter(|bytes| *bytes <= device_limit)
            .ok_or_else(|| {
                ValidationError::limit(
                    "gpu.particle_tiles",
                    "max_bytes",
                    "Particle indices exceed device storage addressing",
                )
            })?;
        budget.work(
            u64::from(summary[1]) | (u64::from(summary[2]) << 32),
            "gpu.particle_tiles",
        )?;
        budget.bytes(references, "gpu.particle_tiles")?;
        Ok(bytes)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tile_layout_and_dynamic_work_are_checked() -> Result<(), ValidationError> {
        let layout = ParticleTiles::new(
            PixelRect {
                left: 3,
                top: 5,
                right: 20,
                bottom: 38,
            },
            10_000,
        )?;
        assert_eq!(layout.tiles, 6);
        assert_eq!(layout.summary, (30_000 + 6) * 16);
        assert_eq!(layout.state_bytes(), 10_000 * 48);
        assert_eq!(layout.mask_bytes(), Some(6 * 313 * 4));
        assert_eq!(
            layout.mask_composition_work(33 * 17),
            Some(33 * 17 * (10_000 * 64 + 313))
        );
        assert_eq!(
            layout.mask_build_work(),
            Some(6 * (10_000 * 32 + 64) + 10_000 * 128)
        );
        let mut budget = Budget::new(PixelSize::new(32, 48), RasterLimits::default())?;
        assert_eq!(
            layout.admit([12, 42, 0, 0], &mut budget, u64::MAX)?,
            layout.bytes + 192
        );
        assert_eq!(budget.work_done(), 42);
        assert_eq!(budget.live_bytes(), 192);
        assert!(layout.admit([0, 0, 0, 1], &mut budget, u64::MAX).is_err());
        assert!(
            layout
                .admit([12, 0, 0, 0], &mut budget, layout.bytes)
                .is_err()
        );
        assert_eq!(
            layout.admit([0, 0, 1, 0], &mut budget, u64::MAX)?,
            layout.bytes
        );
        assert_eq!(budget.work_done(), 42 + (1_u64 << 32));
        assert!(
            layout
                .admit([0, u32::MAX, u32::MAX, 0], &mut budget, u64::MAX)
                .is_err()
        );
        assert_eq!(budget.work_done(), 42 + (1_u64 << 32));
        let mut sandbox = Budget::new(PixelSize::new(32, 48), RasterLimits::sandbox())?;
        assert_eq!(
            layout
                .admit([0, 0, 1, 0], &mut sandbox, u64::MAX)
                .unwrap_err()
                .field,
            "max_work"
        );
        assert_eq!(sandbox.work_done(), 0);
        let mut bytes = Budget::new(PixelSize::new(32, 48), RasterLimits::default())?;
        bytes.limits.max_bytes = 16;
        assert_eq!(
            layout
                .admit([2, 0, 0, 0], &mut bytes, u64::MAX)
                .unwrap_err()
                .field,
            "max_bytes"
        );
        let mut work = Budget::new(PixelSize::new(32, 48), RasterLimits::default())?;
        work.limits.max_work = 1;
        assert_eq!(
            layout
                .admit([0, 2, 0, 0], &mut work, u64::MAX)
                .unwrap_err()
                .field,
            "max_work"
        );
        assert!(
            ParticleTiles::new(
                PixelRect {
                    left: 0,
                    top: 0,
                    right: 16,
                    bottom: 16
                },
                motion_core::instances::MAX_INSTANCE_COUNT as u32 + 1
            )
            .is_err()
        );
        assert!(
            ParticleTiles::new(
                PixelRect {
                    left: 0,
                    top: 0,
                    right: 16,
                    bottom: 16
                },
                motion_core::instances::MAX_INSTANCE_COUNT as u32,
            )
            .is_ok()
        );
        Ok(())
    }
}
