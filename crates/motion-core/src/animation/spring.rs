//! Closed-form damped spring response and conservative tick-based settling time.

use super::Signal;
use crate::{Duration, Interpolate, TICKS_PER_SECOND};

/// A finite physical spring between arbitrary interpolatable values.
///
/// Initial velocity is in endpoint distances per second. The default mass is one,
/// stiffness 100, damping 10 and initial velocity zero. Duration is the conservative
/// displacement-envelope settling time at threshold 0.001, rounded up to ticks.
/// Values at/before zero and at/after duration are exact stored endpoints.
///
/// Builders validate the complete candidate configuration immediately. Nonfinite
/// or nonpositive physical parameters, an unrepresentable response or settling
/// time, and invalid thresholds panic. Zero damping is rejected because this type
/// promises a finite settling duration. Use a signal for undamped oscillation.
///
/// ```
/// use motion_core::prelude::*;
/// use motion_core::{
///     DisplayList, RasterLimits, ResourcePool, ValidatedDisplayList, MAX_PATH_VERBS,
///     MAX_STROKE_VERBS, MAX_SVG_PATH_BYTES, sample_offsets,
/// };
/// let movement = Spring::new(Point::ORIGIN, Point::new(100.0, 40.0))
///     .damping(20.0)
///     .settling_threshold(0.0001);
/// let end = movement.settle_duration();
/// assert_eq!(movement.sample(end), Point::new(100.0, 40.0));
/// assert_eq!(movement.sample((-1).seconds()), Point::ORIGIN);
/// ```
#[derive(Clone, Debug)]
pub struct Spring<Value> {
    from: Value,
    to: Value,
    parameters: Parameters,
    response: Response,
}

impl<Value> Spring<Value> {
    /// Creates a spring with the default physical parameters and settling threshold.
    pub fn new(from: Value, to: Value) -> Self {
        let parameters = Parameters::default();
        Self {
            from,
            to,
            parameters,
            response: Response::new(parameters),
        }
    }

    /// Sets finite positive mass and recomputes the finite response immediately.
    pub fn mass(mut self, mass: f64) -> Self {
        self.parameters.mass = mass;
        self.rebuild()
    }

    /// Sets finite positive stiffness and recomputes the response immediately.
    pub fn stiffness(mut self, stiffness: f64) -> Self {
        self.parameters.stiffness = stiffness;
        self.rebuild()
    }

    /// Sets finite strictly positive damping. Zero has no finite settling time.
    pub fn damping(mut self, damping: f64) -> Self {
        self.parameters.damping = damping;
        self.rebuild()
    }

    /// Sets finite normalized velocity towards the target; negative moves away.
    /// Units are endpoint distances per second, not pixels per second.
    pub fn initial_velocity(mut self, velocity: f64) -> Self {
        self.parameters.velocity = velocity;
        self.rebuild()
    }

    /// Sets a finite relative displacement-envelope threshold strictly between
    /// zero and one. This is not a pixel tolerance or a velocity threshold.
    pub fn settling_threshold(mut self, threshold: f64) -> Self {
        self.parameters.threshold = threshold;
        self.rebuild()
    }

    /// Returns the conservative time at which the spring reaches its target exactly.
    pub fn settle_duration(&self) -> Duration {
        self.response.duration
    }

    fn rebuild(mut self) -> Self {
        self.response = Response::new(self.parameters);
        self
    }
}

impl Default for Spring<f64> {
    fn default() -> Self {
        Self::new(0.0, 1.0)
    }
}

impl<Value: Interpolate + Send + Sync> Signal<Value> for Spring<Value> {
    fn sample(&self, time: Duration) -> Value {
        if time <= Duration::ZERO {
            return self.from.clone();
        }
        if time >= self.response.duration {
            return self.to.clone();
        }
        self.from
            .interpolate(&self.to, self.response.progress(time.as_seconds()))
    }
}

#[derive(Clone, Copy, Debug)]
struct Parameters {
    mass: f64,
    stiffness: f64,
    damping: f64,
    velocity: f64,
    threshold: f64,
}

impl Default for Parameters {
    fn default() -> Self {
        Self {
            mass: 1.0,
            stiffness: 100.0,
            damping: 10.0,
            velocity: 0.0,
            threshold: 0.001,
        }
    }
}

#[derive(Clone, Copy, Debug)]
enum Regime {
    Under { oscillation: f64 },
    Critical,
    Over { separation: f64 },
}

#[derive(Clone, Copy, Debug)]
struct Response {
    regime: Regime,
    rate: f64,
    velocity: f64,
    duration: Duration,
}

impl Response {
    fn new(parameters: Parameters) -> Self {
        assert!(
            parameters.mass.is_finite() && parameters.mass > 0.0,
            "spring mass must be finite and positive"
        );
        assert!(
            parameters.stiffness.is_finite() && parameters.stiffness > 0.0,
            "spring stiffness must be finite and positive"
        );
        assert!(
            parameters.damping.is_finite() && parameters.damping > 0.0,
            "finite spring damping must be finite and positive"
        );
        assert!(
            parameters.velocity.is_finite(),
            "spring velocity must be finite"
        );
        assert!(
            parameters.threshold.is_finite()
                && parameters.threshold > 0.0
                && parameters.threshold < 1.0,
            "spring settling threshold must be finite and between zero and one"
        );
        let frequency = libm::sqrt(parameters.stiffness) / libm::sqrt(parameters.mass);
        let quotient = parameters.damping / parameters.mass;
        let decay = if quotient.is_finite() {
            quotient * 0.5
        } else {
            (parameters.damping * 0.5) / parameters.mass
        };
        assert!(
            frequency.is_finite() && frequency > 0.0 && decay.is_finite() && decay > 0.0,
            "spring derived rates must be finite and positive"
        );
        let (regime, rate) = if decay < frequency {
            let oscillation =
                frequency * libm::sqrt((frequency - decay) / frequency * (1.0 + decay / frequency));
            assert!(
                oscillation.is_finite() && oscillation > 0.0,
                "spring oscillation must be representable"
            );
            (Regime::Under { oscillation }, decay)
        } else if decay == frequency {
            (Regime::Critical, decay)
        } else {
            let root = decay * libm::sqrt((decay - frequency) / decay * (1.0 + frequency / decay));
            let separation = 2.0 * root;
            let slow = (frequency / decay) * frequency / (1.0 + root / decay);
            assert!(
                separation.is_finite() && separation > 0.0 && slow.is_finite() && slow > 0.0,
                "spring characteristic rates must be representable"
            );
            (Regime::Over { separation }, slow)
        };
        let velocity = parameters.velocity / rate;
        assert!(
            velocity.is_finite(),
            "spring normalized velocity must be representable"
        );
        let duration = settling_duration(rate, (1.0 - velocity).abs(), parameters.threshold);
        if let Regime::Under { oscillation } = regime {
            assert!(
                (oscillation * duration.as_seconds()).is_finite(),
                "spring phase must be representable"
            );
        }
        Self {
            regime,
            rate,
            velocity,
            duration,
        }
    }

    fn progress(self, seconds: f64) -> f64 {
        let elapsed = self.rate * seconds;
        if elapsed.is_infinite() {
            return 1.0;
        }
        let decay = libm::exp(-elapsed);
        let weighted_time = elapsed * decay;
        let base = critical_progress(elapsed, decay);
        let progress = match self.regime {
            Regime::Critical => base + self.velocity * weighted_time,
            Regime::Under { oscillation } => {
                let angle = oscillation * seconds;
                let (sinc, remainder) = sine_ratio(angle);
                let half_sine = libm::sin(angle * 0.5);
                base + weighted_time * remainder
                    + decay * (2.0 * half_sine * half_sine)
                    + self.velocity * weighted_time * sinc
            }
            Regime::Over { separation } => {
                let (ratio, remainder) = exponential_ratio(separation * seconds);
                base + weighted_time * remainder + self.velocity * weighted_time * ratio
            }
        };
        assert!(progress.is_finite(), "spring response must be finite");
        progress
    }
}

fn critical_progress(elapsed: f64, decay: f64) -> f64 {
    if elapsed < 0.001 {
        elapsed
            * elapsed
            * (0.5
                + elapsed
                    * (-1.0 / 3.0 + elapsed * (0.125 + elapsed * (-1.0 / 30.0 + elapsed / 144.0))))
    } else {
        -libm::expm1(-elapsed) - elapsed * decay
    }
}

fn sine_ratio(angle: f64) -> (f64, f64) {
    if angle.abs() < 0.001 {
        let square = angle * angle;
        let remainder = square * (1.0 / 6.0 + square * (-1.0 / 120.0 + square / 5_040.0));
        (1.0 - remainder, remainder)
    } else {
        let ratio = libm::sin(angle) / angle;
        (ratio, 1.0 - ratio)
    }
}

fn exponential_ratio(elapsed: f64) -> (f64, f64) {
    if elapsed.is_infinite() {
        (0.0, 1.0)
    } else if elapsed < 0.001 {
        let remainder = elapsed
            * (0.5
                + elapsed
                    * (-1.0 / 6.0
                        + elapsed * (1.0 / 24.0 + elapsed * (-1.0 / 120.0 + elapsed / 720.0))));
        (1.0 - remainder, remainder)
    } else {
        let ratio = -libm::expm1(-elapsed) / elapsed;
        (ratio, 1.0 - ratio)
    }
}

fn log_envelope(elapsed: f64, slope: f64) -> f64 {
    if elapsed == 0.0 {
        return 0.0;
    }
    if slope == 0.0 {
        return -elapsed;
    }
    let product_log = libm::log(slope) + libm::log(elapsed);
    let envelope_log = if product_log > 0.0 {
        product_log + libm::log1p(libm::exp(-product_log))
    } else {
        libm::log1p(libm::exp(product_log))
    };
    envelope_log - elapsed
}

fn settling_duration(rate: f64, slope: f64, threshold: f64) -> Duration {
    let target = libm::log(threshold);
    let mut lower = 0.0;
    let mut upper = 1.0;
    while log_envelope(upper, slope) > target {
        upper *= 2.0;
    }
    for _ in 0..80 {
        let middle = lower + (upper - lower) * 0.5;
        if middle == lower || middle == upper {
            break;
        }
        if log_envelope(middle, slope) > target {
            lower = middle;
        } else {
            upper = middle;
        }
    }
    let seconds = (upper * (1.0 + 32.0 * f64::EPSILON)) / rate;
    let ticks = libm::ceil(seconds * TICKS_PER_SECOND as f64).max(1.0);
    assert!(
        ticks.is_finite() && ticks < -(i64::MIN as f64),
        "spring settling duration exceeds the signed tick range"
    );
    Duration::from_ticks(ticks as i64)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{Spring, TimeLiteral};

    fn close(actual: f64, expected: f64) {
        assert!(actual.is_finite() && expected.is_finite());
        assert!(
            (actual - expected).abs() <= 2e-12 * expected.abs().max(1e-12),
            "{actual} != {expected}"
        );
    }

    #[test]
    fn all_damping_regimes_match_analytic_references_and_retain_first_tick_motion() {
        for damping in [10.0, 20.0, 40.0] {
            let response = Response::new(Parameters {
                damping,
                ..Parameters::default()
            });
            let decay = damping * 0.5;
            for seconds in [0.01, 0.1, 0.25, 0.5] {
                let expected = if damping < 20.0 {
                    let oscillation = libm::sqrt(100.0 - decay * decay);
                    1.0 - libm::exp(-decay * seconds)
                        * (libm::cos(oscillation * seconds)
                            + decay / oscillation * libm::sin(oscillation * seconds))
                } else if damping == 20.0 {
                    1.0 - libm::exp(-decay * seconds) * (1.0 + decay * seconds)
                } else {
                    let root = libm::sqrt(decay * decay - 100.0);
                    1.0 - libm::exp(-decay * seconds)
                        * (libm::cosh(root * seconds) + decay / root * libm::sinh(root * seconds))
                };
                close(response.progress(seconds), expected);
            }
            let tick = 1.0 / TICKS_PER_SECOND as f64;
            let expected = 50.0 * tick * tick;
            assert!((response.progress(tick) / expected - 1.0).abs() < 1e-6);
        }
    }

    #[test]
    fn settling_envelope_stays_below_threshold_across_regimes_and_large_velocities() {
        for damping in [0.1, 10.0, 20.0 - 1e-8, 20.0, 20.0 + 1e-8, 40.0, 1e8] {
            for velocity in [-4.0, 0.0, 20.0, 1e300] {
                let parameters = Parameters {
                    damping,
                    velocity,
                    ..Parameters::default()
                };
                let response = Response::new(parameters);
                assert!(response.duration > Duration::ZERO);
                for factor in [1.0, 1.1, 2.0, 10.0] {
                    let seconds = response.duration.as_seconds() * factor;
                    let error = (1.0 - response.progress(seconds)).abs();
                    assert!(
                        error <= parameters.threshold * (1.0 + 1e-10),
                        "damping={damping}, velocity={velocity}, error={error}"
                    );
                    assert!(
                        log_envelope(response.rate * seconds, (1.0 - response.velocity).abs())
                            <= libm::log(parameters.threshold) + 1e-12
                    );
                }
            }
        }
    }

    #[test]
    fn initial_velocity_and_mass_scaling_have_physical_units() {
        let tick = Duration::from_ticks(1);
        for velocity in [-3.0, 4.0] {
            let spring = Spring::new(0.0_f64, 1.0).initial_velocity(velocity);
            assert!((spring.sample(tick) / tick.as_seconds() - velocity).abs() < 1e-6);
        }
        let base = Spring::new(0.0_f64, 1.0);
        let scaled = Spring::new(0.0_f64, 1.0)
            .mass(2.0)
            .stiffness(200.0)
            .damping(20.0);
        for time in [100.milliseconds(), 300.milliseconds(), 500.milliseconds()] {
            close(base.sample(time), scaled.sample(time));
        }
        assert!((base.settle_duration().ticks() - scaled.settle_duration().ticks()).abs() <= 1);
    }

    #[test]
    fn near_critical_responses_and_durations_converge_without_coefficient_blowup() {
        let critical = Spring::new(0.0_f64, 1.0).damping(20.0);
        for damping in [20.0 - 1e-10, 20.0 + 1e-10] {
            let nearby = Spring::new(0.0_f64, 1.0).damping(damping);
            for time in [10.milliseconds(), 100.milliseconds(), 500.milliseconds()] {
                assert!((nearby.sample(time) - critical.sample(time)).abs() < 1e-11);
            }
            assert!(
                (nearby.settle_duration().as_seconds() / critical.settle_duration().as_seconds()
                    - 1.0)
                    .abs()
                    < 1e-4
            );
        }
    }

    #[test]
    fn default_threshold_and_random_order_sampling_pin_exact_endpoints() {
        let spring = Spring::default();
        let end = spring.settle_duration();
        assert!(end > Duration::ZERO);
        assert_eq!(spring.sample(Duration::from_ticks(i64::MIN)), 0.0);
        assert_eq!(spring.sample(Duration::ZERO), 0.0);
        assert_eq!(spring.sample(end), 1.0);
        assert_eq!(spring.sample(Duration::from_ticks(i64::MAX)), 1.0);
        let expected = spring.sample(300.milliseconds());
        assert!(spring.sample(500.milliseconds()) > 1.0);
        for time in [
            end,
            Duration::ZERO,
            300.milliseconds(),
            100.milliseconds(),
            300.milliseconds(),
        ] {
            assert_eq!(spring.sample(time), spring.clone().sample(time));
            if time == 300.milliseconds() {
                assert_eq!(spring.sample(time), expected);
            }
        }
        let tighter = spring.clone().settling_threshold(1e-6);
        assert!(tighter.settle_duration() > end);
        let smallest = spring.settling_threshold(f64::from_bits(1));
        let response = smallest.response;
        assert!(
            log_envelope(
                response.rate * response.duration.as_seconds(),
                (1.0 - response.velocity).abs()
            ) <= libm::log(f64::from_bits(1))
        );
    }

    #[test]
    fn invalid_physical_parameters_and_unrepresentable_durations_fail_while_building() {
        for value in [f64::NAN, f64::INFINITY, -1.0, 0.0] {
            assert!(std::panic::catch_unwind(|| Spring::default().mass(value)).is_err());
            assert!(std::panic::catch_unwind(|| Spring::default().stiffness(value)).is_err());
            assert!(std::panic::catch_unwind(|| Spring::default().damping(value)).is_err());
        }
        for value in [f64::NAN, f64::INFINITY, f64::NEG_INFINITY] {
            assert!(
                std::panic::catch_unwind(|| Spring::default().initial_velocity(value)).is_err()
            );
        }
        for value in [0.0, 1.0, -1.0, f64::NAN, f64::INFINITY] {
            assert!(
                std::panic::catch_unwind(|| Spring::default().settling_threshold(value)).is_err()
            );
        }
        assert!(std::panic::catch_unwind(|| Spring::default().damping(1e-15)).is_err());
        assert!(std::panic::catch_unwind(|| Spring::default().stiffness(1e-30)).is_err());
        assert!(std::panic::catch_unwind(|| Spring::default().mass(f64::from_bits(1))).is_err());
    }
}
