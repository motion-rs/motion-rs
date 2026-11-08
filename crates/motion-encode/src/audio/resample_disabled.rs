//! Resampling fallback used when the `audio` feature is disabled; only same-rate input is
//! accepted and any rate conversion is reported as a validation error.

use motion_core::{Error, ValidationError};

pub(super) fn resample(
    input: &[f32],
    channels: usize,
    source_rate: u32,
    rate: u32,
    count: usize,
) -> motion_core::Result<Vec<f32>> {
    if source_rate == rate {
        return Ok(input[..count * channels].to_vec());
    }
    Err(Error::from(ValidationError::unsupported(
        "audio",
        "resample",
        "Audio resampling requires the audio feature",
    )))
}
