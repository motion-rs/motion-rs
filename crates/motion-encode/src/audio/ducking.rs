//! Offline sidechain ducking driven by a peak-hold envelope follower.
//!
//! The trigger is rendered completely before the ducked signal is touched, so the result
//! does not depend on read order. Attack/release smoothing runs on the gain reduction in dB.

use motion_core::Duck;

/// Detector hold decay; keeps zero crossings of the trigger from chattering the gain.
const DETECTOR_RELEASE: f64 = 0.005;

fn coefficient(seconds: f64, rate: u32) -> f64 {
    if seconds <= 0.0 {
        0.0
    } else {
        libm::exp(-1.0 / (seconds * f64::from(rate)))
    }
}

/// Per-frame gain reduction in dB (nonnegative) for an interleaved stereo trigger.
pub(crate) fn reduction_db(trigger: &[f32], rate: u32, duck: &Duck) -> Vec<f32> {
    let frames = trigger.len() / 2;
    let hold = coefficient(DETECTOR_RELEASE, rate);
    let slope = 1.0 - 1.0 / duck.ratio;
    let mut detector = 0.0f64;
    let mut target = Vec::with_capacity(frames);
    for pair in trigger.as_chunks::<2>().0 {
        let level = f64::from(pair[0].abs().max(pair[1].abs()));
        detector = level.max(detector * hold);
        let over = 20.0 * libm::log10(detector.max(1e-12)) - duck.threshold_db;
        target.push((over.max(0.0) * slope).min(duck.depth_db) as f32);
    }
    let attack = coefficient(duck.attack.as_seconds(), rate);
    let release = coefficient(duck.release.as_seconds(), rate);
    let ahead = (duck.lookahead.as_seconds() * f64::from(rate)).round() as usize;
    let mut current = 0.0f64;
    for frame in 0..frames {
        let wanted = f64::from(target.get(frame + ahead).copied().unwrap_or(0.0));
        let blend = if wanted > current { attack } else { release };
        current = wanted + (current - wanted) * blend;
        target[frame] = current as f32;
    }
    target
}

/// Multiplies stereo frames starting at `start` by the matching reduction entries.
pub(crate) fn apply(data: &mut [f32], start: usize, reduction: &[f32]) {
    for (index, pair) in data.as_chunks_mut::<2>().0.iter_mut().enumerate() {
        let Some(db) = reduction.get(start + index).copied().filter(|db| *db > 0.0) else {
            continue;
        };
        let gain = libm::exp(-f64::from(db) * std::f64::consts::LN_10 / 20.0) as f32;
        pair[0] *= gain;
        pair[1] *= gain;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use motion_core::TimeLiteral;

    fn burst(rate: u32, seconds: f64, from: f64, to: f64, level: f32) -> Vec<f32> {
        (0..(seconds * f64::from(rate)) as usize)
            .flat_map(|frame| {
                let time = frame as f64 / f64::from(rate);
                let value = if (from..to).contains(&time) {
                    level * libm::sin(2.0 * std::f64::consts::PI * 440.0 * time) as f32
                } else {
                    0.0
                };
                [value, value]
            })
            .collect()
    }

    #[test]
    fn follower_attacks_holds_and_releases() {
        let rate = 8000;
        let duck = Duck::new(-40.0, 12.0)
            .timing(20.milliseconds(), 100.milliseconds())
            .lookahead(0.milliseconds());
        let curve = reduction_db(&burst(rate, 2.0, 0.5, 1.0, 0.5), rate, &duck);
        assert_eq!(curve[(0.25 * f64::from(rate)) as usize], 0.0);
        assert!(curve[(0.9 * f64::from(rate)) as usize] > 11.9);
        assert!(curve[(0.9 * f64::from(rate)) as usize] <= 12.0);
        assert!(curve[(1.9 * f64::from(rate)) as usize] < 0.01);
        assert!(curve.iter().all(|db| (0.0..=12.0).contains(db)));
    }

    #[test]
    fn lookahead_anticipates_and_ratio_limits_depth() {
        let rate = 8000;
        let trigger = burst(rate, 1.0, 0.5, 1.0, 0.5);
        let soft = Duck::new(-40.0, 40.0)
            .ratio(2.0)
            .timing(0.milliseconds(), 0.milliseconds())
            .lookahead(0.milliseconds());
        let early = Duck {
            lookahead: 100.milliseconds(),
            ..soft
        };
        let plain = reduction_db(&trigger, rate, &soft);
        let ahead = reduction_db(&trigger, rate, &early);
        let before = (0.45 * f64::from(rate)) as usize;
        assert_eq!(plain[before], 0.0);
        assert!(ahead[before] > 5.0);
        assert!(plain[(0.9 * f64::from(rate)) as usize] < 20.0);
        assert_eq!(plain, reduction_db(&trigger, rate, &soft));
    }

    #[test]
    fn apply_scales_frames_by_the_reduction() {
        let mut data = vec![1.0; 4];
        apply(&mut data, 1, &[0.0, 20.0, 6.0]);
        assert!((data[0] - 0.1).abs() < 1e-6 && (data[1] - 0.1).abs() < 1e-6);
        assert!((data[2] - 0.501_187).abs() < 1e-4);
    }
}
