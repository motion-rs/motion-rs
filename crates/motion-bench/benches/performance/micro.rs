//! Primitive microbenchmarks for the performance target, plus the PPM file output
//! reference.

use super::artwork::artwork;
use super::harness::ppm;
use super::support;
use motion::prelude::*;
use motion::render::Composition;
use std::hint::black_box;

pub(super) fn microbenchmarks(
    report: &mut support::Report,
    settings: &support::Settings,
) -> motion::Result {
    use serde_json::json;
    let mut index = 0_i64;
    let easing = Easing::cubic_bezier(0.2, 0.0, 0.8, 1.0)?;
    let bezier = Tween::new(0.0_f64, 1.0, 1.seconds()).easing(easing);
    let spring = Spring::default();
    let tracks: [(&str, &dyn Signal<f64>); 2] =
        [("animation/bezier", &bezier), ("animation/spring", &spring)];
    for (name, track) in tracks {
        let measurement = support::measure(
            || {
                index += 1;
                Ok(track
                    .sample((index % 1000).milliseconds().after_origin())
                    .to_bits())
            },
            settings,
            20_000,
        )?;
        report.record(name, measurement, json!({}))?;
    }
    let rig = Skeleton::new(std::array::from_fn::<_, 64, _>(|index| {
        Bone::new(index.checked_sub(1), (2.0, 1.0))
    }))?;
    let pose = Pose::new(
        [BonePose {
            rotation: 0.02,
            ..BonePose::default()
        }; 64],
    );
    let measured = support::measure(
        || Ok(rig.solve(black_box(&pose))?[63].position().0.to_bits()),
        settings,
        2_000,
    )?;
    report.record("skeleton/solve64", measured, json!({}))?;
    let track = (1..64).fold(Keyframes::new(0.0_f64), |track, index| {
        track.to((index * 10).milliseconds(), index as f64, Easing::Linear)
    });
    let measured = support::measure(
        || {
            index += 1;
            Ok(track
                .sample((index % 630).milliseconds().after_origin())
                .to_bits())
        },
        settings,
        20_000,
    )?;
    report.record("animation/keyframes64", measured, json!({}))?;
    let snapshot = Composition::new(
        640,
        360,
        motion::render::FrameClock::new(motion::fps!(30), 150),
    )
    .render_frame(0, Size::new(640.0, 360.0), |context| {
        artwork("shapes", context)
    })?;
    report.reference("io-reference.ppm", &ppm(&snapshot))?;
    let output = report.directory.join("io-snapshot.ppm");
    let measured = support::measure(
        || {
            snapshot.write_ppm(&output)?;
            Ok(std::fs::metadata(&output)?.len())
        },
        settings,
        settings.iterations,
    )?;
    report.record(
        "snapshot/write-ppm",
        measured,
        json!({"includes_filesystem":true}),
    )?;
    Ok(())
}
