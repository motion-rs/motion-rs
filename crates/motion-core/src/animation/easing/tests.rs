//! Unit tests for easing curves and their boundary behavior.

use super::*;

#[test]
fn checked_animation_inputs_report_errors_without_changing_samples() {
    use crate::{Signal, TimeLiteral, Tween};
    assert!(Easing::steps(0).is_err());
    let movement = Tween::new(0.0, 100.0, 2.seconds()).easing(Easing::CubicOut);
    assert_eq!(movement.sample(1.seconds().after_origin()), 87.5);
    assert!(Easing::Linear.try_apply(f64::NAN).is_err());
    let invalid_output = Easing::Custom(Arc::new(|_| f64::NAN));
    assert!(invalid_output.try_apply(0.0).is_err());
    assert!(invalid_output.try_apply(1.0).is_err());
    assert!(invalid_output.try_apply(0.5).is_err());
}

fn close(actual: f64, expected: f64) {
    assert!(actual.is_finite() && expected.is_finite());
    let tolerance = 2e-12 * expected.abs().max(f64::MIN_POSITIVE);
    assert!(
        (actual - expected).abs() <= tolerance,
        "{actual} != {expected}"
    );
}

fn bounded_curves() -> Vec<Easing> {
    vec![
        Easing::Linear,
        Easing::SineIn,
        Easing::SineOut,
        Easing::SineInOut,
        Easing::QuadraticIn,
        Easing::QuadraticOut,
        Easing::QuadraticInOut,
        Easing::CubicIn,
        Easing::CubicOut,
        Easing::CubicInOut,
        Easing::QuarticIn,
        Easing::QuarticOut,
        Easing::QuarticInOut,
        Easing::ExponentialIn,
        Easing::ExponentialOut,
        Easing::ExponentialInOut,
        Easing::Steps(7),
        Easing::CubicBezier {
            x1: 0.3,
            y1: 0.1,
            x2: 0.7,
            y2: 0.8,
        },
    ]
}

#[test]
fn bounded_curves_are_monotonic_and_builtins_pin_endpoints() {
    let bounded = bounded_curves();
    for curve in &bounded {
        let mut previous = 0.0;
        for index in 0..=1_000 {
            let value = curve.apply(f64::from(index) / 1_000.0);
            assert!((0.0..=1.0).contains(&value), "{curve:?}: {value}");
            assert!(value >= previous, "{curve:?}: {value} < {previous}");
            previous = value;
        }
    }
    let mut all = bounded;
    all.extend([
        Easing::BackIn { overshoot: 1.70158 },
        Easing::BackOut { overshoot: 1.70158 },
        Easing::BackInOut { overshoot: 1.70158 },
        Easing::ElasticOut {
            amplitude: 1.0,
            period: 0.3,
        },
        Easing::BounceOut,
    ]);
    assert_eq!(all.len(), 23);
    for curve in all {
        for progress in [-100.0, -0.0, 0.0] {
            assert_eq!(curve.apply(progress), 0.0);
        }
        for progress in [1.0, 100.0] {
            assert_eq!(curve.apply(progress), 1.0);
        }
    }
}

#[test]
fn custom_curves_preserve_endpoint_outputs_and_clamp_only_input() {
    let reverse = Easing::Custom(Arc::new(|progress| 1.0 - progress));
    for progress in [-4.0, 0.0, 0.25, 1.0, 4.0] {
        assert_eq!(reverse.apply(progress), 1.0 - progress.clamp(0.0, 1.0));
    }
    let extended = Easing::Custom(Arc::new(|progress| 2.0 * progress - 0.5));
    assert_eq!(extended.apply(0.0), -0.5);
    assert_eq!(extended.apply(1.0), 1.5);
    assert_eq!(reverse.reversed().apply(0.0), 1.0);
}

#[test]
fn analytic_samples_preserve_small_progress_steps_and_overshoot() {
    close(
        Easing::SineIn.apply(0.5),
        1.0 - std::f64::consts::FRAC_1_SQRT_2,
    );
    close(Easing::SineOut.apply(0.5), std::f64::consts::FRAC_1_SQRT_2);
    assert_eq!(Easing::SineInOut.apply(0.5), 0.5);
    assert_eq!(Easing::QuadraticIn.apply(0.5), 0.25);
    assert_eq!(Easing::CubicIn.apply(0.5), 0.125);
    assert_eq!(Easing::QuarticIn.apply(0.5), 0.0625);
    assert_eq!(Easing::CubicOut.apply(0.5), 0.875);
    assert_eq!(Easing::QuarticOut.apply(0.5), 0.9375);
    assert_eq!(Easing::ExponentialIn.apply(0.5), 1.0 / 32.0);
    close(Easing::ExponentialOut.apply(0.5), 31.0 / 32.0);
    assert_eq!(Easing::ExponentialInOut.apply(0.5), 0.5);
    assert!(Easing::SineIn.apply(1e-20) > 0.0);
    assert!(Easing::ExponentialOut.apply(1e-20) > 0.0);
    assert!(Easing::BackIn { overshoot: 1.7 }.apply(0.25) < 0.0);
    assert!(Easing::BackOut { overshoot: 1.7 }.apply(0.75) > 1.0);
    assert!(
        Easing::ElasticOut {
            amplitude: 1.0,
            period: 0.3
        }
        .apply(0.15)
            > 1.0
    );
    close(Easing::BounceOut.apply(1.5 / 2.75), 0.75);
    assert_eq!(
        Easing::Steps(4).apply(f64::from_bits(0.25_f64.to_bits() - 1)),
        0.0
    );
    assert_eq!(Easing::Steps(4).apply(0.25), 0.25);
    assert_eq!(Easing::Steps(4).apply(0.999), 0.75);
    for progress in [0.1, 0.5, 0.9] {
        assert!(
            Easing::ElasticOut {
                amplitude: 1e300,
                period: 1e-300
            }
            .apply(progress)
            .is_finite()
        );
        assert!(
            Easing::BackInOut {
                overshoot: f64::MAX
            }
            .apply(progress)
            .is_finite()
        );
    }
}

#[test]
fn reversal_and_mirroring_match_the_corresponding_curve_families() {
    for (inward, outward, symmetric) in [
        (Easing::SineIn, Easing::SineOut, Easing::SineInOut),
        (
            Easing::QuadraticIn,
            Easing::QuadraticOut,
            Easing::QuadraticInOut,
        ),
        (Easing::CubicIn, Easing::CubicOut, Easing::CubicInOut),
        (Easing::QuarticIn, Easing::QuarticOut, Easing::QuarticInOut),
        (
            Easing::ExponentialIn,
            Easing::ExponentialOut,
            Easing::ExponentialInOut,
        ),
        (
            Easing::BackIn { overshoot: 1.7 },
            Easing::BackOut { overshoot: 1.7 },
            Easing::BackInOut { overshoot: 1.7 },
        ),
    ] {
        let reversed = inward.clone().reversed();
        let mirrored = inward.mirrored();
        for progress in [0.1, 0.25, 0.5, 0.75, 0.9] {
            close(reversed.apply(progress), outward.apply(progress));
            close(mirrored.apply(progress), symmetric.apply(progress));
        }
    }
}

#[test]
fn bezier_solves_flat_axes_and_preserves_double_precision_controls() {
    let control = 0.500_000_000_1;
    let curve = Easing::CubicBezier {
        x1: 0.0,
        y1: control,
        x2: 1.0,
        y2: control,
    };
    let value = curve.apply(0.5);
    assert_eq!(value, 0.75 * control + 0.125);
    assert_ne!(value, f64::from(value as f32));
    let flat = Easing::CubicBezier {
        x1: 0.0,
        y1: 1.0 / 3.0,
        x2: 0.0,
        y2: 2.0 / 3.0,
    };
    close(flat.apply(0.125), 0.5);
    close(flat.apply(1e-240), 1e-80);
    let flat_end = Easing::CubicBezier {
        x1: 1.0,
        y1: 1.0 / 3.0,
        x2: 1.0,
        y2: 2.0 / 3.0,
    };
    let near_end = 1.0 - f64::EPSILON;
    close(flat_end.apply(near_end), 1.0 - libm::cbrt(1.0 - near_end));
    let overshooting = Easing::CubicBezier {
        x1: 0.3,
        y1: 1.8,
        x2: 0.7,
        y2: 1.8,
    };
    assert!(overshooting.apply(0.5) > 1.0);
    let diagonal = Easing::CubicBezier {
        x1: 0.0,
        y1: 0.0,
        x2: 0.0,
        y2: 0.0,
    };
    for progress in [1e-240, 1e-19, 0.001, 0.5, 1.0 - f64::EPSILON] {
        close(diagonal.apply(progress), progress);
    }
}

#[test]
fn constructors_reject_invalid_parameters() {
    let invalid = [
        Easing::steps(0),
        Easing::back_in(-1.0),
        Easing::back_out(f64::NAN),
        Easing::back_in_out(f64::INFINITY),
        Easing::elastic_out(0.5, 0.3),
        Easing::elastic_out(f64::INFINITY, 0.3),
        Easing::elastic_out(1.0, 0.0),
        Easing::elastic_out(1.0, f64::NAN),
        Easing::cubic_bezier(-0.1, 0.0, 1.0, 1.0),
        Easing::cubic_bezier(0.0, f64::NAN, 1.0, 1.0),
        Easing::cubic_bezier(0.0, 0.0, 1.1, 1.0),
    ];
    for result in invalid {
        assert!(result.is_err());
    }
    assert!(Easing::back_in(1.70158).is_ok());
    assert!(Easing::elastic_out(1.0, 0.3).is_ok());
    assert!(Easing::cubic_bezier(0.3, 1.8, 0.7, -0.5).is_ok());
}

#[test]
fn nonfinite_progress_is_total_for_apply_and_rejected_by_try_apply() {
    for progress in [f64::NAN, f64::NEG_INFINITY, f64::INFINITY] {
        assert!(Easing::Linear.try_apply(progress).is_err());
    }
    assert!(Easing::Linear.apply(f64::NAN).is_nan());
    assert_eq!(Easing::Linear.apply(f64::NEG_INFINITY), 0.0);
    assert_eq!(Easing::CubicOut.apply(f64::INFINITY), 1.0);
}

#[test]
fn custom_curves_clone_across_threads_without_debug_evaluating_them() {
    let curve = Easing::Custom(Arc::new(|progress| progress * progress));
    assert_eq!(format!("{curve:?}"), "Custom(..)");
    let copied = curve.clone();
    assert_eq!(
        std::thread::spawn(move || copied.apply(0.5))
            .join()
            .unwrap(),
        0.25
    );
    assert_eq!(curve.apply(0.25), 0.0625);
    let invalid = Easing::Custom(Arc::new(|_| f64::NAN));
    assert!(invalid.apply(0.5).is_nan());
    assert!(invalid.try_apply(0.5).is_err());
}

#[test]
fn inputs_preserve_double_precision_across_clip_durations() {
    use crate::{Keyframes, Signal, Time, TimeLiteral, Tween};
    for curve in [
        Easing::Linear,
        Easing::QuadraticIn,
        Easing::QuadraticOut,
        Easing::QuadraticInOut,
        Easing::CubicBezier {
            x1: 0.3,
            y1: 1.8,
            x2: 0.7,
            y2: 1.8,
        },
    ] {
        for duration in [1.seconds(), 3.seconds()] {
            let tween = Tween::new(10.0_f64, 20.0, duration).easing(curve.clone());
            let track = Keyframes::new(10.0_f64).to(duration, 20.0, curve.clone());
            for time in [
                Time::from_ticks(i64::MIN),
                Time::ZERO,
                Time::from_ticks(1),
                (duration / 4.0).after_origin(),
                (duration / 2.0).after_origin(),
                duration.after_origin(),
                Time::from_ticks(i64::MAX),
            ] {
                let progress = curve.apply(time.ticks() as f64 / duration.ticks() as f64);
                let expected = 10.0 * (1.0 - progress) + 20.0 * progress;
                assert_eq!(tween.sample(time).to_bits(), expected.to_bits());
                assert_eq!(track.sample(time).to_bits(), expected.to_bits());
            }
        }
    }
}
