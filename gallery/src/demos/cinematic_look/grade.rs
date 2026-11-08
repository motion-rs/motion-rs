//! Final grade for Last Order: the quantized teal/orange strength curve and the
//! log-encoded `Lut3d` that applies the CDL and colour wheels before the AgX tone map.

use super::{GRADE_STEPS, LOG_MAX, LOG_MIN, LUT_EDGE};
use motion::prelude::*;
use std::sync::Arc;

/// Strength of the teal/orange grade: it rises over the sixth beat and stays.
pub(super) fn grade_strength(t: f64) -> f64 {
    ((math::smoothstep(6.0, 6.9, t) * GRADE_STEPS).round() / GRADE_STEPS).clamp(0.0, 1.0)
}

/// One lattice that grades and then tone maps, log-encoded so the dark night keeps
/// resolution while highlights up to 32x white still reach the shoulder.
pub(super) fn look_lut(context: &SceneContext<'_>, grade: f64) -> motion::Result<Arc<Lut3d>> {
    let key = (grade * GRADE_STEPS) as u32;
    let lut = context.cache(("look-lut", key), || bake_look(grade as f32));
    match lut.as_ref() {
        Ok(lut) => Ok(Arc::new(lut.clone())),
        Err(error) => Err(std::io::Error::other(error.to_string()).into()),
    }
}

pub(super) fn bake_look(grade: f32) -> Result<Lut3d, ValidationError> {
    let display = DisplayTransform {
        tone_map: ToneMap::AgX,
        ..DisplayTransform::default()
    };
    let cdl = Cdl {
        slope: [1.0 + 0.08 * grade, 1.0, 1.0 - 0.1 * grade],
        power: [1.0, 1.0, 1.0 - 0.04 * grade],
        saturation: 1.0 + 0.18 * grade,
        ..Cdl::default()
    };
    let wheels = ColorWheels {
        shadows: [-0.006, 0.004, 0.012].map(|value| value * grade),
        midtones: [0.02, 0.0, -0.025].map(|value| value * grade),
        highlights: [0.09, 0.035, -0.08].map(|value| value * grade),
    };
    let span = LOG_MAX - LOG_MIN;
    let edge = usize::from(LUT_EDGE);
    let stop = |index: usize| index as f32 / (edge - 1) as f32 * span + LOG_MIN;
    let mut data = Vec::with_capacity(edge * edge * edge);
    for blue in 0..edge {
        for green in 0..edge {
            for red in 0..edge {
                let linear = [red, green, blue].map(|index| stop(index).exp2());
                let graded = wheels.apply(cdl.apply(linear)).map(|value| value.max(0.0));
                data.push(display.apply(graded).map(|value| {
                    ((value.max(LOG_MIN.exp2()).log2() - LOG_MIN) / span).clamp(0.0, 1.0)
                }));
            }
        }
    }
    Lut3d::new(LUT_EDGE, [0.0; 3], [1.0; 3], data)
}
