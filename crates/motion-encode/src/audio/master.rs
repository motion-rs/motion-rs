//! Offline gated loudness, reconstructed peaks and deterministic master limiting.

use super::{
    effects::{Biquad, amplitude},
    pcm::invalid,
};
use motion_core::sync::MutexExt;
use motion_core::{AudioSource, Master};
use std::{
    collections::VecDeque,
    hash::{Hash, Hasher},
    sync::{Arc, Mutex, OnceLock},
};

/// Measured stereo output levels. Silence has no integrated LUFS value.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct LoudnessReport {
    /// EBU R128 absolute/relative gated integrated loudness.
    pub integrated_lufs: Option<f64>,
    /// Largest stored sample magnitude.
    pub sample_peak: f64,
    /// Four-times windowed-sinc reconstructed peak magnitude.
    pub true_peak: f64,
    /// Whole-output normalized cross-channel correlation; None for silent channels.
    pub correlation: Option<f64>,
}

struct CachedReport {
    key: u64,
    rate: u32,
    samples: Arc<[f32]>,
    report: LoudnessReport,
}

fn report_cache() -> &'static Mutex<VecDeque<CachedReport>> {
    static CACHE: OnceLock<Mutex<VecDeque<CachedReport>>> = OnceLock::new();
    CACHE.get_or_init(|| Mutex::new(VecDeque::new()))
}

fn content_key(samples: &[f32], rate: u32) -> u64 {
    let mut hash = std::collections::hash_map::DefaultHasher::new();
    rate.hash(&mut hash);
    samples.len().hash(&mut hash);
    for sample in samples {
        sample.to_bits().hash(&mut hash);
    }
    hash.finish()
}

fn cached_report(samples: &[f32], rate: u32, key: u64) -> Option<LoudnessReport> {
    report_cache()
        .lock_unpoisoned()
        .iter()
        .find(|entry| {
            entry.key == key
                && entry.rate == rate
                && entry.samples.len() == samples.len()
                && entry
                    .samples
                    .iter()
                    .zip(samples)
                    .all(|(first, second)| first.to_bits() == second.to_bits())
        })
        .map(|entry| entry.report)
}

fn k_filters(rate: u32) -> [Biquad; 2] {
    let tangent = libm::tan(std::f64::consts::PI * 1681.974450955533 / f64::from(rate));
    let quality = 0.7071752369554196;
    let high = amplitude(3.999843853973347);
    let middle = libm::pow(high, 0.4996667741545416);
    let denominator = 1.0 + tangent / quality + tangent * tangent;
    let shelf = Biquad::new([
        (high + middle * tangent / quality + tangent * tangent) / denominator,
        2.0 * (tangent * tangent - high) / denominator,
        (high - middle * tangent / quality + tangent * tangent) / denominator,
        2.0 * (tangent * tangent - 1.0) / denominator,
        (1.0 - tangent / quality + tangent * tangent) / denominator,
    ]);
    let tangent = libm::tan(std::f64::consts::PI * 38.13547087602444 / f64::from(rate));
    let quality = 0.5003270373238773;
    let denominator = 1.0 + tangent / quality + tangent * tangent;
    [
        shelf,
        Biquad::new([
            1.0,
            -2.0,
            1.0,
            2.0 * (tangent * tangent - 1.0) / denominator,
            (1.0 - tangent / quality + tangent * tangent) / denominator,
        ]),
    ]
}

fn decibels(energy: f64) -> f64 {
    -0.691 + 10.0 * libm::log10(energy.max(f64::MIN_POSITIVE))
}

pub(crate) fn report(samples: &[f32], rate: u32) -> LoudnessReport {
    let key = content_key(samples, rate);
    if let Some(report) = cached_report(samples, rate, key) {
        return report;
    }
    let report = measure(samples, rate);
    let bytes = samples.len().saturating_mul(4);
    if bytes <= 64 * 1024 * 1024 {
        let mut cache = report_cache().lock_unpoisoned();
        while cache.len() >= 16
            || cache
                .iter()
                .map(|entry| entry.samples.len() * 4)
                .sum::<usize>()
                .saturating_add(bytes)
                > 64 * 1024 * 1024
        {
            cache.pop_front();
        }
        cache.push_back(CachedReport {
            key,
            rate,
            samples: samples.to_vec().into(),
            report,
        });
    }
    report
}

fn measure(samples: &[f32], rate: u32) -> LoudnessReport {
    let sample_peak = samples
        .iter()
        .map(|value| f64::from(value.abs()))
        .fold(0.0, f64::max);
    let mut filters = [k_filters(rate); 2];
    let block = (f64::from(rate) * 0.4).round().max(1.0) as usize;
    let hop = (f64::from(rate) * 0.1).round().max(1.0) as usize;
    let mut ring = vec![0.0; block];
    let mut energy = 0.0;
    let mut blocks = Vec::new();
    let mut correlation = [0.0; 3];
    for (index, frame) in samples.as_chunks::<2>().0.iter().enumerate() {
        let mut power = 0.0;
        for (sample, filters) in frame.iter().zip(&mut filters) {
            let value = filters[0].sample(f64::from(*sample));
            let value = filters[1].sample(value);
            power += value * value;
        }
        let cursor = index % block;
        energy += power - ring[cursor];
        ring[cursor] = power;
        if index + 1 >= block && (index + 1 - block).is_multiple_of(hop) {
            blocks.push((energy / block as f64).max(0.0));
        }
        correlation[0] += f64::from(frame[0]) * f64::from(frame[1]);
        correlation[1] += f64::from(frame[0]).powi(2);
        correlation[2] += f64::from(frame[1]).powi(2);
    }
    let gated: Vec<_> = blocks
        .into_iter()
        .filter(|energy| decibels(*energy) >= -70.0)
        .collect();
    let integrated_lufs = if gated.is_empty() {
        None
    } else {
        let threshold = decibels(gated.iter().sum::<f64>() / gated.len() as f64) - 10.0;
        let relative: Vec<_> = gated
            .into_iter()
            .filter(|energy| decibels(*energy) >= threshold)
            .collect();
        Some(decibels(
            relative.iter().sum::<f64>() / relative.len() as f64,
        ))
    };
    LoudnessReport {
        integrated_lufs,
        sample_peak,
        true_peak: reconstructed_peak(samples),
        correlation: (correlation[1] > 0.0 && correlation[2] > 0.0).then(|| {
            (correlation[0] / libm::sqrt(correlation[1] * correlation[2])).clamp(-1.0, 1.0)
        }),
    }
}

pub(crate) fn reconstructed_peak(samples: &[f32]) -> f64 {
    let mut peak = samples
        .iter()
        .map(|value| f64::from(value.abs()))
        .fold(0.0, f64::max);
    let kernels = [0.25, 0.5, 0.75].map(|phase| {
        let mut weights = [0.0; 32];
        for (index, weight) in weights.iter_mut().enumerate() {
            let offset = index as f64 - 15.0 - phase;
            let angle = std::f64::consts::PI * offset;
            let window = 0.42
                + 0.5 * libm::cos(std::f64::consts::PI * offset / 16.0)
                + 0.08 * libm::cos(std::f64::consts::TAU * offset / 16.0);
            *weight = libm::sin(angle) / angle * window;
        }
        let sum: f64 = weights.iter().sum();
        for weight in &mut weights {
            *weight /= sum;
        }
        weights
    });
    let frames = samples.len() / 2;
    for frame in 0..frames {
        // Interior windows read every tap, so they skip the edge tests. Each kernel/channel
        // accumulator still adds taps in index order, keeping the sums bit-identical.
        if frame >= 15 && frame + 17 <= frames {
            let mut values = [[0.0_f64; 2]; 3];
            let window = &samples[(frame - 15) * 2..(frame + 17) * 2];
            for (index, pair) in window.as_chunks::<2>().0.iter().enumerate() {
                for (kernel, value) in kernels.iter().zip(&mut values) {
                    value[0] += f64::from(pair[0]) * kernel[index];
                    value[1] += f64::from(pair[1]) * kernel[index];
                }
            }
            for value in values.iter().flatten() {
                peak = peak.max(value.abs());
            }
            continue;
        }
        for kernel in &kernels {
            for channel in 0..2 {
                let mut value = 0.0;
                for (index, weight) in kernel.iter().enumerate() {
                    let position = frame as i64 + index as i64 - 15;
                    if position >= 0 {
                        value += f64::from(
                            samples
                                .get(position as usize * 2 + channel)
                                .copied()
                                .unwrap_or(0.0),
                        ) * weight;
                    }
                }
                peak = peak.max(value.abs());
            }
        }
    }
    peak
}

pub(crate) fn process(samples: &mut [f32], rate: u32, master: Master) -> motion_core::Result {
    master.validate()?;
    if master.normalize.is_some() && rate < 8000 {
        return Err(invalid(
            "loudness_rate",
            "K-weighted measurement requires at least 8000 Hz",
        )
        .into());
    }
    let mut gain = amplitude(master.gain_db);
    if let Some(normalize) = master.normalize {
        let levels = report(samples, rate);
        if let Some(loudness) = levels.integrated_lufs {
            gain *= amplitude(normalize.target_lufs - loudness);
            if levels.true_peak > 0.0 {
                gain = gain.min(amplitude(normalize.true_peak_db) / levels.true_peak);
            }
        }
    }
    for sample in samples.iter_mut() {
        *sample = (f64::from(*sample) * gain) as f32;
    }
    if let Some(limiter) = master.limiter {
        let ceiling = amplitude(limiter.ceiling_db);
        let lookahead = motion_core::audio::ceil_frames(limiter.lookahead, rate) as usize;
        let release = motion_core::audio::ceil_frames(limiter.release, rate).max(1) as f64;
        let frames = samples.len() / 2;
        let mut envelope: Vec<f64> = samples
            .as_chunks::<2>()
            .0
            .iter()
            .map(|frame| {
                (ceiling / f64::from(frame[0].abs().max(frame[1].abs())).max(f64::MIN_POSITIVE))
                    .min(1.0)
            })
            .collect();
        if lookahead > 0 {
            for frame in (0..frames.saturating_sub(1)).rev() {
                envelope[frame] = envelope[frame].min(envelope[frame + 1] + 1.0 / lookahead as f64);
            }
        }
        let mut gain = envelope.first().copied().unwrap_or(1.0);
        for frame in 0..frames {
            gain = (gain + 1.0 / release).min(envelope[frame]);
            for sample in &mut samples[frame * 2..frame * 2 + 2] {
                *sample = (f64::from(*sample) * gain) as f32;
            }
        }
        let peak = reconstructed_peak(samples);
        if peak > ceiling {
            let correction = ceiling / peak * (1.0 - 2.0 * f64::from(f32::EPSILON));
            for sample in samples.iter_mut() {
                *sample = (f64::from(*sample) * correction) as f32;
            }
        }
    }
    if samples.iter().any(|sample| !sample.is_finite()) {
        return Err(invalid("master_output", "Master produced nonfinite PCM").into());
    }
    Ok(())
}

impl crate::MixedAudio {
    /// Measures gated LUFS, mono compatibility and reconstructed peak of the mix.
    pub fn loudness_report(&self) -> motion_core::Result<LoudnessReport> {
        if self.sample_rate() < 8000 {
            return Err(invalid("loudness_rate", "Require at least 8000 Hz").into());
        }
        if let Some(report) = self.levels.get() {
            return Ok(*report);
        }
        let samples = self.snapshot()?;
        Ok(*self
            .levels
            .get_or_init(|| report(&samples, self.sample_rate())))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{MixOptions, MixedAudio, PcmBuffer};
    use motion_core::{AudioClip, Loudness, Size, TimeLiteral, Timeline};

    #[test]
    fn report_content_cache_checks_samples_rate_and_hash_collisions() {
        let samples = [0.3125, -0.1875, 0.15625, 0.0625];
        let rate = 48000;
        let key = content_key(&samples, rate);
        let report = report(&samples, rate);
        assert_eq!(cached_report(&samples, rate, key), Some(report));
        assert_eq!(
            cached_report(&[0.3125, -0.1875, 0.15625, 0.03125], rate, key),
            None
        );
        assert_eq!(cached_report(&samples, 44100, key), None);
    }

    #[test]
    fn relative_gate_excludes_quiet_passages_and_reports_antiphase() {
        let loud = (0..480000)
            .flat_map(|frame| {
                let value = (libm::sin(std::f64::consts::TAU * 1000.0 * frame as f64 / 48000.0)
                    * 0.1) as f32;
                [value, value]
            })
            .collect::<Vec<_>>();
        let reference = report(&loud, 48000).integrated_lufs.unwrap();
        let gain = amplitude(-23.0 - reference);
        let samples = loud
            .iter()
            .map(|value| (f64::from(*value) * gain) as f32)
            .chain(
                loud.iter()
                    .map(|value| (f64::from(*value) * gain * 0.01) as f32),
            )
            .collect::<Vec<_>>();
        let measured = report(&samples, 48000).integrated_lufs.unwrap();
        assert!(
            (measured + 23.0).abs() < 0.1,
            "relative-gate LUFS={measured}"
        );
        assert_eq!(
            report(&[0.5, -0.5, 0.25, -0.25], 48000).correlation,
            Some(-1.0)
        );
    }

    #[test]
    fn k_weighted_sine_normalization_and_seek_are_stable() -> motion_core::Result {
        let signal = (0..96000)
            .flat_map(|index| {
                let value = (libm::sin(std::f64::consts::TAU * 1000.0 * index as f64 / 48000.0)
                    * 0.1) as f32;
                [value, value]
            })
            .collect();
        let timeline = Timeline::new(Size::new(1.0, 1.0), motion_core::fps!(30))
            .audio(AudioClip::source(PcmBuffer::new(48000, 2, signal)?))
            .build()?;
        let mix = MixedAudio::new(&timeline, MixOptions::new(48000))?;
        assert!((mix.loudness_report()?.integrated_lufs.unwrap() + 20.0).abs() < 0.1);
        assert!(mix.levels.get().is_some());
        assert!(std::sync::Arc::ptr_eq(&mix.levels, &mix.clone().levels));
        let timeline = timeline
            .to_builder()
            .master(Master {
                normalize: Some(Loudness {
                    target_lufs: -23.0,
                    true_peak_db: -1.0,
                }),
                ..Default::default()
            })
            .build()?;
        let mix = MixedAudio::new(&timeline, MixOptions::new(48000))?;
        assert!((mix.loudness_report()?.integrated_lufs.unwrap() + 23.0).abs() < 0.1);
        let window = MixedAudio::new(
            &timeline,
            MixOptions::new(48000).range(1.seconds()..1500.milliseconds()),
        )?;
        let mut expected = [0.0; 64];
        let mut actual = [0.0; 64];
        mix.read(48123, &mut expected);
        window.read(123, &mut actual);
        assert_eq!(actual, expected);
        Ok(())
    }

    #[test]
    fn limiter_controls_reconstructed_peaks_and_silence() -> motion_core::Result {
        let mut samples = vec![2.0; 2000];
        process(&mut samples, 48000, Master::safe())?;
        assert!(reconstructed_peak(&samples) <= amplitude(-1.0) + 1e-6);
        assert!(report(&vec![0.0; 48000], 48000).integrated_lufs.is_none());
        Ok(())
    }

    #[test]
    fn lookahead_gain_has_bounded_attack_and_release() -> motion_core::Result {
        let mut samples = vec![0.2; 10000];
        samples[4000] = 4.0;
        samples[4001] = 4.0;
        let original = samples.clone();
        process(&mut samples, 48000, Master::safe())?;
        let gains: Vec<_> = samples
            .as_chunks::<2>()
            .0
            .iter()
            .zip(original.as_chunks::<2>().0)
            .map(|(output, input)| f64::from(output[0]) / f64::from(input[0]))
            .collect();
        assert!(
            gains
                .windows(2)
                .all(|pair| pair[0] - pair[1] <= 1.0 / 240.0 + 1e-6
                    && pair[1] - pair[0] <= 1.0 / 4800.0 + 1e-6)
        );
        assert!(reconstructed_peak(&samples) <= amplitude(-1.0) + 1e-6);
        Ok(())
    }

    #[test]
    fn reconstructed_peak_interior_path_matches_edge_checked_reference() {
        // The original edge-checked evaluation, applied to every frame.
        fn reference(samples: &[f32]) -> f64 {
            let mut peak = samples
                .iter()
                .map(|value| f64::from(value.abs()))
                .fold(0.0, f64::max);
            let kernels = [0.25, 0.5, 0.75].map(|phase| {
                let mut weights = [0.0; 32];
                for (index, weight) in weights.iter_mut().enumerate() {
                    let offset = index as f64 - 15.0 - phase;
                    let angle = std::f64::consts::PI * offset;
                    let window = 0.42
                        + 0.5 * libm::cos(std::f64::consts::PI * offset / 16.0)
                        + 0.08 * libm::cos(std::f64::consts::TAU * offset / 16.0);
                    *weight = libm::sin(angle) / angle * window;
                }
                let sum: f64 = weights.iter().sum();
                weights.map(|weight| weight / sum)
            });
            for frame in 0..samples.len() / 2 {
                for kernel in &kernels {
                    for channel in 0..2 {
                        let mut value = 0.0;
                        for (index, weight) in kernel.iter().enumerate() {
                            let position = frame as i64 + index as i64 - 15;
                            if position >= 0 {
                                value += f64::from(
                                    samples
                                        .get(position as usize * 2 + channel)
                                        .copied()
                                        .unwrap_or(0.0),
                                ) * weight;
                            }
                        }
                        peak = peak.max(value.abs());
                    }
                }
            }
            peak
        }
        let mut state = 0x2545f4914f6cdd1d_u64;
        let signal: Vec<f32> = (0..4097)
            .map(|_| {
                state ^= state << 13;
                state ^= state >> 7;
                state ^= state << 17;
                (state >> 40) as f32 / (1 << 23) as f32 * 1.6 - 0.8
            })
            .collect();
        for length in [0, 1, 2, 30, 32, 33, 64, 65, 4096, 4097] {
            let samples = &signal[..length];
            assert_eq!(
                reconstructed_peak(samples).to_bits(),
                reference(samples).to_bits(),
                "{length}"
            );
        }
    }
}
