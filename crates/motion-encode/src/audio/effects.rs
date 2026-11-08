//! Bounded offline DSP, evaluated once before clip gain and panning.

use super::pcm::invalid;
use motion_core::{AudioEffect, Duration, Time, ValidationError};

/// RBJ equalizer response.
#[derive(Clone, Copy, Debug)]
pub enum EqKind {
    /// Second-order high pass.
    HighPass,
    /// Second-order low pass.
    LowPass,
    /// Parametric bell.
    Peak,
    /// Low-frequency shelf.
    LowShelf,
    /// High-frequency shelf.
    HighShelf,
    /// Band-rejection filter.
    Notch,
}

/// A bounded digital audio effect. State exists only during offline preparation.
#[derive(Clone, Debug)]
pub enum Effect {
    /// RBJ biquad; Q must be in 0.1..=20 and gain in -24..=24 dB.
    Eq {
        /// Filter response.
        kind: EqKind,
        /// Center or cutoff frequency below Nyquist, in Hz.
        frequency: f64,
        /// Resonance in 0.1..=20.
        q: f64,
        /// Shelf or bell gain in -24..=24 dB.
        gain_db: f64,
    },
    /// Linked-channel feed-forward compressor.
    Compressor {
        /// Threshold in -96..=0 dBFS.
        threshold_db: f64,
        /// Compression ratio in 1..=100.
        ratio: f64,
        /// Envelope rise time in (0,2] seconds.
        attack: Duration,
        /// Envelope decay time in (0,2] seconds.
        release: Duration,
    },
    /// Linked-channel noise gate with smoothed gain.
    Gate {
        /// Opening threshold in -96..=0 dBFS.
        threshold_db: f64,
        /// Opening time in (0,2] seconds.
        attack: Duration,
        /// Closing time in (0,2] seconds.
        release: Duration,
    },
    /// Compresses the high-frequency band, preserving the low band.
    DeEsser {
        /// High-band crossover below Nyquist, in Hz.
        frequency: f64,
        /// High-band threshold in -96..=0 dBFS.
        threshold_db: f64,
        /// High-band compression ratio in 1..=100.
        ratio: f64,
    },
    /// Feedback delay; the declared tail truncates the decay deterministically.
    Delay {
        /// Echo interval in (0,5] seconds.
        delay: Duration,
        /// Signed feedback with absolute value below 0.99.
        feedback: f64,
        /// Wet contribution in 0..=1.
        wet: f64,
        /// Explicit truncation of the decay, at most 30 seconds.
        tail: Duration,
    },
    /// Parallel damped combs and serial all-pass diffusion.
    Reverb {
        /// Room feedback control in 0..=1.
        room: f64,
        /// High-frequency damping in 0..=1.
        damping: f64,
        /// Wet contribution in 0..=1.
        wet: f64,
        /// Explicit truncation of the decay, at most 30 seconds.
        tail: Duration,
    },
}

/// EQ automation sampled on the clip's native clock, with 64-frame coefficient ramps.
pub struct AnimatedEq {
    kind: EqKind,
    frequency: std::sync::Arc<dyn motion_core::Animation<f64>>,
    q: std::sync::Arc<dyn motion_core::Animation<f64>>,
    gain: std::sync::Arc<dyn motion_core::Animation<f64>>,
}

impl std::fmt::Debug for AnimatedEq {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("AnimatedEq")
            .field("kind", &self.kind)
            .finish_non_exhaustive()
    }
}

impl AnimatedEq {
    /// Creates a frequency-automated EQ with static initial resonance and gain.
    pub fn new(
        kind: EqKind,
        frequency: impl motion_core::Animation<f64> + 'static,
        q: f64,
        gain_db: f64,
    ) -> Self {
        Self {
            kind,
            frequency: std::sync::Arc::new(frequency),
            q: std::sync::Arc::new(motion_core::Constant::new(q)),
            gain: std::sync::Arc::new(motion_core::Constant::new(gain_db)),
        }
    }

    /// Replaces resonance with a clip-local animation.
    pub fn resonance(mut self, q: impl motion_core::Animation<f64> + 'static) -> Self {
        self.q = std::sync::Arc::new(q);
        self
    }

    /// Replaces bell/shelf gain with a clip-local animation, in dB.
    pub fn gain(mut self, gain: impl motion_core::Animation<f64> + 'static) -> Self {
        self.gain = std::sync::Arc::new(gain);
        self
    }

    fn at(&self, frame: usize, rate: u32) -> Result<[f64; 5], ValidationError> {
        let time = Time::from_ticks(
            (frame as u128 * motion_core::TICKS_PER_SECOND as u128 / u128::from(rate)) as i64,
        );
        let frequency = self.frequency.sample(time);
        let q = self.q.sample(time);
        let gain = self.gain.sample(time);
        Effect::Eq {
            kind: self.kind,
            frequency,
            q,
            gain_db: gain,
        }
        .validate(rate)?;
        Ok(coefficients(self.kind, frequency, q, gain, rate))
    }
}

impl AudioEffect for AnimatedEq {
    fn validate(&self, rate: u32) -> Result<(), ValidationError> {
        if !(1..=192000).contains(&rate) {
            return Err(invalid("eq_rate", "Invalid automation rate"));
        }
        self.at(0, rate).map(|_| ())
    }

    fn process(&self, input: &mut [f32], channels: u16, rate: u32) -> motion_core::Result {
        self.validate(rate)?;
        let channels = usize::from(channels);
        if !(1..=2).contains(&channels)
            || !input.len().is_multiple_of(channels)
            || input.len() > super::pcm::MAX_SAMPLES
            || input.iter().any(|sample| !sample.is_finite())
        {
            return Err(invalid("eq_input", "Invalid automated EQ PCM").into());
        }
        let mut previous = self.at(0, rate)?;
        let mut filters = vec![Biquad::new(previous); channels];
        for (block, chunk) in input.chunks_mut(64 * channels).enumerate() {
            let next = self.at(block * 64 + chunk.len() / channels, rate)?;
            let count = chunk.len() / channels;
            for (index, frame) in chunk.chunks_exact_mut(channels).enumerate() {
                let amount = (index + 1) as f64 / count as f64;
                for (sample, filter) in frame.iter_mut().zip(&mut filters) {
                    filter.coefficients = std::array::from_fn(|index| {
                        previous[index] + (next[index] - previous[index]) * amount
                    });
                    *sample = filter.sample(f64::from(*sample)) as f32;
                }
            }
            previous = next;
        }
        if input.iter().any(|sample| !sample.is_finite()) {
            return Err(invalid("eq_output", "Automated EQ produced nonfinite PCM").into());
        }
        Ok(())
    }
}

pub(crate) fn amplitude(db: f64) -> f64 {
    libm::pow(10.0, db / 20.0)
}

#[derive(Clone, Copy)]
pub(crate) struct Biquad {
    coefficients: [f64; 5],
    state: [f64; 2],
}

impl Biquad {
    pub(crate) fn new(coefficients: [f64; 5]) -> Self {
        Self {
            coefficients,
            state: [0.0; 2],
        }
    }

    pub(crate) fn sample(&mut self, input: f64) -> f64 {
        let [first, second, third, fourth, fifth] = self.coefficients;
        let output = first * input + self.state[0];
        self.state[0] = second * input - fourth * output + self.state[1];
        self.state[1] = third * input - fifth * output;
        output
    }
}

pub(crate) fn coefficients(kind: EqKind, frequency: f64, q: f64, gain: f64, rate: u32) -> [f64; 5] {
    let omega = std::f64::consts::TAU * frequency / f64::from(rate);
    let cosine = libm::cos(omega);
    let alpha = libm::sin(omega) / (2.0 * q);
    let level = amplitude(gain * 0.5);
    let shelf = 2.0 * libm::sqrt(level) * alpha;
    let (numerator, denominator) = match kind {
        EqKind::LowPass => (
            [(1.0 - cosine) * 0.5, 1.0 - cosine, (1.0 - cosine) * 0.5],
            [1.0 + alpha, -2.0 * cosine, 1.0 - alpha],
        ),
        EqKind::HighPass => (
            [(1.0 + cosine) * 0.5, -(1.0 + cosine), (1.0 + cosine) * 0.5],
            [1.0 + alpha, -2.0 * cosine, 1.0 - alpha],
        ),
        EqKind::Notch => (
            [1.0, -2.0 * cosine, 1.0],
            [1.0 + alpha, -2.0 * cosine, 1.0 - alpha],
        ),
        EqKind::Peak => (
            [1.0 + alpha * level, -2.0 * cosine, 1.0 - alpha * level],
            [1.0 + alpha / level, -2.0 * cosine, 1.0 - alpha / level],
        ),
        EqKind::LowShelf => (
            [
                level * ((level + 1.0) - (level - 1.0) * cosine + shelf),
                2.0 * level * ((level - 1.0) - (level + 1.0) * cosine),
                level * ((level + 1.0) - (level - 1.0) * cosine - shelf),
            ],
            [
                (level + 1.0) + (level - 1.0) * cosine + shelf,
                -2.0 * ((level - 1.0) + (level + 1.0) * cosine),
                (level + 1.0) + (level - 1.0) * cosine - shelf,
            ],
        ),
        EqKind::HighShelf => (
            [
                level * ((level + 1.0) + (level - 1.0) * cosine + shelf),
                -2.0 * level * ((level - 1.0) + (level + 1.0) * cosine),
                level * ((level + 1.0) + (level - 1.0) * cosine - shelf),
            ],
            [
                (level + 1.0) - (level - 1.0) * cosine + shelf,
                2.0 * ((level - 1.0) - (level + 1.0) * cosine),
                (level + 1.0) - (level - 1.0) * cosine - shelf,
            ],
        ),
    };
    [
        numerator[0] / denominator[0],
        numerator[1] / denominator[0],
        numerator[2] / denominator[0],
        denominator[1] / denominator[0],
        denominator[2] / denominator[0],
    ]
}

fn valid_time(time: Duration) -> bool {
    time > Time::ZERO && time.as_seconds() <= 2.0
}

fn threshold(value: f64) -> bool {
    value.is_finite() && (-96.0..=0.0).contains(&value)
}

fn unit(value: f64) -> bool {
    value.is_finite() && (0.0..=1.0).contains(&value)
}

impl AudioEffect for Effect {
    fn validate(&self, rate: u32) -> Result<(), ValidationError> {
        let frequency_ok = |frequency: f64| {
            frequency.is_finite() && frequency > 0.0 && frequency < f64::from(rate) * 0.5
        };
        let valid = match *self {
            Self::Eq {
                frequency,
                q,
                gain_db,
                ..
            } => {
                frequency_ok(frequency)
                    && q.is_finite()
                    && (0.1..=20.0).contains(&q)
                    && gain_db.is_finite()
                    && (-24.0..=24.0).contains(&gain_db)
            }
            Self::Compressor {
                threshold_db,
                ratio,
                attack,
                release,
            } => {
                threshold(threshold_db)
                    && ratio.is_finite()
                    && (1.0..=100.0).contains(&ratio)
                    && valid_time(attack)
                    && valid_time(release)
            }
            Self::Gate {
                threshold_db,
                attack,
                release,
            } => threshold(threshold_db) && valid_time(attack) && valid_time(release),
            Self::DeEsser {
                frequency,
                threshold_db,
                ratio,
            } => {
                frequency_ok(frequency)
                    && threshold(threshold_db)
                    && ratio.is_finite()
                    && (1.0..=100.0).contains(&ratio)
            }
            Self::Delay {
                delay,
                feedback,
                wet,
                tail,
            } => {
                delay > Time::ZERO
                    && delay.as_seconds() <= 5.0
                    && feedback.is_finite()
                    && feedback.abs() < 0.99
                    && unit(wet)
                    && tail >= Time::ZERO
                    && tail.as_seconds() <= 30.0
            }
            Self::Reverb {
                room,
                damping,
                wet,
                tail,
            } => {
                unit(room)
                    && unit(damping)
                    && unit(wet)
                    && tail >= Time::ZERO
                    && tail.as_seconds() <= 30.0
            }
        };
        if !(1..=192_000).contains(&rate) || !valid {
            return Err(invalid(
                "effect",
                "Invalid DSP frequency, gain, time or bounded feedback",
            ));
        }
        Ok(())
    }

    fn tail(&self) -> Duration {
        match *self {
            Self::Delay { tail, .. } | Self::Reverb { tail, .. } => tail,
            _ => Time::ZERO,
        }
    }

    fn process(&self, input: &mut [f32], channels: u16, rate: u32) -> motion_core::Result {
        self.validate(rate)?;
        if !(1..=2).contains(&channels)
            || !input.len().is_multiple_of(usize::from(channels))
            || input.len() > super::pcm::MAX_SAMPLES
            || input.iter().any(|value| !value.is_finite())
        {
            return Err(invalid(
                "effect_pcm",
                "Require bounded complete finite mono/stereo PCM",
            )
            .into());
        }
        let channel_count = usize::from(channels);
        match *self {
            Self::Eq {
                kind,
                frequency,
                q,
                gain_db,
            } => {
                let mut filters = vec![
                    Biquad::new(coefficients(kind, frequency, q, gain_db, rate));
                    channel_count
                ];
                for frame in input.chunks_exact_mut(channel_count) {
                    for (sample, filter) in frame.iter_mut().zip(&mut filters) {
                        *sample = filter.sample(f64::from(*sample)) as f32;
                    }
                }
            }
            Self::Compressor {
                threshold_db,
                attack,
                release,
                ..
            }
            | Self::Gate {
                threshold_db,
                attack,
                release,
            } => {
                let ratio = if let Self::Compressor { ratio, .. } = *self {
                    ratio
                } else {
                    1.0
                };
                Dynamics {
                    threshold_db,
                    ratio,
                    attack: attack.as_seconds(),
                    release: release.as_seconds(),
                    gate: matches!(self, Self::Gate { .. }),
                }
                .apply(input, channel_count, rate);
            }
            Self::DeEsser {
                frequency,
                threshold_db,
                ratio,
            } => {
                let mut high = input.to_vec();
                Self::Eq {
                    kind: EqKind::HighPass,
                    frequency,
                    q: std::f64::consts::FRAC_1_SQRT_2,
                    gain_db: 0.0,
                }
                .process(&mut high, channels, rate)?;
                let original = high.clone();
                Dynamics {
                    threshold_db,
                    ratio,
                    attack: 0.001,
                    release: 0.05,
                    gate: false,
                }
                .apply(&mut high, channel_count, rate);
                for ((sample, processed), band) in input.iter_mut().zip(high).zip(original) {
                    *sample += processed - band;
                }
            }
            Self::Delay {
                delay,
                feedback,
                wet,
                ..
            } => {
                let length =
                    motion_core::audio::frames(delay, rate)?.max(1) as usize * channel_count;
                let mut buffer = vec![0.0_f64; length];
                for (index, sample) in input.iter_mut().enumerate() {
                    let echo = buffer[index % length];
                    buffer[index % length] = f64::from(*sample) + echo * feedback;
                    *sample = (f64::from(*sample) * (1.0 - wet) + echo * wet) as f32;
                }
            }
            Self::Reverb {
                room, damping, wet, ..
            } => {
                let mut combs = [0.0297, 0.0371, 0.0411, 0.0437].map(|seconds| {
                    vec![
                        0.0_f64;
                        (seconds * f64::from(rate)).round().max(1.0) as usize * channel_count
                    ]
                });
                let mut low = [[0.0_f64; 2]; 4];
                let mut allpasses = [0.005, 0.0017].map(|seconds| {
                    vec![
                        0.0_f64;
                        (seconds * f64::from(rate)).round().max(1.0) as usize * channel_count
                    ]
                });
                for (index, sample) in input.iter_mut().enumerate() {
                    let channel = index % channel_count;
                    let dry = f64::from(*sample);
                    let mut reverberation = 0.0;
                    for (comb, filtered) in combs.iter_mut().zip(&mut low) {
                        let cursor = index % comb.len();
                        let delayed = comb[cursor];
                        filtered[channel] =
                            delayed * (1.0 - damping * 0.95) + filtered[channel] * damping * 0.95;
                        comb[cursor] = dry + filtered[channel] * (0.4 + room * 0.55);
                        reverberation += delayed * 0.25;
                    }
                    for allpass in &mut allpasses {
                        let cursor = index % allpass.len();
                        let delayed = allpass[cursor];
                        let next = delayed - reverberation * 0.5;
                        allpass[cursor] = reverberation + next * 0.5;
                        reverberation = next;
                    }
                    *sample = (dry * (1.0 - wet) + reverberation * wet) as f32;
                }
            }
        }
        if input.iter().any(|value| !value.is_finite()) {
            return Err(invalid("effect_output", "DSP produced nonfinite PCM").into());
        }
        Ok(())
    }
}

/// Envelope-following compressor or gate settings; times are in seconds.
struct Dynamics {
    threshold_db: f64,
    ratio: f64,
    attack: f64,
    release: f64,
    gate: bool,
}

impl Dynamics {
    fn apply(&self, input: &mut [f32], channels: usize, rate: u32) {
        let Self {
            threshold_db,
            ratio,
            attack,
            release,
            gate,
        } = *self;
        let attack = libm::exp(-1.0 / (attack * f64::from(rate)));
        let release = libm::exp(-1.0 / (release * f64::from(rate)));
        let ceiling = amplitude(threshold_db);
        let mut envelope = 0.0;
        let mut gain = if gate { 0.0 } else { 1.0 };
        for frame in input.chunks_exact_mut(channels) {
            let peak = frame
                .iter()
                .map(|sample| f64::from(sample.abs()))
                .fold(0.0, f64::max);
            let coefficient = if peak > envelope { attack } else { release };
            envelope = coefficient * envelope + (1.0 - coefficient) * peak;
            let target = if gate {
                if envelope >= ceiling { 1.0 } else { 0.0 }
            } else if envelope > ceiling {
                libm::pow(ceiling / envelope, 1.0 - 1.0 / ratio)
            } else {
                1.0
            };
            let coefficient = if (gate && target > gain) || (!gate && target < gain) {
                attack
            } else {
                release
            };
            gain = coefficient * gain + (1.0 - coefficient) * target;
            for sample in frame {
                *sample = (f64::from(*sample) * gain) as f32;
            }
        }
    }
}

/// Conventional speech cleanup, with optional de-essing left to the caller.
pub fn speech_clarity(clip: motion_core::AudioClip) -> motion_core::AudioClip {
    clip.effect(Effect::Eq {
        kind: EqKind::HighPass,
        frequency: 80.0,
        q: 0.707,
        gain_db: 0.0,
    })
    .effect(Effect::Eq {
        kind: EqKind::Peak,
        frequency: 3500.0,
        q: 0.7,
        gain_db: 2.0,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{MixedAudio, PcmBuffer};
    use motion_core::{AudioClip, AudioSource, TimeLiteral};

    #[test]
    fn eq_response_and_effect_seek_replay() -> motion_core::Result {
        let signal: Vec<_> = (0..8000)
            .map(|index| {
                (libm::sin(std::f64::consts::TAU * 1000.0 * index as f64 / 8000.0) * 0.5) as f32
            })
            .collect();
        let mut filtered = signal.clone();
        Effect::Eq {
            kind: EqKind::Notch,
            frequency: 1000.0,
            q: 1.0,
            gain_db: 0.0,
        }
        .process(&mut filtered, 1, 8000)?;
        assert!(filtered[1000..].iter().all(|sample| sample.abs() < 1e-5));
        let clip = AudioClip::source(PcmBuffer::new(8000, 1, signal)?).effect(Effect::Eq {
            kind: EqKind::LowPass,
            frequency: 500.0,
            q: 0.707,
            gain_db: 0.0,
        });
        let mix = MixedAudio::new(&[clip], 8000, 1.seconds())?;
        let mut full = vec![0.0; 16000];
        mix.read(0, &mut full);
        let mut window = [0.0; 64];
        mix.read(1234, &mut window);
        assert_eq!(window, full[2468..2532]);
        assert!(window.iter().any(|sample| sample.abs() > 0.001));
        assert!(window.iter().all(|sample| sample.abs() < 0.1));
        Ok(())
    }

    #[test]
    fn delay_tail_and_invalid_configuration() -> motion_core::Result {
        let delay = Effect::Delay {
            delay: 10.milliseconds(),
            feedback: 0.5,
            wet: 1.0,
            tail: 50.milliseconds(),
        };
        let clip = AudioClip::source(PcmBuffer::new(1000, 1, vec![1.0])?)
            .pan(-1.0)
            .effect(delay);
        assert_eq!(clip.resolved_duration()?, 51.milliseconds());
        let mix = MixedAudio::new(&[clip], 1000, 51.milliseconds())?;
        let mut output = vec![0.0; 102];
        mix.read(0, &mut output);
        assert_eq!(output[20], 1.0);
        assert_eq!(output[40], 0.5);
        assert_eq!(output[60], 0.25);
        assert!(
            Effect::Eq {
                kind: EqKind::Peak,
                frequency: f64::NAN,
                q: 1.0,
                gain_db: 0.0
            }
            .validate(48000)
            .is_err()
        );
        Ok(())
    }

    #[test]
    fn automated_eq_is_seek_exact_and_validates_dynamic_controls() -> motion_core::Result {
        use motion_core::{AudioClip, AudioSource, Size, TimeLiteral, Timeline, Tween};
        let pcm = crate::PcmBuffer::new(
            8000,
            1,
            (0..8000)
                .map(|index| {
                    libm::sin(std::f64::consts::TAU * 440.0 * index as f64 / 8000.0) as f32
                })
                .collect(),
        )?;
        let effect = AnimatedEq::new(
            EqKind::LowPass,
            Tween::new(100.0, 2000.0, 1.seconds()),
            0.707,
            0.0,
        );
        let timeline = Timeline::new(Size::new(1.0, 1.0), 30)
            .audio(AudioClip::source(pcm.clone()).effect(effect));
        let whole = crate::MixedAudio::from_timeline(&timeline, 8000)?;
        let window = crate::MixedAudio::from_window(
            &timeline,
            400.milliseconds()..600.milliseconds(),
            8000,
        )?;
        let mut first = [0.0; 80];
        let mut second = [0.0; 80];
        whole.read(3257, &mut first);
        window.read(57, &mut second);
        assert_eq!(first, second);
        assert!(first.iter().all(|sample| sample.is_finite()));
        let invalid = AnimatedEq::new(
            EqKind::LowPass,
            Tween::new(100.0, 5000.0, 1.seconds()),
            0.707,
            0.0,
        );
        let mut samples = pcm.samples().to_vec();
        assert!(invalid.process(&mut samples, 1, 8000).is_err());
        Ok(())
    }
}
