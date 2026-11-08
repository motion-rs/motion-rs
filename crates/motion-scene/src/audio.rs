//! Audio occurrences, buses, effects, ducking and the master chain as plain data.

use crate::value::Animated;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

/// An audio file occurrence on the film clock.
///
/// Times are seconds. Animated `volume`, `pan`, `spatial` and effect parameters are
/// sampled in seconds since the occurrence starts; expressions read that time as `t`.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct AudioSpec {
    /// Identifier that other clips' `ducked_by` can name; unique and declared earlier.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub id: Option<String>,
    /// File path, relative to the document directory.
    pub source: String,
    /// Film start in seconds; zero by default.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub start: Option<f64>,
    /// Source offset in seconds; zero by default.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub offset: Option<f64>,
    /// Played duration in seconds; the rest of the source by default.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub duration: Option<f64>,
    /// Linear gain in `0..=16`; animated values become a volume envelope.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub volume: Option<Animated<f64>>,
    /// Fade-in seconds.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub fade_in: Option<f64>,
    /// Fade-out seconds.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub fade_out: Option<f64>,
    /// Stereo pan in `-1..=1`, left to right; animated values pan over the clip.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub pan: Option<Animated<f64>>,
    /// Pan gain law; constant power by default.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub pan_law: Option<PanLawSpec>,
    /// Stereo width in `0..=2`; one keeps the source image.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub stereo_width: Option<f64>,
    /// Position-driven pan and distance attenuation.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub spatial: Option<SpatialSpec>,
    /// Name of a declared bus to route to; the master by default.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub bus: Option<String>,
    /// Effect chain applied in order, at most 16; requires feature `audio`.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub effects: Vec<EffectSpec>,
    /// Lower this clip while the named triggers play, at most 4.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub ducked_by: Vec<DuckSpec>,
}

impl AudioSpec {
    /// Plays a file from film time zero.
    pub fn new(source: impl Into<String>) -> Self {
        Self {
            id: None,
            source: source.into(),
            start: None,
            offset: None,
            duration: None,
            volume: None,
            fade_in: None,
            fade_out: None,
            pan: None,
            pan_law: None,
            stereo_width: None,
            spatial: None,
            bus: None,
            effects: Vec::new(),
            ducked_by: Vec::new(),
        }
    }

    /// Names the clip so later clips can duck under it.
    pub fn id(mut self, id: impl Into<String>) -> Self {
        self.id = Some(id.into());
        self
    }

    /// Sets the gain; animate it for a volume envelope.
    pub fn volume(mut self, volume: impl Into<Animated<f64>>) -> Self {
        self.volume = Some(volume.into());
        self
    }

    /// Sets the pan; animate it to sweep across the stereo field.
    pub fn pan(mut self, pan: impl Into<Animated<f64>>) -> Self {
        self.pan = Some(pan.into());
        self
    }

    /// Routes the clip to a declared bus.
    pub fn bus(mut self, bus: impl Into<String>) -> Self {
        self.bus = Some(bus.into());
        self
    }

    /// Appends an effect.
    pub fn effect(mut self, effect: EffectSpec) -> Self {
        self.effects.push(effect);
        self
    }

    /// Ducks the clip under a trigger.
    pub fn ducked_by(mut self, duck: DuckSpec) -> Self {
        self.ducked_by.push(duck);
        self
    }
}

/// Pan gain law.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum PanLawSpec {
    /// Linear gains.
    Linear,
    /// Equal-power gains.
    #[default]
    ConstantPower,
    /// Equal-power with a 4.5 dB center attenuation.
    #[serde(rename = "minus_4_5_db")]
    Minus4Point5Db,
}

/// Position-driven spatialization against a fixed listener.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct SpatialSpec {
    /// Source position `[x, y]` in design units.
    pub position: Animated<[f64; 2]>,
    /// Listener position `[x, y]`.
    pub listener: [f64; 2],
    /// Distance over which the source pans fully left or right; positive.
    pub width: f64,
    /// Distance attenuation in `0..=1000`.
    pub rolloff: f64,
}

/// A condition that lowers a clip or bus while its trigger is audible.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct DuckSpec {
    /// What listens: an earlier clip or a bus.
    pub trigger: DuckTriggerSpec,
    /// Level that starts ducking in dBFS, `-96..=0`; -45 by default.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub threshold_db: Option<f64>,
    /// Attenuation depth in dB, `0..=96`; 12 by default.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub depth_db: Option<f64>,
    /// Compression ratio in `1..=100`; 100 by default.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub ratio: Option<f64>,
    /// Attack seconds in `0..=5`; 0.08 by default.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub attack: Option<f64>,
    /// Release seconds in `0..=10`; 0.4 by default.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub release: Option<f64>,
    /// Lookahead seconds in `0..=0.5`; 0.01 by default.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub lookahead: Option<f64>,
}

impl DuckSpec {
    /// Ducks under `trigger` with the engine's default dynamics.
    pub fn new(trigger: DuckTriggerSpec) -> Self {
        Self {
            trigger,
            threshold_db: None,
            depth_db: None,
            ratio: None,
            attack: None,
            release: None,
            lookahead: None,
        }
    }
}

/// The signal a duck listens to, written as `{"clip": "voice"}` or `{"bus": "dialogue"}`.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case", deny_unknown_fields)]
pub enum DuckTriggerSpec {
    /// The `id` of an earlier audio clip.
    Clip(String),
    /// The name of an earlier bus.
    Bus(String),
}

/// A named submix with its own gain, effects and ducking.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct BusSpec {
    /// Unique name of 1 to 128 bytes.
    pub name: String,
    /// Gain in dB, `-96..=24`; zero by default.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub gain_db: Option<f64>,
    /// Effect chain, at most 16; requires feature `audio`.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub effects: Vec<EffectSpec>,
    /// Triggers that lower the whole bus; bus triggers must be declared earlier.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub ducked_by: Vec<DuckSpec>,
}

impl BusSpec {
    /// Declares a unity-gain bus.
    pub fn new(name: impl Into<String>) -> Self {
        Self {
            name: name.into(),
            gain_db: None,
            effects: Vec::new(),
            ducked_by: Vec::new(),
        }
    }
}

/// Final output processing.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct MasterSpec {
    /// Gain in dB, `-96..=24`; zero by default.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub gain_db: Option<f64>,
    /// True-peak style limiter.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub limiter: Option<LimiterSpec>,
    /// Loudness normalization.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub normalize: Option<LoudnessSpec>,
}

/// Limiter settings.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct LimiterSpec {
    /// Ceiling in dBFS, `-24..=0`.
    pub ceiling_db: f64,
    /// Lookahead seconds in `0..=0.02`; 0.005 by default.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub lookahead: Option<f64>,
    /// Release seconds in `(0, 2]`; 0.1 by default.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub release: Option<f64>,
}

/// Loudness normalization target.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct LoudnessSpec {
    /// Integrated target in LUFS, `-70..=-5`.
    pub target_lufs: f64,
    /// True-peak ceiling in dBFS, `-24..=0`.
    pub true_peak_db: f64,
}

/// EQ filter shape.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum EqKindSpec {
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
    /// Band rejection.
    Notch,
}

/// An offline audio effect, written as `{"reverb": {...}}` and similar. Effects
/// need feature `audio`; their ranges are checked when the timeline is validated.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case", deny_unknown_fields)]
pub enum EffectSpec {
    /// Biquad equalizer; `frequency` and `gain_db` may be automated.
    Eq {
        /// Filter shape.
        kind: EqKindSpec,
        /// Center or corner frequency in hertz, below Nyquist.
        frequency: Animated<f64>,
        /// Quality factor in `0.1..=20`; 0.707 by default.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        q: Option<f64>,
        /// Gain in dB, `-24..=24`; zero by default.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        gain_db: Option<Animated<f64>>,
    },
    /// Downward compressor.
    Compressor {
        /// Threshold in dBFS, `-96..=0`.
        threshold_db: f64,
        /// Ratio in `1..=100`.
        ratio: f64,
        /// Attack seconds in `(0, 2]`; 0.01 by default.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        attack: Option<f64>,
        /// Release seconds in `(0, 2]`; 0.1 by default.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        release: Option<f64>,
    },
    /// Noise gate.
    Gate {
        /// Threshold in dBFS, `-96..=0`.
        threshold_db: f64,
        /// Attack seconds in `(0, 2]`; 0.005 by default.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        attack: Option<f64>,
        /// Release seconds in `(0, 2]`; 0.1 by default.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        release: Option<f64>,
    },
    /// Sibilance reducer.
    DeEsser {
        /// Center frequency in hertz.
        frequency: f64,
        /// Threshold in dBFS, `-96..=0`.
        threshold_db: f64,
        /// Ratio in `1..=100`.
        ratio: f64,
    },
    /// Feedback delay.
    Delay {
        /// Delay seconds in `(0, 5]`.
        delay: f64,
        /// Feedback magnitude below 0.99; 0.3 by default.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        feedback: Option<f64>,
        /// Wet mix in `0..=1`; 0.3 by default.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        wet: Option<f64>,
        /// Extra tail seconds in `0..=30`; 1 by default.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        tail: Option<f64>,
    },
    /// Room reverb.
    Reverb {
        /// Room size in `0..=1`.
        room: f64,
        /// Damping in `0..=1`; 0.5 by default.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        damping: Option<f64>,
        /// Wet mix in `0..=1`; 0.3 by default.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        wet: Option<f64>,
        /// Extra tail seconds in `0..=30`; 2 by default.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        tail: Option<f64>,
    },
}
