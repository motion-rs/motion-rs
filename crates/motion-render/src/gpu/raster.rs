//! GPU-side preparation: device coverage admission and the GPU scene result.
//!
//! The shared raster frontend in `crate::raster` supplies normalized geometry and scene walking.
//! This module owns everything that exists only for the GPU backend.

pub(crate) mod device;
pub(crate) mod prepare;
