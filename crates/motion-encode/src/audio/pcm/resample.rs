//! Source-rate conversion into the mix rate.

#[cfg(feature = "audio")]
use super::{MAX_SAMPLES, invalid};

/// Sample-rate conversion used when a source rate differs from the mix rate.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum ResampleQuality {
    /// Fixed-ratio FFT resampler; fast and the default (requires feature `audio`).
    #[default]
    Fast,
    /// Windowed-sinc interpolation with 32 zero crossings per side; slower, any rates.
    Sinc,
}

#[cfg(feature = "audio")]
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

    use rubato::Resampler;
    let chunk = (source_rate as usize).clamp(1, 1024);
    let mut common = source_rate;
    let mut remainder = rate;
    while remainder != 0 {
        let next = common % remainder;
        common = remainder;
        remainder = next;
    }
    let unit = (source_rate / common) as usize;
    let blocks = chunk.div_ceil(unit * 2) * 2;
    let chunk = blocks * unit;
    let output_block = blocks * (rate / common) as usize;
    if output_block.saturating_mul(channels).saturating_mul(16) > MAX_SAMPLES {
        return Err(invalid(
            "resample_scratch",
            "Resampler block exceeds scratch allowance",
        )
        .into());
    }
    let mut resampler =
        rubato::FftFixedInOut::<f32>::new(source_rate as usize, rate as usize, chunk, channels)
            .map_err(|error| motion_core::Error::Other(Box::new(error)))?;
    let chunk = resampler.input_frames_next();
    let delay = resampler.output_delay();
    if (count + delay + resampler.output_frames_max()).saturating_mul(channels) > MAX_SAMPLES {
        return Err(invalid(
            "resample_bytes",
            "Resampling output/scratch exceeds allowance",
        )
        .into());
    }
    let mut result = vec![Vec::new(); channels];
    let mut start = 0;
    while result[0].len() < count + delay {
        let mut block = vec![vec![0.0; chunk]; channels];
        for (channel, samples) in block.iter_mut().enumerate() {
            for (frame, sample) in samples.iter_mut().enumerate() {
                *sample = input
                    .get((start + frame) * channels + channel)
                    .copied()
                    .unwrap_or(0.0);
            }
        }
        let output = resampler
            .process(&block, None)
            .map_err(|error| motion_core::Error::Other(Box::new(error)))?;
        for (result, output) in result.iter_mut().zip(output) {
            result.extend(output);
        }
        start += chunk;
    }
    let mut values = Vec::with_capacity(count * channels);
    values.extend(
        (delay..delay + count).flat_map(|frame| result.iter().map(move |samples| samples[frame])),
    );
    Ok(values)
}
