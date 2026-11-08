//! Deterministic windowed-sinc resampling (Blackman-Harris, 32 zero crossings per side).
//!
//! Output frame `j` samples the source at `j * source_rate / rate`; no delay is introduced.
//! Frames outside the supplied input are silence, so the final few output frames of a
//! window see a truncated kernel. The kernel stretches when downsampling to suppress aliasing.

use super::pcm::{MAX_SAMPLES, invalid};

const ZERO_CROSSINGS: usize = 32;

const MAX_CACHED_PHASES: u64 = 65_536;

fn gcd(mut a: u64, mut b: u64) -> u64 {
    while b != 0 {
        (a, b) = (b, a % b);
    }
    a
}

fn weights(offset: f64, scale: f64, half: i64) -> Vec<f64> {
    let width = half as f64;
    let mut values: Vec<f64> = (1 - half..=half)
        .map(|tap| {
            let distance = tap as f64 - offset;
            let phase = (distance / width + 1.0) * 0.5;
            let window = if (0.0..=1.0).contains(&phase) {
                let angle = 2.0 * std::f64::consts::PI * phase;
                0.358_75 - 0.488_29 * libm::cos(angle) + 0.141_28 * libm::cos(2.0 * angle)
                    - 0.011_68 * libm::cos(3.0 * angle)
            } else {
                0.0
            };
            let x = std::f64::consts::PI * scale * distance;
            let sinc = if x.abs() < 1e-12 {
                1.0
            } else {
                libm::sin(x) / x
            };
            sinc * window
        })
        .collect();
    let sum: f64 = values.iter().sum();
    if sum.abs() > 1e-12 {
        values.iter_mut().for_each(|value| *value /= sum);
    }
    values
}

/// Resamples interleaved `input` to exactly `count` frames at `rate`.
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
    if count.saturating_mul(channels) > MAX_SAMPLES {
        return Err(invalid("resample_bytes", "Resampling output exceeds allowance").into());
    }
    let scale = (f64::from(rate) / f64::from(source_rate)).min(1.0);
    let half = (ZERO_CROSSINGS as f64 / scale).ceil() as i64;
    let frames = (input.len() / channels) as i64;
    let phases = u64::from(rate) / gcd(u64::from(source_rate), u64::from(rate));
    let mut cache: Vec<Option<Vec<f64>>> = if phases <= MAX_CACHED_PHASES {
        vec![None; phases as usize]
    } else {
        Vec::new()
    };
    let mut output = Vec::with_capacity(count * channels);
    let mut accumulators = vec![0.0f64; channels];
    for frame in 0..count as u64 {
        let numerator = frame * u64::from(source_rate);
        let base = (numerator / u64::from(rate)) as i64;
        let remainder = numerator % u64::from(rate);
        let offset = remainder as f64 / f64::from(rate);
        let fresh;
        let kernel = if cache.is_empty() {
            fresh = weights(offset, scale, half);
            &fresh
        } else {
            let slot =
                &mut cache[(remainder / gcd(u64::from(source_rate), u64::from(rate))) as usize];
            slot.get_or_insert_with(|| weights(offset, scale, half))
        };
        accumulators.fill(0.0);
        for (index, weight) in kernel.iter().enumerate() {
            let position = base + 1 - half + index as i64;
            if (0..frames).contains(&position) {
                let start = position as usize * channels;
                for (accumulator, sample) in accumulators.iter_mut().zip(&input[start..]) {
                    *accumulator += weight * f64::from(*sample);
                }
            }
        }
        output.extend(accumulators.iter().map(|value| *value as f32));
    }
    Ok(output)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tone(rate: u32, hertz: f64, frames: usize) -> Vec<f32> {
        (0..frames)
            .map(|frame| {
                libm::sin(2.0 * std::f64::consts::PI * hertz * frame as f64 / f64::from(rate))
                    as f32
            })
            .collect()
    }

    #[test]
    fn upsampling_preserves_a_tone() {
        let input = tone(44_100, 1000.0, 8000);
        let output = resample(&input, 1, 44_100, 48_000, 8000).unwrap();
        let expected = tone(48_000, 1000.0, 8000);
        let error = (500..7000)
            .map(|frame| (output[frame] - expected[frame]).abs())
            .fold(0.0, f32::max);
        assert!(error < 1e-3, "{error}");
    }

    #[test]
    fn downsampling_rejects_aliasing_content() {
        let input = tone(48_000, 12_000.0, 6000);
        let output = resample(&input, 1, 48_000, 16_000, 2000).unwrap();
        assert!(output[300..1700].iter().all(|value| value.abs() < 0.01));
        let passband = resample(&tone(48_000, 1000.0, 6000), 1, 48_000, 16_000, 2000).unwrap();
        assert!(passband[300..1700].iter().any(|value| value.abs() > 0.95));
    }

    #[test]
    fn stereo_is_deterministic_and_deinterleaved() {
        let mono = tone(8000, 300.0, 800);
        let stereo: Vec<f32> = mono.iter().flat_map(|value| [*value, -*value]).collect();
        let first = resample(&stereo, 2, 8000, 11_025, 1000).unwrap();
        assert_eq!(first, resample(&stereo, 2, 8000, 11_025, 1000).unwrap());
        assert!(first.chunks(2).all(|pair| pair[0] == -pair[1]));
        let left = resample(&mono, 1, 8000, 11_025, 1000).unwrap();
        assert_eq!(first[0], left[0]);
        assert_eq!(first[600], left[300]);
    }
}
