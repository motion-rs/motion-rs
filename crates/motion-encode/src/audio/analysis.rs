//! Cached forward-window RMS and Hann FFT band-energy animations.
//!
//! ```
//! use motion_encode::prelude::*;
//! use motion_core::prelude::*;
//! use motion_core::{
//!     DisplayList, RasterLimits, ResourceStore, ValidatedDisplayList, MAX_PATH_VERBS,
//!     MAX_STROKE_VERBS, MAX_SVG_PATH_BYTES, sample_offsets,
//! };
//! use motion_render::{Composition, RenderPrepare, RenderOptions, Renderer, render_still};
//! use std::sync::Arc;
//! let pcm = Arc::new(PcmBuffer::new(48_000, 1, vec![0.5; 4800])?);
//! let analysis = AudioAnalysis::from_pcm(pcm);
//! let loudness = analysis.loudness()?;
//! assert!((loudness.sample(10.milliseconds().after_origin()) - 0.5).abs() < 1e-9);
//! # Ok::<(), motion_core::Error>(())
//! ```

use super::pcm::{MAX_SAMPLES, ceil_frames, invalid};
use crate::PcmBuffer;
use motion_core::{AudioSource, Duration, FiniteAnimation, Signal, Time};
use rustfft::{FftPlanner, num_complex::Complex};
use std::{
    collections::BTreeMap,
    sync::{Arc, Mutex},
};

mod advanced;

pub use advanced::{AudioMarkers, AudioSpectrum, BeatGrid};

pub(crate) type Cache = Arc<Mutex<BTreeMap<Key, Vec<Arc<AnalysisTrace>>>>>;

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub(crate) enum Key {
    Rms,
    Band(u64, u64),
    Spectrum(usize, u64, u64, usize, usize),
    Chroma,
    Pitch,
    Flux,
}

#[derive(Debug)]
pub(crate) struct AnalysisTrace {
    values: Vec<f64>,
    sample_rate: u32,
    hop_frames: usize,
    duration: Duration,
}

/// A finite, stateless f64 animation, sampled without source I/O or FFT work.
#[derive(Clone, Debug)]
pub struct AudioSignal(Arc<AnalysisTrace>);

impl Signal<f64> for AudioSignal {
    fn sample(&self, time: Time) -> f64 {
        let Some(time) = time.duration_since(Time::ZERO) else {
            return 0.0;
        };
        if time >= self.0.duration {
            return 0.0;
        }
        let frame = (time.ticks() as u128 * u128::from(self.0.sample_rate)
            / motion_core::TICKS_PER_SECOND as u128) as usize;
        let index = frame / self.0.hop_frames;
        let amount = (frame % self.0.hop_frames) as f64 / self.0.hop_frames as f64;
        let first = self.0.values.get(index).copied().unwrap_or(0.0);
        let second = self.0.values.get(index + 1).copied().unwrap_or(0.0);
        first * (1.0 - amount) + second * amount
    }
}

impl FiniteAnimation<f64> for AudioSignal {
    fn duration(&self) -> Duration {
        self.0.duration
    }
}

/// Shared analysis environment for one immutable PCM snapshot. Clones and new
/// views of the same PcmBuffer share at most 16 cached traces / 64 MiB trace payload.
#[derive(Clone, Debug)]
pub struct AudioAnalysis {
    pcm: Arc<PcmBuffer>,
}

impl AudioAnalysis {
    /// Resolves a lazy audio resource once, preserving its decode/error identity.
    pub fn new(handle: motion_core::Handle<PcmBuffer>) -> motion_core::Result<Self> {
        Ok(Self::from_pcm(handle.get()?))
    }

    /// Shares existing decoded data and all cached analysis traces.
    pub fn from_pcm(pcm: Arc<PcmBuffer>) -> Self {
        Self { pcm }
    }

    /// Freezes a bounded interval of any pure procedural or mixed source.
    pub fn from_source(source: &dyn AudioSource, duration: Duration) -> motion_core::Result<Self> {
        let channels = usize::from(source.channels());
        let count = ceil_frames(duration, source.sample_rate());
        if !(1..=192_000).contains(&source.sample_rate())
            || !(1..=2).contains(&channels)
            || count == 0
            || count > (MAX_SAMPLES / channels) as u64
        {
            return Err(invalid(
                "analysis_source",
                "Analysis snapshot exceeds PCM format or size allowance",
            )
            .into());
        }
        let mut values = vec![0.0; count as usize * channels];
        let mut start = 0;
        while start < count as usize {
            let end = (start + 4096).min(count as usize);
            let target = &mut values[start * channels..end * channels];
            let got = source.try_read(start as u64, target)?;
            if got > end - start
                || target[..got * channels]
                    .iter()
                    .any(|value| !value.is_finite())
            {
                return Err(invalid("analysis_source", "Invalid source count or PCM").into());
            }
            target[got * channels..].fill(0.0);
            if got == 0 {
                break;
            }
            start += got;
        }
        Ok(Self::from_pcm(Arc::new(PcmBuffer::new(
            source.sample_rate(),
            channels as u16,
            values,
        )?)))
    }

    /// Linear RMS amplitude, not perceptual LUFS/dB. Channels contribute energy,
    /// so opposite-phase stereo does not cancel. Windows look forward offline.
    pub fn loudness(&self) -> motion_core::Result<AudioSignal> {
        self.trace(Key::Rms)
    }

    /// RMS energy in [low_hz,high_hz), selecting FFT-bin centers with Hann power
    /// compensation and one-sided energy. Neither dB normalization nor a filter.
    pub fn band(&self, low_hz: f64, high_hz: f64) -> motion_core::Result<AudioSignal> {
        if !low_hz.is_finite()
            || !high_hz.is_finite()
            || low_hz < 0.0
            || low_hz >= high_hz
            || high_hz > f64::from(self.pcm.sample_rate()) * 0.5
        {
            return Err(invalid("band", "Require finite 0<=low<high<=Nyquist").into());
        }
        self.trace(Key::Band(low_hz.to_bits(), high_hz.to_bits()))
    }

    fn trace(&self, key: Key) -> motion_core::Result<AudioSignal> {
        if let Some(data) = self
            .pcm
            .analysis
            .lock()
            .map_err(|_| invalid("cache", "Analysis cache is poisoned"))?
            .get(&key)
            .cloned()
        {
            return Ok(AudioSignal(data[0].clone()));
        }
        let rate = self.pcm.sample_rate();
        let channels = usize::from(self.pcm.channels());
        let window = ((rate / 25) as usize).next_power_of_two().clamp(64, 8192);
        let hop = window / 4;
        let count = (self.pcm.frames() as usize).div_ceil(hop);
        if count > 1_000_000 {
            return Err(invalid(
                "analysis_points",
                "Analysis exceeds one million time points",
            )
            .into());
        }
        let mut values = Vec::with_capacity(count);
        let weights = (0..window)
            .map(|index| {
                0.5 - 0.5 * libm::cos(std::f64::consts::TAU * index as f64 / window as f64)
            })
            .collect::<Vec<_>>();
        let energy: f64 = weights.iter().map(|value| value * value).sum();
        let fft = FftPlanner::<f64>::new().plan_fft_forward(window);
        let mut buffer = vec![Complex::new(0.0, 0.0); window];
        for start in (0..self.pcm.frames() as usize).step_by(hop) {
            let mut sum = 0.0;
            for channel in 0..channels {
                match key {
                    Key::Rms => {
                        for index in 0..window {
                            let value = f64::from(
                                self.pcm
                                    .samples()
                                    .get((start + index) * channels + channel)
                                    .copied()
                                    .unwrap_or(0.0),
                            );
                            sum += value * value / window as f64;
                        }
                    }
                    Key::Band(low, high) => {
                        for index in 0..window {
                            buffer[index] = Complex::new(
                                f64::from(
                                    self.pcm
                                        .samples()
                                        .get((start + index) * channels + channel)
                                        .copied()
                                        .unwrap_or(0.0),
                                ) * weights[index],
                                0.0,
                            );
                        }
                        fft.process(&mut buffer);
                        for (index, value) in buffer[..=window / 2].iter().enumerate() {
                            let frequency = index as f64 * f64::from(rate) / window as f64;
                            if frequency >= f64::from_bits(low) && frequency < f64::from_bits(high)
                            {
                                sum += value.norm_sqr()
                                    * if index == 0 || index == window / 2 {
                                        1.0
                                    } else {
                                        2.0
                                    }
                                    / (window as f64 * energy);
                            }
                        }
                    }
                    _ => {
                        return Err(
                            invalid("analysis_key", "Expected a scalar RMS or band key").into()
                        );
                    }
                }
            }
            let value = libm::sqrt(sum / channels as f64);
            if !value.is_finite() {
                return Err(invalid("analysis", "Nonfinite analysis result").into());
            }
            values.push(value);
        }
        let data = Arc::new(AnalysisTrace {
            values,
            sample_rate: rate,
            hop_frames: hop,
            duration: self.pcm.duration().expect("finite PCM"),
        });
        let mut cache = self
            .pcm
            .analysis
            .lock()
            .map_err(|_| invalid("cache", "Analysis cache is poisoned"))?;
        if let Some(existing) = cache.get(&key) {
            return Ok(AudioSignal(existing[0].clone()));
        }
        let retained: usize = cache
            .values()
            .flatten()
            .map(|data| data.values.len() * 8)
            .sum();
        if cache.len() >= 16 || retained + data.values.len() * 8 > 64 * 1024 * 1024 {
            cache.clear();
        }
        cache.insert(key, vec![data.clone()]);
        Ok(AudioSignal(data))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{MixOptions, MixSource, MixedAudio};
    use motion_core::{AudioClip, TimeLiteral};

    #[test]
    fn rms_and_fft_band_signals_match_known_tones_cache_and_replay() -> motion_core::Result {
        let samples = (0..8192)
            .flat_map(|index| {
                let value = (libm::sin(std::f64::consts::TAU * 1024.0 * f64::from(index) / 8192.0)
                    * 0.5) as f32;
                [value, -value]
            })
            .collect();
        let pcm = Arc::new(PcmBuffer::new(8192, 2, samples)?);
        let analysis = AudioAnalysis::from_pcm(pcm.clone());
        let loudness = analysis.loudness()?;
        let band = analysis.band(900.0, 1200.0)?;
        let quiet = analysis.band(0.0, 100.0)?;
        for time in [
            500.milliseconds().after_origin(),
            100.milliseconds().after_origin(),
            700.milliseconds().after_origin(),
            500.milliseconds().after_origin(),
        ] {
            assert!((loudness.sample(time) - 0.5 / libm::sqrt(2.0)).abs() < 1e-6);
            assert!((band.sample(time) - 0.5 / libm::sqrt(2.0)).abs() < 1e-6);
            assert!(quiet.sample(time) < 1e-6);
        }
        assert_eq!(loudness.sample(Time::ZERO - 1.milliseconds()), 0.0);
        assert_eq!(band.sample(1.seconds().after_origin()), 0.0);
        assert!(Arc::ptr_eq(
            &band.0,
            &AudioAnalysis::from_pcm(pcm).band(900.0, 1200.0)?.0
        ));
        std::thread::scope(|scope| {
            let tasks = (0..3)
                .map(|_| {
                    scope.spawn(|| {
                        analysis
                            .band(900.0, 1200.0)
                            .unwrap()
                            .sample(500.milliseconds().after_origin())
                    })
                })
                .collect::<Vec<_>>();
            for task in tasks {
                assert_eq!(
                    task.join().unwrap(),
                    band.sample(500.milliseconds().after_origin())
                );
            }
        });
        assert!(analysis.band(-1.0, 10.0).is_err());
        assert!(analysis.band(0.0, 9000.0).is_err());
        let clip = AudioClip::source(PcmBuffer::new(8192, 1, vec![1.0; 8192])?)
            .pan(-1.0)
            .volume_envelope(loudness);
        let mixed = MixedAudio::new(
            MixSource::clips(&[clip], 1.seconds()),
            MixOptions::new(8192),
        )?;
        let mut output = [0.0; 2];
        mixed.read(4096, &mut output);
        assert!(
            (f64::from(output[0]) - band.sample(500.milliseconds().after_origin())).abs() < 1e-6
        );
        Ok(())
    }
}
