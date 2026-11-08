//! Checked particle state, tile metadata and ordered index storage layout.

use super::*;
use motion_core::ValidationError;

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
        let columns = (bounds.right - bounds.left).div_ceil(16);
        let rows = (bounds.bottom - bounds.top).div_ceil(16);
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
