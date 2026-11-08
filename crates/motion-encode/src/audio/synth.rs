//! Random-access synthesis and seeded sound effects, without binary assets.

use super::{effects::amplitude, pcm::invalid};
use motion_core::{Animation, AudioSource, Duration, Time};

/// Segments in the oscillator sine table. Linear interpolation keeps the error below `3e-7`
/// (about -130 dB), under 24-bit quantization.
const SINE_SEGMENTS: usize = 4096;

/// Oscillator or octave-shaped procedural noise.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Wave {
    /// Sinusoidal oscillator.
    Sine,
    /// Band-edge-corrected sawtooth.
    Saw,
    /// Band-edge-corrected square.
    Square,
    /// Triangle oscillator.
    Triangle,
    /// Independent seeded white noise.
    White,
    /// Equal-energy octave noise approximation.
    Pink,
    /// Bounded low-frequency octave noise approximation.
    Brown,
}

/// A pure ADSR envelope, with a fixed finite note length.
#[derive(Clone, Copy, Debug)]
pub struct Adsr {
    attack: Duration,
    decay: Duration,
    sustain: f64,
    release: Duration,
    duration: Duration,
}

impl Adsr {
    /// Validates nonnegative stages, sustain in `[0, 1]`, and a positive note length.
    pub fn new(
        attack: Duration,
        decay: Duration,
        sustain: f64,
        release: Duration,
        duration: Duration,
    ) -> motion_core::Result<Self> {
        if attack < Time::ZERO
            || decay < Time::ZERO
            || release < Time::ZERO
            || duration <= Time::ZERO
            || duration.as_seconds() > 86400.0
            || attack.as_seconds() + decay.as_seconds() + release.as_seconds()
                > duration.as_seconds()
            || !sustain.is_finite()
            || !(0.0..=1.0).contains(&sustain)
        {
            return Err(invalid("adsr", "Invalid ADSR stages or sustain").into());
        }
        Ok(Self {
            attack,
            decay,
            sustain,
            release,
            duration,
        })
    }
}

impl Animation<f64> for Adsr {
    fn sample(&self, time: Time) -> f64 {
        if time < Time::ZERO || time >= self.duration {
            return 0.0;
        }
        let seconds = time.as_seconds();
        if time < self.attack {
            return seconds / self.attack.as_seconds();
        }
        if time < self.attack + self.decay {
            return 1.0
                + (self.sustain - 1.0) * (seconds - self.attack.as_seconds())
                    / self.decay.as_seconds();
        }
        if time >= self.duration - self.release && self.release > Time::ZERO {
            return self.sustain * (self.duration - time).as_seconds() / self.release.as_seconds();
        }
        self.sustain
    }

    fn duration(&self) -> Option<Duration> {
        Some(self.duration)
    }
}

#[derive(Clone, Copy, Debug)]
enum Kind {
    Oscillator(Wave),
    Effect(Sound),
    Metronome { bpm: f64, accent: u8 },
    Drums { bpm: f64, steps: [u8; 16] },
}

/// Programmatically generated UI, transition and percussion accents.
#[derive(Clone, Copy, Debug)]
pub enum Sound {
    /// Short filtered-noise transition with a rising tone.
    Whoosh,
    /// Short high-frequency decaying accent.
    Click,
    /// Rounded low-frequency accent.
    Pop,
    /// Dry metronome accent.
    Tick,
    /// Increasing noise and rising chirp.
    Riser,
    /// Bass and noise impact.
    Impact,
    /// Descending bass chirp.
    SubDrop,
    /// Synthetic bass drum.
    Kick,
    /// Synthetic noise drum.
    Snare,
    /// Short bright noise percussion.
    Hat,
    /// Quiet slowly varying seeded noise bed.
    Ambience,
}

/// Finite or unbounded mono source. Phase is integrated analytically from time.
#[derive(Clone, Debug)]
pub struct Synth {
    rate: u32,
    frequency: f64,
    end_frequency: f64,
    gain: f64,
    seed: u32,
    duration: Option<Duration>,
    kind: Kind,
}

impl Synth {
    /// Creates an unbounded oscillator with unit amplitude.
    pub fn new(wave: Wave, frequency: f64, rate: u32, seed: u32) -> motion_core::Result<Self> {
        if !(1..=192000).contains(&rate)
            || !frequency.is_finite()
            || frequency <= 0.0
            || frequency >= f64::from(rate) * 0.5
        {
            return Err(invalid("synth", "Frequency must be positive and below Nyquist").into());
        }
        Ok(Self {
            rate,
            frequency,
            end_frequency: frequency,
            gain: 1.0,
            seed,
            duration: None,
            kind: Kind::Oscillator(wave),
        })
    }

    /// Makes a bounded source, with an optional analytically integrated chirp.
    pub fn chirp(mut self, end_frequency: f64, duration: Duration) -> motion_core::Result<Self> {
        if !end_frequency.is_finite()
            || end_frequency <= 0.0
            || end_frequency >= f64::from(self.rate) * 0.5
            || duration <= Time::ZERO
            || duration.as_seconds() > 86400.0
        {
            return Err(invalid("chirp", "Invalid chirp frequency or duration").into());
        }
        self.end_frequency = end_frequency;
        self.duration = Some(duration);
        Ok(self)
    }

    /// Sets a finite amplitude multiplier in -96..=0 dBFS.
    pub fn gain_db(mut self, gain: f64) -> motion_core::Result<Self> {
        if !gain.is_finite() || !(-96.0..=0.0).contains(&gain) {
            return Err(invalid("synth_gain", "Require finite -96..=0 dB").into());
        }
        self.gain = amplitude(gain);
        Ok(self)
    }

    /// A finite seeded effect; duration is caller-authored rather than implicit.
    pub fn sound(sound: Sound, duration: Duration, seed: u32) -> motion_core::Result<Self> {
        let mut source = Self::new(Wave::Sine, 440.0, 48000, seed)?.chirp(440.0, duration)?;
        source.kind = Kind::Effect(sound);
        source.gain = 0.8;
        Ok(source)
    }

    /// Finite click track, with an accented beat every `accent` beats.
    pub fn metronome(bpm: f64, accent: u8, duration: Duration) -> motion_core::Result<Self> {
        if !bpm.is_finite() || !(20.0..=400.0).contains(&bpm) || !(1..=16).contains(&accent) {
            return Err(invalid("metronome", "Invalid BPM or accent period").into());
        }
        let mut source = Self::sound(Sound::Tick, duration, 0)?;
        source.kind = Kind::Metronome { bpm, accent };
        Ok(source)
    }

    /// Sixteenth-note drum machine. Each step is a bit mask: 1 kick, 2 snare, 4 hat.
    pub fn drum_machine(
        bpm: f64,
        steps: [u8; 16],
        duration: Duration,
        seed: u32,
    ) -> motion_core::Result<Self> {
        if !bpm.is_finite() || !(20.0..=400.0).contains(&bpm) || steps.iter().any(|step| *step > 7)
        {
            return Err(invalid("drums", "Invalid BPM or kick/snare/hat mask").into());
        }
        let mut source = Self::sound(Sound::Kick, duration, seed)?;
        source.kind = Kind::Drums { bpm, steps };
        Ok(source)
    }

    fn sample(&self, frame: u64) -> f32 {
        let seconds = frame as f64 / f64::from(self.rate);
        let duration = self.duration.map_or(1.0, |duration| duration.as_seconds());
        let progress = (seconds / duration).clamp(0.0, 1.0);
        let phase = (self.frequency * seconds
            + 0.5 * (self.end_frequency - self.frequency) * seconds * seconds / duration)
            .fract();
        let tone =
            |frequency: f64, decay: f64| sine(frequency * seconds) * libm::exp(-decay * seconds);
        let noise = |wave: Wave| noise(wave, frame, self.seed);
        let output = match self.kind {
            Kind::Oscillator(wave) => match wave {
                Wave::Sine => sine(phase),
                Wave::Saw => {
                    phase * 2.0
                        - 1.0
                        - poly_blep(
                            phase,
                            (self.frequency + (self.end_frequency - self.frequency) * progress)
                                / f64::from(self.rate),
                        )
                }
                Wave::Square => {
                    let step = (self.frequency + (self.end_frequency - self.frequency) * progress)
                        / f64::from(self.rate);
                    (if phase < 0.5 { 1.0 } else { -1.0 }) + poly_blep(phase, step)
                        - poly_blep((phase + 0.5).fract(), step)
                }
                Wave::Triangle => 1.0 - 4.0 * (phase - 0.5).abs(),
                wave => noise(wave),
            },
            Kind::Metronome { bpm, accent } => {
                let beat = seconds * bpm / 60.0;
                let local = beat.fract() * 60.0 / bpm;
                let frequency = if (beat.floor() as u64).is_multiple_of(u64::from(accent)) {
                    1800.0
                } else {
                    1100.0
                };
                sine(frequency * local) * libm::exp(-local * 100.0)
            }
            Kind::Drums { bpm, steps } => {
                let step = seconds * bpm / 15.0;
                let index = step.floor() as u64;
                let duration = 15.0 / bpm;
                let mut total = 0.0;
                for previous in 0..16_u64 {
                    if previous > index {
                        break;
                    }
                    let local = (step.fract() + previous as f64) * duration;
                    if local > 0.6 {
                        break;
                    }
                    let mask = steps[((index - previous) % 16) as usize];
                    let noise = self::noise(
                        Wave::White,
                        (local * f64::from(self.rate)).floor() as u64,
                        self.seed ^ motion_core::noise::hash32((index - previous) as u32),
                    );
                    if mask & 1 != 0 {
                        total += sine(45.0 * local + 1.2 * (1.0 - libm::exp(-40.0 * local)))
                            * libm::exp(-local * 15.0)
                            * 0.8;
                    }
                    if mask & 2 != 0 {
                        total += (noise * 0.8 + sine(180.0 * local) * 0.2)
                            * libm::exp(-local * 30.0)
                            * 0.5;
                    }
                    if mask & 4 != 0 {
                        total += noise * libm::exp(-local * 80.0) * 0.2;
                    }
                }
                total
            }
            Kind::Effect(sound) => match sound {
                Sound::Click => tone(2400.0, 200.0),
                Sound::Tick => tone(1600.0, 120.0),
                Sound::Pop => tone(220.0, 35.0),
                Sound::Whoosh => {
                    (noise(Wave::Pink) * 0.8
                        + sine(100.0 * seconds + 800.0 * seconds * seconds / duration) * 0.2)
                        * sine(0.5 * progress).powi(2)
                }
                Sound::Riser => {
                    (noise(Wave::White) * 0.5
                        + sine(100.0 * seconds + 1600.0 * seconds * seconds / duration) * 0.5)
                        * progress
                        * (1.0 - progress).min(0.02)
                        * 50.0
                }
                Sound::Impact | Sound::Kick => {
                    (sine(45.0 * seconds + 1.2 * (1.0 - libm::exp(-seconds * 40.0))) * 0.8
                        + noise(Wave::White) * libm::exp(-seconds * 70.0) * 0.2)
                        * libm::exp(-seconds * 12.0)
                }
                Sound::SubDrop => {
                    sine(100.0 * seconds - 35.0 * seconds * seconds / duration) * (1.0 - progress)
                }
                Sound::Snare => {
                    (noise(Wave::White) * 0.8 + tone(180.0, 20.0) * 0.2)
                        * libm::exp(-seconds * 20.0)
                }
                Sound::Hat => noise(Wave::White) * libm::exp(-seconds * 65.0),
                Sound::Ambience => {
                    noise(Wave::Brown)
                        * (0.5 + 0.1 * sine(seconds * 0.3 / std::f64::consts::TAU))
                        * 0.15
                }
            },
        };
        let edge = if matches!(self.kind, Kind::Effect(_)) {
            (seconds * 1000.0).min(1.0) * ((duration - seconds) * 1000.0).clamp(0.0, 1.0)
        } else {
            1.0
        };
        (output.clamp(-1.0, 1.0) * self.gain * edge) as f32
    }
}

/// `sin(TAU * cycles)` from a shared table with linear interpolation. Per-sample oscillators
/// use it instead of evaluating `libm::sin`; the table is fixed, so output stays deterministic.
fn sine(cycles: f64) -> f64 {
    static TABLE: std::sync::OnceLock<[f64; SINE_SEGMENTS + 1]> = std::sync::OnceLock::new();
    let table = TABLE.get_or_init(|| {
        std::array::from_fn(|index| {
            libm::sin(std::f64::consts::TAU * index as f64 / SINE_SEGMENTS as f64)
        })
    });
    let position = cycles.rem_euclid(1.0) * SINE_SEGMENTS as f64;
    let index = (position as usize).min(SINE_SEGMENTS - 1);
    table[index] + (table[index + 1] - table[index]) * (position - index as f64)
}

fn poly_blep(phase: f64, step: f64) -> f64 {
    if phase < step {
        let value = phase / step;
        value + value - value * value - 1.0
    } else if phase > 1.0 - step {
        let value = (phase - 1.0) / step;
        value * value + value + value + 1.0
    } else {
        0.0
    }
}

fn noise(wave: Wave, frame: u64, seed: u32) -> f64 {
    if wave == Wave::White {
        return f64::from(motion_core::noise::hash32(
            frame as u32 ^ (frame >> 32) as u32 ^ seed,
        )) / f64::from(u32::MAX)
            * 2.0
            - 1.0;
    }
    let mut sum = 0.0;
    let mut total = 0.0;
    for octave in 0..12_u32 {
        let scale = f64::from(1_u32 << octave);
        let weight = if wave == Wave::Brown {
            libm::sqrt(scale)
        } else {
            1.0
        };
        sum += motion_core::noise::value2(
            [frame as f64 / scale, 0.0],
            seed.wrapping_add(octave * 7919),
        ) * weight;
        total += weight;
    }
    sum / total
}

impl AudioSource for Synth {
    fn sample_rate(&self) -> u32 {
        self.rate
    }

    fn channels(&self) -> u16 {
        1
    }

    fn duration(&self) -> Option<Duration> {
        self.duration
    }

    fn read(&self, start: u64, output: &mut [f32]) -> usize {
        output.fill(0.0);
        let count = self
            .duration
            .map_or(output.len(), |duration| {
                motion_core::audio::ceil_frames(duration, self.rate)
                    .unwrap_or(0)
                    .saturating_sub(start)
                    .min(output.len() as u64) as usize
            })
            .min(
                u64::MAX
                    .saturating_sub(start)
                    .saturating_add(1)
                    .min(usize::MAX as u64) as usize,
            );
        for (index, sample) in output[..count].iter_mut().enumerate() {
            *sample = self.sample(start + index as u64);
        }
        count
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use motion_core::TimeLiteral;

    #[test]
    fn every_wave_and_sound_is_finite_and_seek_independent() -> motion_core::Result {
        let mut sources = Vec::new();
        for wave in [
            Wave::Sine,
            Wave::Saw,
            Wave::Square,
            Wave::Triangle,
            Wave::White,
            Wave::Pink,
            Wave::Brown,
        ] {
            sources.push(Synth::new(wave, 440.0, 48000, 71)?.chirp(880.0, 100.milliseconds())?);
        }
        for sound in [
            Sound::Whoosh,
            Sound::Click,
            Sound::Pop,
            Sound::Tick,
            Sound::Riser,
            Sound::Impact,
            Sound::SubDrop,
            Sound::Kick,
            Sound::Snare,
            Sound::Hat,
            Sound::Ambience,
        ] {
            sources.push(Synth::sound(sound, 100.milliseconds(), 71)?);
        }
        for source in sources {
            let mut whole = vec![0.0; 4800];
            source.read(0, &mut whole);
            let mut window = [0.0; 37];
            source.read(731, &mut window);
            assert_eq!(window, whole[731..768]);
            assert!(
                whole
                    .iter()
                    .all(|sample| sample.is_finite() && sample.abs() <= 1.0)
            );
            assert!(whole.iter().any(|sample| sample.abs() > 1e-4));
            assert_eq!(source.read(4800, &mut window), 0);
            assert_eq!(window, [0.0; 37]);
        }
        let envelope = Adsr::new(
            10.milliseconds(),
            10.milliseconds(),
            0.5,
            20.milliseconds(),
            100.milliseconds(),
        )?;
        assert_eq!(envelope.sample(Time::ZERO), 0.0);
        assert_eq!(envelope.sample(10.milliseconds()), 1.0);
        assert_eq!(envelope.sample(100.milliseconds()), 0.0);
        Ok(())
    }

    #[test]
    fn drum_machine_and_extreme_random_reads_are_deterministic() -> motion_core::Result {
        let source = Synth::drum_machine(
            120.0,
            [5, 0, 4, 0, 6, 0, 4, 0, 5, 0, 4, 0, 6, 0, 4, 0],
            2.seconds(),
            71,
        )?;
        let mut whole = vec![0.0; 96000];
        source.read(0, &mut whole);
        let mut window = [0.0; 83];
        source.read(24931, &mut window);
        assert_eq!(window, whole[24931..25014]);
        assert!(whole.iter().all(|value| value.is_finite()));
        let source = Synth::new(Wave::White, 440.0, 48000, 71)?;
        assert_eq!(source.read(u64::MAX, &mut window), 1);
        assert!(window[0].is_finite());
        Ok(())
    }

    #[test]
    fn table_sine_tracks_libm_within_its_documented_error() {
        let mut worst = 0.0_f64;
        for step in -200_000..=200_000 {
            // Includes negative phases, exact table nodes and phases far beyond one cycle.
            for cycles in [f64::from(step) * 1.0e-5, f64::from(step) * 0.37 + 0.123] {
                let error = (sine(cycles) - libm::sin(std::f64::consts::TAU * cycles)).abs();
                worst = worst.max(error);
            }
        }
        assert!(worst < 3.0e-7, "{worst}");
        assert_eq!(sine(0.0), 0.0);
        assert!(sine(f64::NAN).is_nan());
    }
}
