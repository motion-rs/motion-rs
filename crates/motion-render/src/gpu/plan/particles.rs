//! Separation of per-draw particle records from immutable sprite atlas texels.

use super::*;
use motion_core::ValidationError;

/// Count, sprite width, sprite height and packed stride/blend words.
const INSTANCE_HEADER_WORDS: usize = 4;

/// Low header bits encoding the checked twelve- or sixteen-word record stride.
const INSTANCE_STRIDE_MASK: u32 = 31;

impl Planner<'_> {
    /// Admits the new host atlas before copying; the original upload remains live during the copy.
    pub(in crate::gpu) fn particle_resource(
        &mut self,
        data: &mut Vec<f32>,
    ) -> Result<Option<Vec<f32>>, ValidationError> {
        let invalid = || {
            ValidationError::invalid(
                "gpu.particles",
                "upload",
                "Particle records exceed their checked upload",
            )
        };
        if data.len() < INSTANCE_HEADER_WORDS {
            return Err(invalid());
        }
        let stride = (data[3] as u32 & INSTANCE_STRIDE_MASK) as usize;
        let first = (data[0] as usize)
            .checked_mul(stride)
            .and_then(|words| words.checked_add(INSTANCE_HEADER_WORDS))
            .ok_or_else(invalid)?;
        let atlas = data.get(first..).ok_or_else(invalid)?;
        if atlas.is_empty() {
            return Ok(None);
        }
        let mut words = self.data(atlas.len(), 0.0)?;
        words.copy_from_slice(atlas);
        let bytes = atlas.len() as u64 * size_of::<f32>() as u64;
        data.truncate(first);
        // Logical words, not vector capacity: per-draw staging and device uploads keep only records.
        self.budget.release(bytes);
        Ok(Some(words))
    }
}
