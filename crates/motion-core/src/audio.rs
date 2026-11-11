//! Stateless audio sources and immutable timeline occurrence metadata.
use crate::{Duration, ValidationError};
use std::sync::Arc;

mod clip;
mod metadata;

pub use clip::{AudioClip, AudioClipParameters};
pub use metadata::{AudioChapter, AudioMetadata};

fn invalid(field: &str, message: &str) -> ValidationError {
    ValidationError::invalid("audio", field, message)
}

/// A deterministic offline effect. Mixers process from the occurrence origin once,
/// retain the result, then serve exact random-access windows without IIR warmup errors.
pub trait AudioEffect: Send + Sync + std::fmt::Debug {
    /// Validates parameters at the native source rate, without processing PCM.
    fn validate(&self, rate: u32) -> Result<(), ValidationError>;

    /// Extra silent input needed to render a bounded decay tail.
    fn tail(&self) -> Duration {
        Duration::ZERO
    }

    /// Processes complete interleaved frames from a zero initial state.
    fn process(&self, input: &mut [f32], channels: u16, rate: u32) -> crate::Result;
}

/// Stereo balance law. Mono panning always uses constant power.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum PanLaw {
    /// Linear balance: attenuates only the opposite channel.
    Linear,
    /// Sine/cosine balance normalized to unity at the center; the default.
    #[default]
    ConstantPower,
    /// Intermediate 4.5 dB pan law, normalized at the center.
    Minus4_5dB,
}

/// Offline peak limiter parameters. The release is a finite linear gain recovery.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Limiter {
    /// Maximum reconstructed peak in dBFS, in -24..=0.
    pub ceiling_db: f64,
    /// Future peak inspection interval, at most 20 milliseconds.
    pub lookahead: Duration,
    /// Gain recovery interval, in (0,2] seconds.
    pub release: Duration,
}

/// EBU R128 target and reconstructed peak ceiling for static normalization.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Loudness {
    /// Target integrated loudness, in -70..=-5 LUFS.
    pub target_lufs: f64,
    /// Maximum reconstructed peak after gain, in -24..=0 dBFS.
    pub true_peak_db: f64,
}

/// Optional master processing; the default leaves the mix unchanged.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Master {
    /// Static gain in -96..=24 dB.
    pub gain_db: f64,
    /// Optional lookahead peak limiting.
    pub limiter: Option<Limiter>,
    /// Optional gated integrated loudness normalization.
    pub normalize: Option<Loudness>,
}

impl Master {
    /// Conservative online-video limiter preset; normalization remains opt-in.
    pub fn safe() -> Self {
        Self {
            limiter: Some(Limiter {
                ceiling_db: -1.0,
                lookahead: Duration::from_ticks(crate::TICKS_PER_SECOND as u64 / 200),
                release: Duration::from_ticks(crate::TICKS_PER_SECOND as u64 / 10),
            }),
            ..Self::default()
        }
    }

    /// Checks all fields, including disabled processing branches.
    pub fn validate(self) -> Result<(), ValidationError> {
        if !self.gain_db.is_finite() || !(-96.0..=24.0).contains(&self.gain_db) {
            return Err(invalid("master_gain", "Require finite -96..=24 dB"));
        }
        if self.limiter.is_some_and(|limit| {
            !limit.ceiling_db.is_finite()
                || !(-24.0..=0.0).contains(&limit.ceiling_db)
                || limit.lookahead.as_seconds() > 0.02
                || limit.release <= Duration::ZERO
                || limit.release.as_seconds() > 2.0
        }) {
            return Err(invalid("limiter", "Invalid ceiling, lookahead or release"));
        }
        if self.normalize.is_some_and(|normal| {
            !normal.target_lufs.is_finite()
                || !(-70.0..=-5.0).contains(&normal.target_lufs)
                || !normal.true_peak_db.is_finite()
                || !(-24.0..=0.0).contains(&normal.true_peak_db)
        }) {
            return Err(invalid(
                "normalize",
                "Invalid LUFS target or true peak ceiling",
            ));
        }
        Ok(())
    }
}

/// Sidechain ducking parameters. The trigger's peak-hold envelope is converted to a gain
/// reduction in dB, smoothed by attack/release and applied to the ducked signal.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Duck {
    /// Trigger level above which ducking starts, in -96..=0 dBFS.
    pub threshold_db: f64,
    /// Maximum gain reduction, in 0..=96 dB.
    pub depth_db: f64,
    /// Compression ratio above the threshold, in 1..=100; reduction is capped by `depth_db`.
    pub ratio: f64,
    /// Duration to reach a deeper reduction, in 0..=5 seconds.
    pub attack: Duration,
    /// Duration to recover after the trigger falls, in 0..=10 seconds.
    pub release: Duration,
    /// Interval by which ducking anticipates the trigger, in 0..=500 milliseconds.
    pub lookahead: Duration,
}

impl Default for Duck {
    fn default() -> Self {
        Self {
            threshold_db: -45.0,
            depth_db: 12.0,
            ratio: 100.0,
            attack: Duration::from_ticks(crate::TICKS_PER_SECOND as u64 / 1000 * 80),
            release: Duration::from_ticks(crate::TICKS_PER_SECOND as u64 / 1000 * 400),
            lookahead: Duration::from_ticks(crate::TICKS_PER_SECOND as u64 / 100),
        }
    }
}

impl Duck {
    /// Creates ducking with the given threshold and depth and default timing.
    pub fn new(threshold_db: f64, depth_db: f64) -> Self {
        Self {
            threshold_db,
            depth_db,
            ..Self::default()
        }
    }

    /// Sets the compression ratio above the threshold.
    pub fn ratio(mut self, ratio: f64) -> Self {
        self.ratio = ratio;
        self
    }

    /// Sets attack and release times.
    pub fn timing(mut self, attack: Duration, release: Duration) -> Self {
        self.attack = attack;
        self.release = release;
        self
    }

    /// Sets the anticipation interval.
    pub fn lookahead(mut self, lookahead: Duration) -> Self {
        self.lookahead = lookahead;
        self
    }

    /// Checks finite bounded parameters.
    pub fn validate(self) -> Result<(), ValidationError> {
        let seconds =
            |time: Duration, max: f64| time.as_seconds().is_finite() && time.as_seconds() <= max;
        if !self.threshold_db.is_finite()
            || !(-96.0..=0.0).contains(&self.threshold_db)
            || !self.depth_db.is_finite()
            || !(0.0..=96.0).contains(&self.depth_db)
            || !self.ratio.is_finite()
            || !(1.0..=100.0).contains(&self.ratio)
            || !seconds(self.attack, 5.0)
            || !seconds(self.release, 10.0)
            || !seconds(self.lookahead, 0.5)
        {
            return Err(invalid(
                "duck",
                "Require threshold -96..=0 dB, depth 0..=96 dB, ratio 1..=100, attack <=5 s, release <=10 s and lookahead <=0.5 s",
            ));
        }
        Ok(())
    }
}

/// Signal that drives a [`Duck`].
#[derive(Clone, Debug)]
pub enum DuckTrigger {
    /// Another occurrence, rendered standalone at its own gain and pan. Its own ducking is ignored.
    Clip(Arc<AudioClip>),
    /// An earlier-declared bus, measured after its effects, gain and ducking.
    Bus(String),
}

/// A sidechain ducking assignment.
#[derive(Clone, Debug)]
pub struct Ducking {
    /// Signal that lowers the target.
    pub trigger: DuckTrigger,
    /// Detector and gain-reduction parameters.
    pub duck: Duck,
}

/// Named submix processed before the master. At most sixteen buses are admitted.
#[derive(Clone, Debug)]
pub struct AudioBus {
    /// Stable name matched by AudioClip::bus.
    pub name: String,
    /// Static gain, in -96..=24 dB.
    pub gain_db: f64,
    /// Deterministic offline effects, in declaration order.
    pub effects: Vec<Arc<dyn AudioEffect>>,
    /// Sidechain ducking applied after effects and gain; bus triggers must be declared earlier.
    pub ducking: Vec<Ducking>,
}

impl AudioBus {
    /// Creates an identity submix with the supplied name.
    pub fn new(name: impl Into<String>) -> Self {
        Self {
            name: name.into(),
            gain_db: 0.0,
            effects: Vec::new(),
            ducking: Vec::new(),
        }
    }

    /// Ducks the whole submix under a clip or earlier bus.
    pub fn ducked_by(mut self, trigger: DuckTrigger, duck: Duck) -> Self {
        self.ducking.push(Ducking { trigger, duck });
        self
    }
}

/// A pure, concurrent random-access interleaved PCM source.
/// Repeated reads must not depend on prior requests. Implementations return frames,
/// never samples; EOF/short reads leave the rest of the caller's buffer silent.
pub trait AudioSource: Send + Sync {
    /// Deferred child mix, resolved by the encoding boundary before PCM queries.
    fn composition(&self) -> Option<&crate::Timeline> {
        None
    }

    /// Optional local file identity, used by execution boundaries for lazy decoding.
    fn file_path(&self) -> Option<&std::path::Path> {
        None
    }

    /// Checked reads let file-backed sources propagate decoding failures.
    fn try_read(&self, start_frame: u64, output: &mut [f32]) -> crate::Result<usize> {
        Ok(self.read(start_frame, output))
    }

    /// Native PCM sampling frequency, in frames per second.
    fn sample_rate(&self) -> u32;

    /// Interleaved channel count; mixing supports mono and stereo.
    fn channels(&self) -> u16;

    /// Reads complete frames at an absolute source offset. The return value must
    /// not exceed output.len()/channels; all returned samples must be finite.
    fn read(&self, start_frame: u64, output: &mut [f32]) -> usize;

    /// Finite source duration, or None for a procedurally unbounded source.
    fn duration(&self) -> Option<Duration>;
}

impl<Source: AudioSource + ?Sized> AudioSource for Arc<Source> {
    fn composition(&self) -> Option<&crate::Timeline> {
        self.as_ref().composition()
    }

    fn file_path(&self) -> Option<&std::path::Path> {
        self.as_ref().file_path()
    }

    fn try_read(&self, start: u64, output: &mut [f32]) -> crate::Result<usize> {
        self.as_ref().try_read(start, output)
    }

    fn sample_rate(&self) -> u32 {
        self.as_ref().sample_rate()
    }

    fn channels(&self) -> u16 {
        self.as_ref().channels()
    }

    fn read(&self, start: u64, output: &mut [f32]) -> usize {
        self.as_ref().read(start, output)
    }

    fn duration(&self) -> Option<Duration> {
        self.as_ref().duration()
    }
}

/// Converts ticks to the nearest audio frame, with half ties upward.
/// A zero sampling rate produces zero frames.
pub fn frames(time: Duration, rate: u32) -> u64 {
    ((u128::from(time.ticks()) * u128::from(rate) + crate::TICKS_PER_SECOND as u128 / 2)
        / crate::TICKS_PER_SECOND as u128) as u64
}

/// Converts a duration to the smallest covering number of audio frames.
/// Integer arithmetic avoids floating-point endpoint drift.
pub fn ceil_frames(time: Duration, rate: u32) -> u64 {
    (u128::from(time.ticks()) * u128::from(rate)).div_ceil(crate::TICKS_PER_SECOND as u128) as u64
}
