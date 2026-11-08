//! Animated property values, motion sources, easing curves and serialized colors.

use crate::options::WaveformSpec;
use motion_core::{Color, Easing};
use schemars::{JsonSchema, Schema, SchemaGenerator, json_schema};
use serde::de::{DeserializeOwned, Error as _};
use serde::{Deserialize, Deserializer, Serialize, Serializer};
use std::borrow::Cow;

/// A property that is either a fixed value or sampled from scene-local time.
///
/// JSON writes a fixed value directly (`12`, `[1, 2]`, `"#fff"`). A motion is an
/// object with exactly one key: `tween`, `keyframes`, `oscillate` or `noise`.
/// Because no fixed value is a JSON object, the two forms never overlap.
///
/// ```
/// use motion_scene::{Animated, EasingSpec, TweenSpec};
/// let fixed: Animated<f64> = serde_json::from_str("12")?;
/// assert_eq!(fixed, Animated::Fixed(12.0));
/// let moving: Animated<f64> =
///     serde_json::from_str(r#"{"tween": {"from": 0, "to": 1, "duration": 2, "easing": "cubic_out"}}"#)?;
/// assert_eq!(
///     moving,
///     TweenSpec::new(0.0, 1.0, 2.0).easing(EasingSpec::CubicOut).into()
/// );
/// # Ok::<(), serde_json::Error>(())
/// ```
#[derive(Clone, Debug, PartialEq)]
pub enum Animated<T> {
    /// A value that does not change over time.
    Fixed(T),
    /// A value sampled from scene-local time.
    Motion(Box<MotionSpec<T>>),
}

impl<T> Animated<T> {
    /// Interpolates linearly from `from` to `to` over `duration` seconds.
    pub fn tween(from: T, to: T, duration: f64) -> Self {
        TweenSpec::new(from, to, duration).into()
    }

    /// Interpolates through explicit keys, ordered by strictly increasing time.
    pub fn keyframes(keys: impl IntoIterator<Item = KeyframeSpec<T>>) -> Self {
        KeyframesSpec::new(keys).into()
    }

    /// Reports whether this property is constant for every time.
    pub fn is_fixed(&self) -> bool {
        matches!(self, Self::Fixed(_))
    }
}

impl<T> From<T> for Animated<T> {
    fn from(value: T) -> Self {
        Self::Fixed(value)
    }
}

impl<T> From<TweenSpec<T>> for Animated<T> {
    fn from(tween: TweenSpec<T>) -> Self {
        Self::Motion(Box::new(MotionSpec::Tween(tween)))
    }
}

impl<T> From<KeyframesSpec<T>> for Animated<T> {
    fn from(keyframes: KeyframesSpec<T>) -> Self {
        Self::Motion(Box::new(MotionSpec::Keyframes(keyframes)))
    }
}

impl From<OscillateSpec> for Animated<f64> {
    fn from(oscillate: OscillateSpec) -> Self {
        Self::Motion(Box::new(MotionSpec::Oscillate(oscillate)))
    }
}

impl<T> Animated<T> {
    /// Computes a number from an expression of time, parameters and other elements.
    pub fn expr(source: impl Into<String>) -> Self {
        Self::Motion(Box::new(MotionSpec::Expr(ExprSpec::One(source.into()))))
    }

    /// Computes each component of a point from its own expression.
    pub fn exprs(sources: impl IntoIterator<Item = impl Into<String>>) -> Self {
        let sources = sources.into_iter().map(Into::into).collect();
        Self::Motion(Box::new(MotionSpec::Expr(ExprSpec::Many(sources))))
    }
}

impl From<NoiseSpec> for Animated<f64> {
    fn from(noise: NoiseSpec) -> Self {
        Self::Motion(Box::new(MotionSpec::Noise(noise)))
    }
}

impl From<Color> for Animated<ColorValue> {
    fn from(color: Color) -> Self {
        Self::Fixed(color.into())
    }
}

impl<T: Serialize> Serialize for Animated<T> {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        match self {
            Self::Fixed(value) => value.serialize(serializer),
            Self::Motion(motion) => motion.serialize(serializer),
        }
    }
}

impl<'de, T: DeserializeOwned> Deserialize<'de> for Animated<T> {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let value = serde_json::Value::deserialize(deserializer)?;
        if value.is_object() {
            MotionSpec::deserialize(value)
                .map(|motion| Self::Motion(Box::new(motion)))
                .map_err(D::Error::custom)
        } else {
            T::deserialize(value)
                .map(Self::Fixed)
                .map_err(D::Error::custom)
        }
    }
}

impl<T: JsonSchema> JsonSchema for Animated<T> {
    fn schema_name() -> Cow<'static, str> {
        format!("Animated_{}", T::schema_name()).into()
    }

    fn schema_id() -> Cow<'static, str> {
        format!("motion_scene::Animated<{}>", T::schema_id()).into()
    }

    fn json_schema(generator: &mut SchemaGenerator) -> Schema {
        let fixed = generator.subschema_for::<T>();
        let motion = generator.subschema_for::<MotionSpec<T>>();
        json_schema!({
            "description": "A fixed value, or an object selecting one time-varying motion.",
            "anyOf": [fixed, motion]
        })
    }
}

/// A time-varying source for an [`Animated`] property.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case", deny_unknown_fields)]
pub enum MotionSpec<T> {
    /// Two-value interpolation.
    Tween(TweenSpec<T>),
    /// Multi-key interpolation.
    Keyframes(KeyframesSpec<T>),
    /// A periodic numeric waveform; numeric properties only.
    Oscillate(OscillateSpec),
    /// Deterministic fractal value noise; numeric properties only.
    Noise(NoiseSpec),
    /// An arithmetic expression of scene-local time, parameters and other elements.
    Expr(ExprSpec),
}

/// Expression source text: one string for a number, one string per component for a point.
///
/// Expressions combine numbers, `t` (scene-local seconds), `pi`, `tau`,
/// `$parameter`, `@id.property` bindings to other elements, `+ - * / % ^`,
/// comparisons and functions such as `sin`, `min`, `clamp`, `lerp`, `smoothstep` and
/// `if`. `@id.property` reads the other element's own property value, for example
/// `@dot.position.x` or `@dot.circle.radius`.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(untagged)]
pub enum ExprSpec {
    /// A single numeric expression.
    One(String),
    /// One expression per component of a point or rectangle.
    Many(Vec<String>),
}

impl ExprSpec {
    /// Returns each component's source text.
    pub fn sources(&self) -> &[String] {
        match self {
            Self::One(source) => std::slice::from_ref(source),
            Self::Many(sources) => sources,
        }
    }
}

/// Interpolation between two values starting at scene-local time zero.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct TweenSpec<T> {
    /// Value at and before the start.
    pub from: T,
    /// Value at and after the end.
    pub to: T,
    /// Nonnegative duration in seconds.
    pub duration: f64,
    /// Progress curve; linear by default.
    #[serde(default, skip_serializing_if = "EasingSpec::is_linear")]
    pub easing: EasingSpec,
    /// Nonnegative start offset in seconds, holding `from` before it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub delay: Option<f64>,
    /// Total number of plays; the final play holds its end value.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub repeat: Option<u32>,
    /// Plays forward then backward, ending at `from`.
    #[serde(default, skip_serializing_if = "is_false")]
    pub ping_pong: bool,
}

impl<T> TweenSpec<T> {
    /// Creates a linear tween over `duration` seconds.
    pub fn new(from: T, to: T, duration: f64) -> Self {
        Self {
            from,
            to,
            duration,
            easing: EasingSpec::Linear,
            delay: None,
            repeat: None,
            ping_pong: false,
        }
    }

    /// Selects the progress curve.
    pub fn easing(mut self, easing: EasingSpec) -> Self {
        self.easing = easing;
        self
    }

    /// Starts after `seconds`, holding the first value before.
    pub fn delay(mut self, seconds: f64) -> Self {
        self.delay = Some(seconds);
        self
    }

    /// Plays `count` times in total.
    pub fn repeat(mut self, count: u32) -> Self {
        self.repeat = Some(count);
        self
    }

    /// Plays forward then backward.
    pub fn ping_pong(mut self) -> Self {
        self.ping_pong = true;
        self
    }
}

/// A key track whose segments use the easing of the key they arrive at.
///
/// A first key later than zero holds its value from zero until that time.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct KeyframesSpec<T> {
    /// Nonempty keys with nonnegative, strictly increasing times.
    pub keys: Vec<KeyframeSpec<T>>,
    /// Nonnegative start offset in seconds.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub delay: Option<f64>,
    /// Total number of plays; the final play holds its end value.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub repeat: Option<u32>,
    /// Plays forward then backward, ending at the first value.
    #[serde(default, skip_serializing_if = "is_false")]
    pub ping_pong: bool,
}

impl<T> KeyframesSpec<T> {
    /// Creates a track from ordered keys.
    pub fn new(keys: impl IntoIterator<Item = KeyframeSpec<T>>) -> Self {
        Self {
            keys: keys.into_iter().collect(),
            delay: None,
            repeat: None,
            ping_pong: false,
        }
    }

    /// Starts after `seconds`.
    pub fn delay(mut self, seconds: f64) -> Self {
        self.delay = Some(seconds);
        self
    }

    /// Plays `count` times in total.
    pub fn repeat(mut self, count: u32) -> Self {
        self.repeat = Some(count);
        self
    }

    /// Plays forward then backward.
    pub fn ping_pong(mut self) -> Self {
        self.ping_pong = true;
        self
    }
}

/// One key of a [`KeyframesSpec`].
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct KeyframeSpec<T> {
    /// Scene-local time in seconds.
    pub time: f64,
    /// Value exactly at `time`.
    pub value: T,
    /// Curve of the segment arriving at this key; ignored on the first key.
    #[serde(default, skip_serializing_if = "EasingSpec::is_linear")]
    pub easing: EasingSpec,
}

impl<T> KeyframeSpec<T> {
    /// Creates a key arriving linearly.
    pub fn new(time: f64, value: T) -> Self {
        Self {
            time,
            value,
            easing: EasingSpec::Linear,
        }
    }

    /// Selects the curve of the arriving segment.
    pub fn easing(mut self, easing: EasingSpec) -> Self {
        self.easing = easing;
        self
    }
}

/// A periodic waveform: `offset + amplitude * wave(frequency * t + phase)`.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct OscillateSpec {
    /// Finite, nonnegative frequency in hertz.
    pub frequency: f64,
    /// Wave shape; sine by default.
    #[serde(default, skip_serializing_if = "WaveformSpec::is_sine")]
    pub waveform: WaveformSpec,
    /// Peak deviation; one by default.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub amplitude: Option<f64>,
    /// Phase in whole turns; zero by default.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub phase: Option<f64>,
    /// Center value; zero by default.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub offset: Option<f64>,
}

impl OscillateSpec {
    /// Creates a unit sine wave at `frequency` hertz.
    pub fn new(frequency: f64) -> Self {
        Self {
            frequency,
            waveform: WaveformSpec::Sine,
            amplitude: None,
            phase: None,
            offset: None,
        }
    }

    /// Selects the wave shape.
    pub fn waveform(mut self, waveform: WaveformSpec) -> Self {
        self.waveform = waveform;
        self
    }

    /// Scales the wave.
    pub fn amplitude(mut self, amplitude: f64) -> Self {
        self.amplitude = Some(amplitude);
        self
    }

    /// Shifts the wave by whole turns.
    pub fn phase(mut self, turns: f64) -> Self {
        self.phase = Some(turns);
        self
    }

    /// Adds a center value.
    pub fn offset(mut self, offset: f64) -> Self {
        self.offset = Some(offset);
        self
    }
}

/// Seeded fractal value noise in `-1..=1`, scaled by `amplitude` around `offset`.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct NoiseSpec {
    /// Explicit deterministic seed.
    pub seed: u64,
    /// Finite, nonnegative base frequency in hertz; one by default.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub frequency: Option<f64>,
    /// Octave count in `1..=32`; one by default.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub octaves: Option<u32>,
    /// Positive frequency multiplier per octave; two by default.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub lacunarity: Option<f64>,
    /// Amplitude multiplier per octave in `0..=1`; one half by default.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub gain: Option<f64>,
    /// Output scale; one by default.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub amplitude: Option<f64>,
    /// Center value; zero by default.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub offset: Option<f64>,
}

impl NoiseSpec {
    /// Creates single-octave unit noise at one hertz.
    pub fn new(seed: u64) -> Self {
        Self {
            seed,
            frequency: None,
            octaves: None,
            lacunarity: None,
            gain: None,
            amplitude: None,
            offset: None,
        }
    }

    /// Selects the base frequency.
    pub fn frequency(mut self, hertz: f64) -> Self {
        self.frequency = Some(hertz);
        self
    }

    /// Selects the octave count.
    pub fn octaves(mut self, octaves: u32) -> Self {
        self.octaves = Some(octaves);
        self
    }

    /// Scales the output.
    pub fn amplitude(mut self, amplitude: f64) -> Self {
        self.amplitude = Some(amplitude);
        self
    }

    /// Adds a center value.
    pub fn offset(mut self, offset: f64) -> Self {
        self.offset = Some(offset);
        self
    }
}

/// A serializable mirror of [`Easing`] without custom callbacks.
///
/// Unit curves are strings such as `"cubic_out"`; parameterized curves are
/// objects such as `{"back_out": {"overshoot": 1.7}}` or `{"steps": 4}`.
#[derive(Clone, Copy, Debug, Default, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case", deny_unknown_fields)]
pub enum EasingSpec {
    /// Constant speed.
    #[default]
    Linear,
    /// Sinusoidal acceleration.
    SineIn,
    /// Sinusoidal deceleration.
    SineOut,
    /// Symmetric sinusoidal movement.
    SineInOut,
    /// Quadratic acceleration.
    QuadraticIn,
    /// Quadratic deceleration.
    QuadraticOut,
    /// Symmetric quadratic movement.
    QuadraticInOut,
    /// Cubic acceleration.
    CubicIn,
    /// Cubic deceleration.
    CubicOut,
    /// Symmetric cubic movement.
    CubicInOut,
    /// Quartic acceleration.
    QuarticIn,
    /// Quartic deceleration.
    QuarticOut,
    /// Symmetric quartic movement.
    QuarticInOut,
    /// Exponential acceleration.
    ExponentialIn,
    /// Exponential deceleration.
    ExponentialOut,
    /// Symmetric exponential movement.
    ExponentialInOut,
    /// Initial backward movement before acceleration.
    BackIn {
        /// Finite, nonnegative backward coefficient.
        overshoot: f64,
    },
    /// Overshoots the target before settling.
    BackOut {
        /// Finite, nonnegative overshoot coefficient.
        overshoot: f64,
    },
    /// Mirrored back easing.
    BackInOut {
        /// Finite, nonnegative coefficient for each half.
        overshoot: f64,
    },
    /// Damped oscillation pinned at both endpoints.
    ElasticOut {
        /// Finite amplitude, at least one.
        amplitude: f64,
        /// Finite, strictly positive period in normalized time.
        period: f64,
    },
    /// Piecewise quadratic outward bounce.
    BounceOut,
    /// Right-continuous equal steps; the count must be nonzero.
    Steps(u32),
    /// Unit-endpoint cubic Bezier with X controls in `0..=1`.
    CubicBezier {
        /// First X control.
        x1: f64,
        /// First Y control.
        y1: f64,
        /// Second X control.
        x2: f64,
        /// Second Y control.
        y2: f64,
    },
}

impl EasingSpec {
    /// Reports the default linear curve, which documents omit.
    pub fn is_linear(&self) -> bool {
        matches!(self, Self::Linear)
    }
}

impl From<EasingSpec> for Easing {
    fn from(easing: EasingSpec) -> Self {
        match easing {
            EasingSpec::Linear => Self::Linear,
            EasingSpec::SineIn => Self::SineIn,
            EasingSpec::SineOut => Self::SineOut,
            EasingSpec::SineInOut => Self::SineInOut,
            EasingSpec::QuadraticIn => Self::QuadraticIn,
            EasingSpec::QuadraticOut => Self::QuadraticOut,
            EasingSpec::QuadraticInOut => Self::QuadraticInOut,
            EasingSpec::CubicIn => Self::CubicIn,
            EasingSpec::CubicOut => Self::CubicOut,
            EasingSpec::CubicInOut => Self::CubicInOut,
            EasingSpec::QuarticIn => Self::QuarticIn,
            EasingSpec::QuarticOut => Self::QuarticOut,
            EasingSpec::QuarticInOut => Self::QuarticInOut,
            EasingSpec::ExponentialIn => Self::ExponentialIn,
            EasingSpec::ExponentialOut => Self::ExponentialOut,
            EasingSpec::ExponentialInOut => Self::ExponentialInOut,
            EasingSpec::BackIn { overshoot } => Self::BackIn { overshoot },
            EasingSpec::BackOut { overshoot } => Self::BackOut { overshoot },
            EasingSpec::BackInOut { overshoot } => Self::BackInOut { overshoot },
            EasingSpec::ElasticOut { amplitude, period } => Self::ElasticOut { amplitude, period },
            EasingSpec::BounceOut => Self::BounceOut,
            EasingSpec::Steps(count) => Self::Steps(count),
            EasingSpec::CubicBezier { x1, y1, x2, y2 } => Self::CubicBezier { x1, y1, x2, y2 },
        }
    }
}

/// A CSS-style color string, parsed with [`Color::parse`] and kept verbatim.
///
/// Documents accept named, hexadecimal, `rgb()`, `hsl()` and `oklch()` syntax.
/// Colors built from [`Color`] serialize as `#rrggbb[aa]` when exactly
/// representable in 8 bits, and as `rgb()` with fractional channels otherwise.
#[derive(Clone, Debug, PartialEq)]
pub struct ColorValue {
    text: String,
    color: Color,
}

impl ColorValue {
    /// Parses and retains color syntax.
    pub fn parse(text: impl Into<String>) -> motion_core::Result<Self> {
        let text = text.into();
        let color = Color::parse(&text)?;
        Ok(Self { text, color })
    }

    /// Returns the retained source text.
    pub fn as_str(&self) -> &str {
        &self.text
    }

    /// Returns the parsed color.
    pub fn color(&self) -> Color {
        self.color
    }
}

impl From<Color> for ColorValue {
    fn from(color: Color) -> Self {
        let [red, green, blue, alpha] = color.to_rgba8();
        let text = if Color::rgba(red, green, blue, alpha) == color {
            if alpha == u8::MAX {
                format!("#{red:02x}{green:02x}{blue:02x}")
            } else {
                format!("#{red:02x}{green:02x}{blue:02x}{alpha:02x}")
            }
        } else {
            let [red, green, blue, alpha] = color.to_rgba_float();
            format!(
                "rgb({} {} {} / {alpha})",
                red * 255.0,
                green * 255.0,
                blue * 255.0
            )
        };
        Self { text, color }
    }
}

impl Serialize for ColorValue {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.serialize_str(&self.text)
    }
}

impl<'de> Deserialize<'de> for ColorValue {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        Self::parse(String::deserialize(deserializer)?).map_err(D::Error::custom)
    }
}

impl JsonSchema for ColorValue {
    fn schema_name() -> Cow<'static, str> {
        "Color".into()
    }

    fn json_schema(_: &mut SchemaGenerator) -> Schema {
        json_schema!({
            "type": "string",
            "description": "Named, #hex, rgb(), hsl() or oklch() color syntax.",
            "examples": ["#14a082", "rgb(20 160 130 / 50%)", "white"]
        })
    }
}

pub(crate) fn is_false(value: &bool) -> bool {
    !*value
}
