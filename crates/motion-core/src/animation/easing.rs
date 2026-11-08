//! Double-precision normalized easing curves and their deterministic math.

use std::{fmt, sync::Arc};

#[cfg(test)]
mod tests;

/// A normalized double-precision easing curve, independent of clip duration.
///
/// Finite input is clamped to the unit interval with exact zero/one endpoints.
/// Output may overshoot. Parameters are checked when sampled; invalid parameters,
/// nonfinite progress and nonfinite custom output panic rather than being hidden.
///
/// ```
/// use motion_core::prelude::*;
/// use motion_core::{
///     DisplayList, RasterLimits, ResourcePool, ValidatedDisplayList, MAX_PATH_VERBS,
///     MAX_STROKE_VERBS, MAX_SVG_PATH_BYTES, sample_offsets,
/// };
/// let movement = Tween::new(0.0_f64, 100.0, 2.seconds()).easing(Easing::CubicOut);
/// assert_eq!(movement.sample(1.seconds()), 87.5);
/// ```
#[derive(Clone)]
pub enum Easing {
    /// Constant speed.
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
    /// Base-two exponential acceleration, pinned at zero.
    ExponentialIn,
    /// Base-two exponential deceleration, pinned at one.
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
    /// Mirrored BackIn, using the coefficient directly without an extra multiplier.
    BackInOut {
        /// Finite, nonnegative coefficient for each half.
        overshoot: f64,
    },
    /// Exponentially damped oscillation, pinned at both endpoints.
    ElasticOut {
        /// Finite amplitude, at least one.
        amplitude: f64,
        /// Finite, strictly positive period in normalized time.
        period: f64,
    },
    /// A piecewise quadratic outward bounce.
    BounceOut,
    /// Right-continuous equal steps; the count must be nonzero.
    Steps(u32),
    /// Unit-endpoint cubic Bezier, solving X before evaluating Y.
    CubicBezier {
        /// First X control, finite and in 0..=1.
        x1: f64,
        /// First Y control, finite and allowed to overshoot.
        y1: f64,
        /// Second X control, finite and in 0..=1.
        x2: f64,
        /// Second Y control, finite and allowed to overshoot.
        y2: f64,
    },
    /// A deterministic, thread-safe mapping with finite output.
    /// Input is clamped to 0..=1; callback endpoint values are preserved.
    Custom(Arc<dyn Fn(f64) -> f64 + Send + Sync>),
}

impl Easing {
    /// Evaluates normalized progress with double-precision math.
    /// Built-in transcendental operations use libm, not platform math routines.
    ///
    /// # Panics
    /// Panics for invalid curve parameters, nonfinite progress or nonfinite output.
    pub fn apply(&self, progress: f64) -> f64 {
        self.try_apply(progress)
            .unwrap_or_else(|error| panic!("{}", error.message))
    }

    /// Evaluates progress with recoverable parameter/output diagnostics.
    /// Panics raised inside a user callback still propagate unchanged.
    pub fn try_apply(&self, progress: f64) -> Result<f64, crate::ValidationError> {
        self.try_validate()?;
        if !progress.is_finite() {
            return Err(easing_error("progress", "easing progress must be finite"));
        }
        let progress = progress.clamp(0.0, 1.0);
        if progress == 0.0 && !matches!(self, Self::Custom(_)) {
            return Ok(0.0);
        }
        if progress == 1.0 && !matches!(self, Self::Custom(_)) {
            return Ok(1.0);
        }
        let value = match self {
            Self::Linear => progress,
            Self::SineIn => {
                let sine = libm::sin(progress * std::f64::consts::FRAC_PI_4);
                2.0 * sine * sine
            }
            Self::SineOut => libm::sin(progress * std::f64::consts::FRAC_PI_2),
            Self::SineInOut => {
                if progress == 0.5 {
                    return Ok(0.5);
                }
                let sine = libm::sin(progress * std::f64::consts::FRAC_PI_2);
                sine * sine
            }
            Self::QuadraticIn => progress * progress,
            Self::QuadraticOut => progress * (2.0 - progress),
            Self::QuadraticInOut => mirrored(progress, |value| value * value),
            Self::CubicIn => progress * progress * progress,
            Self::CubicOut => progress * (3.0 + progress * (progress - 3.0)),
            Self::CubicInOut => mirrored(progress, |value| value * value * value),
            Self::QuarticIn => progress * progress * progress * progress,
            Self::QuarticOut => progress * (4.0 + progress * (-6.0 + progress * (4.0 - progress))),
            Self::QuarticInOut => mirrored(progress, |value| value * value * value * value),
            Self::ExponentialIn => libm::exp2(10.0 * (progress - 1.0)),
            Self::ExponentialOut => -libm::expm1(-10.0 * std::f64::consts::LN_2 * progress),
            Self::ExponentialInOut => mirrored(progress, |value| libm::exp2(10.0 * (value - 1.0))),
            Self::BackIn { overshoot } => back_in(progress, *overshoot),
            Self::BackOut { overshoot } => 1.0 - back_in(1.0 - progress, *overshoot),
            Self::BackInOut { overshoot } => mirrored(progress, |value| back_in(value, *overshoot)),
            Self::ElasticOut { amplitude, period } => {
                let phase = libm::asin(1.0 / amplitude) / std::f64::consts::TAU;
                let cycles = libm::fmod(progress, *period) / period;
                1.0 + amplitude
                    * libm::exp2(-10.0 * progress)
                    * libm::sin(std::f64::consts::TAU * (cycles - phase))
            }
            Self::BounceOut => bounce_out(progress),
            Self::Steps(count) => libm::floor(progress * f64::from(*count)) / f64::from(*count),
            Self::CubicBezier { x1, y1, x2, y2 } => cubic_bezier(progress, *x1, *y1, *x2, *y2),
            Self::Custom(function) => function(progress),
        };
        if !value.is_finite() {
            return Err(easing_error("output", "easing output must be finite"));
        }
        Ok(value)
    }

    /// Converts an inward curve to its outward counterpart: `1 - f(1 - t)`.
    /// Invalid built-in parameters panic before the wrapper is constructed.
    pub fn reversed(self) -> Self {
        self.validate();
        Self::Custom(Arc::new(move |progress| 1.0 - self.apply(1.0 - progress)))
    }

    /// Uses the curve for acceleration and its reversal for deceleration.
    /// Invalid built-in parameters panic before the wrapper is constructed.
    pub fn mirrored(self) -> Self {
        self.validate();
        Self::Custom(Arc::new(move |progress| {
            mirrored(progress, |value| self.apply(value))
        }))
    }

    fn validate(&self) {
        self.try_validate()
            .unwrap_or_else(|error| panic!("{}", error.message));
    }

    /// Checks built-in parameters without evaluating a custom callback.
    pub fn try_validate(&self) -> Result<(), crate::ValidationError> {
        match self {
            Self::BackIn { overshoot }
            | Self::BackOut { overshoot }
            | Self::BackInOut { overshoot } => {
                if !overshoot.is_finite() || *overshoot < 0.0 {
                    return Err(easing_error(
                        "overshoot",
                        "back overshoot must be finite and nonnegative",
                    ));
                }
            }
            Self::ElasticOut { amplitude, period } => {
                if !amplitude.is_finite()
                    || *amplitude < 1.0
                    || !period.is_finite()
                    || *period <= 0.0
                {
                    return Err(easing_error(
                        "elastic",
                        "elastic amplitude must be finite and at least one; period must be finite and positive",
                    ));
                }
            }
            Self::Steps(0) => return Err(easing_error("steps", "easing steps must be nonzero")),
            Self::CubicBezier { x1, y1, x2, y2 }
                if (![x1, y1, x2, y2].iter().all(|value| value.is_finite())
                    || !(0.0..=1.0).contains(x1)
                    || !(0.0..=1.0).contains(x2)) =>
            {
                return Err(easing_error(
                    "controls",
                    "Bezier controls must be finite with X in 0..=1",
                ));
            }
            _ => {}
        }
        Ok(())
    }

    pub(super) fn validated(self) -> Self {
        self.validate();
        self
    }
}

impl fmt::Debug for Easing {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Linear => formatter.write_str("Linear"),
            Self::SineIn => formatter.write_str("SineIn"),
            Self::SineOut => formatter.write_str("SineOut"),
            Self::SineInOut => formatter.write_str("SineInOut"),
            Self::QuadraticIn => formatter.write_str("QuadraticIn"),
            Self::QuadraticOut => formatter.write_str("QuadraticOut"),
            Self::QuadraticInOut => formatter.write_str("QuadraticInOut"),
            Self::CubicIn => formatter.write_str("CubicIn"),
            Self::CubicOut => formatter.write_str("CubicOut"),
            Self::CubicInOut => formatter.write_str("CubicInOut"),
            Self::QuarticIn => formatter.write_str("QuarticIn"),
            Self::QuarticOut => formatter.write_str("QuarticOut"),
            Self::QuarticInOut => formatter.write_str("QuarticInOut"),
            Self::ExponentialIn => formatter.write_str("ExponentialIn"),
            Self::ExponentialOut => formatter.write_str("ExponentialOut"),
            Self::ExponentialInOut => formatter.write_str("ExponentialInOut"),
            Self::BackIn { overshoot } => formatter
                .debug_struct("BackIn")
                .field("overshoot", overshoot)
                .finish(),
            Self::BackOut { overshoot } => formatter
                .debug_struct("BackOut")
                .field("overshoot", overshoot)
                .finish(),
            Self::BackInOut { overshoot } => formatter
                .debug_struct("BackInOut")
                .field("overshoot", overshoot)
                .finish(),
            Self::ElasticOut { amplitude, period } => formatter
                .debug_struct("ElasticOut")
                .field("amplitude", amplitude)
                .field("period", period)
                .finish(),
            Self::BounceOut => formatter.write_str("BounceOut"),
            Self::Steps(count) => formatter.debug_tuple("Steps").field(count).finish(),
            Self::CubicBezier { x1, y1, x2, y2 } => formatter
                .debug_struct("CubicBezier")
                .field("x1", x1)
                .field("y1", y1)
                .field("x2", x2)
                .field("y2", y2)
                .finish(),
            Self::Custom(_) => formatter.write_str("Custom(..)"),
        }
    }
}

fn easing_error(field: &str, message: &str) -> crate::ValidationError {
    crate::ValidationError::invalid("easing", field, message)
}

fn mirrored(progress: f64, inward: impl Fn(f64) -> f64) -> f64 {
    if progress < 0.5 {
        inward(2.0 * progress) * 0.5
    } else {
        1.0 - inward(2.0 * (1.0 - progress)) * 0.5
    }
}

fn back_in(progress: f64, overshoot: f64) -> f64 {
    progress * progress * (progress + overshoot * (progress - 1.0))
}

fn bounce_out(progress: f64) -> f64 {
    let (shifted, offset) = if progress < 1.0 / 2.75 {
        (progress, 0.0)
    } else if progress < 2.0 / 2.75 {
        (progress - 1.5 / 2.75, 0.75)
    } else if progress < 2.5 / 2.75 {
        (progress - 2.25 / 2.75, 0.9375)
    } else {
        (progress - 2.625 / 2.75, 0.984375)
    };
    7.5625 * shifted * shifted + offset
}

fn coordinate(parameter: f64, first: f64, second: f64) -> f64 {
    let inverse = 1.0 - parameter;
    let first_weight = 3.0 * inverse * inverse * parameter;
    let second_weight = 3.0 * inverse * parameter * parameter;
    first_weight * first + second_weight * second + parameter * parameter * parameter
}

fn cubic_bezier(progress: f64, x1: f64, y1: f64, x2: f64, y2: f64) -> f64 {
    coordinate(bezier_parameter(progress, x1, x2), y1, y2)
}

fn bezier_parameter(progress: f64, x1: f64, x2: f64) -> f64 {
    if progress > 0.5 {
        return 1.0 - bezier_parameter(1.0 - progress, 1.0 - x2, 1.0 - x1);
    }
    let mut lower = 0.0;
    let mut upper = 1.0;
    for _ in 0..1_100 {
        let parameter = lower + (upper - lower) * 0.5;
        let horizontal = coordinate(parameter, x1, x2);
        if horizontal == progress || parameter == lower || parameter == upper {
            return parameter;
        }
        if horizontal < progress {
            lower = parameter;
        } else {
            upper = parameter;
        }
    }
    lower + (upper - lower) * 0.5
}
