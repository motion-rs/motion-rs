//! Primitive microbenchmarks for the breadth target: colour conversion, geometry, random
//! and noise sampling, and bitmap conversion hot paths.

use super::harness::{composition, limits};
use super::scenes::{artwork, teal, wave};
use super::{DESIGN, support};
use motion::extend::RenderPrepare;
use motion::prelude::*;
use motion::render::{ValidationLimits, linear_to_srgb, srgb_to_linear};
use std::hint::black_box;

pub(super) fn microbenchmarks(
    report: &mut support::Report,
    settings: &support::Settings,
    resources: &Assets,
) -> motion::Result {
    use serde_json::json;
    use std::f64::consts::PI;
    let mut counter = 0_u64;

    let measured = support::measure(
        || {
            let mut state = 0;
            for index in 0..4096 {
                let value = f64::from(index) / 4095.0;
                let encoded = linear_to_srgb(srgb_to_linear(black_box(value as f32)));
                state = support::fold_checksum(state, u64::from(encoded.to_bits()));
            }
            Ok(state)
        },
        settings,
        20,
    )?;
    report.record("micro/color/srgb-roundtrip-4096", measured, json!({}))?;

    let (first, second) = (Color::from_hex(0x178b79), Color::from_hex(0xf47762));
    let measured = support::measure(
        || {
            let mut state = 0;
            for index in 0..4096 {
                let color = first.interpolate(&second, f64::from(index) / 4095.0);
                state = support::fold_checksum(state, u64::from(color.to_rgba8()[0]));
            }
            Ok(state)
        },
        settings,
        20,
    )?;
    report.record("micro/color/oklab-interpolate-4096", measured, json!({}))?;

    let rotation = Transform::from_coefficients([
        math::cos(0.3),
        math::sin(0.3),
        -math::sin(0.3),
        math::cos(0.3),
        12.0,
        -7.0,
    ]);
    let measured = support::measure(
        || {
            let rotation = black_box(rotation);
            let combined = rotation.then(rotation).then(rotation);
            let inverse = combined.inverse().expect("invertible");
            let mut state = 0;
            for index in 0..4096 {
                let point = inverse.apply(combined.apply(Point::new(f64::from(index), 1.0)));
                state = support::fold_checksum(state, point.x.to_bits());
            }
            Ok(state)
        },
        settings,
        20,
    )?;
    report.record("micro/transform/compose-apply-4096", measured, json!({}))?;

    let mut data = String::from("M0 0");
    for index in 0..600 {
        let x = f64::from(index) * 1.5;
        data.push_str(&format!("C{} 40 {} -40 {} 0", x + 0.5, x + 1.0, x + 1.5));
    }
    let measured = support::measure(
        || Ok(Path2D::parse_svg(black_box(&data))?.verbs().len() as u64),
        settings,
        20,
    )?;
    report.record(
        "micro/path/parse-svg-600-cubics",
        measured,
        json!({"bytes": data.len()}),
    )?;

    // Stroke outlines are cached on the path value, so each call builds a fresh path; the timing
    // therefore includes constructing six cubic verbs.
    for (name, join) in [
        ("miter", LineJoin::Miter { limit: 4.0 }),
        ("round", LineJoin::Round),
    ] {
        let style = StrokeStyle {
            width: 14.0,
            cap: LineCap::Round,
            join,
        };
        let measured = support::measure(
            || {
                counter += 1;
                let curve = wave((counter % 8) as u32);
                Ok(style.outline(&curve, "bench")?.verbs().len() as u64)
            },
            settings,
            50,
        )?;
        report.record(
            format!("micro/path/stroke-outline-{name}-uncached"),
            measured,
            json!({"includes_path_construction": true}),
        )?;
    }

    let curve = wave(0);
    let style = StrokeStyle {
        width: 14.0,
        ..StrokeStyle::default()
    };
    let measured = support::measure(
        || Ok(style.outline(black_box(&curve), "bench")?.verbs().len() as u64),
        settings,
        200,
    )?;
    report.record(
        "micro/path/stroke-outline-cache-hit",
        measured,
        json!({"cached_on_path": true}),
    )?;

    let measured = support::measure(
        || {
            let mut random = Random::from_key(black_box(0x42_u64));
            let mut state = 0;
            for _ in 0..8192 {
                state = support::fold_checksum(state, random.next_float().to_bits());
            }
            Ok(state)
        },
        settings,
        20,
    )?;
    report.record("micro/random/next-float-8192", measured, json!({}))?;

    let noise = Noise::new(11).frequency(3.0);
    let measured = support::measure(
        || {
            counter += 1;
            let mut state = 0;
            for index in 0..1024 {
                let time = ((counter + index) % 3000).milliseconds();
                state = support::fold_checksum(state, noise.sample(time).to_bits());
            }
            Ok(state)
        },
        settings,
        50,
    )?;
    report.record("micro/animation/noise-1024", measured, json!({}))?;

    let rows = Column::new().gap(1.0).children((0..40).map(|row| {
        Row::new().gap(1.0).children((0..40).map(move |column| {
            Node::from(
                Rect::new(0.0, 0.0, 4.0, 4.0 + f64::from((row + column) % 3)).fill(Color::BLACK),
            )
        }))
    }));
    let measured = support::measure(
        || Ok(rows.measure(ValidationLimits::default())?.width().to_bits()),
        settings,
        10,
    )?;
    report.record(
        "micro/layout/measure-40x40",
        measured,
        json!({"leaves": 1600}),
    )?;

    let mut audio_clips = Vec::new();
    for (index, rate) in [48_000_u32, 44_100, 48_000, 44_100].into_iter().enumerate() {
        let samples = (0..rate as usize * 4)
            .map(|frame| {
                (math::sin(frame as f64 * 440.0 * 2.0 * PI / f64::from(rate)) * 0.2) as f32
            })
            .collect();
        audio_clips.push(
            AudioClip::source(motion::encode::PcmBuffer::new(rate, 1, samples)?)
                .start(Time::from_ticks(
                    index as i64 * motion::render::TICKS_PER_SECOND / 2,
                ))
                .volume(0.7)
                .fade_in(250.milliseconds())
                .pan(index as f64 * 0.3 - 0.4),
        );
    }
    let measured = support::measure(
        || Ok(motion::encode::MixedAudio::new(&audio_clips, 48_000, 6.seconds())?.frames()),
        settings,
        2,
    )?;
    report.record(
        "micro/audio/mix-4-clips-6s-with-resample",
        measured,
        json!({"mixed_rates": [44_100, 48_000]}),
    )?;

    let size = PixelSize::new(1920, 1080);
    let frame = {
        let composition = composition(size, Renderer::Cpu, resources)?;
        let mut session = composition.frame_renderer(DESIGN)?;
        session.render_frame(45, |context| artwork("gradient-linear", context))?;
        let list = Node::from(Rect::new(0.0, 0.0, 640.0, 360.0).fill(teal())).compile(
            DESIGN,
            size,
            ValidationLimits::default(),
        )?;
        list.prepare(ValidationLimits::default(), limits())?
            .render_linear(limits())?
    };
    let measured = support::measure_output(
        || Ok(motion::render::Bitmap::from_linear(&frame, limits())?),
        |bitmap| {
            bitmap.bytes().iter().step_by(4096).fold(0, |state, value| {
                support::fold_checksum(state, u64::from(*value))
            })
        },
        settings,
        settings.iterations,
    )?;
    report.record("micro/encode/bitmap-from-linear-1080p", measured, json!({}))?;
    let bitmap = motion::render::Bitmap::from_linear(&frame, limits())?;
    let png = report.directory.join("breadth-1080p.png");
    let measured = support::measure(
        || {
            bitmap.save_png_with_overwrite(&png, true)?;
            Ok(std::fs::metadata(&png)?.len())
        },
        settings,
        2,
    )?;
    report.record(
        "micro/encode/png-1080p",
        measured,
        json!({"includes_filesystem": true}),
    )?;

    #[cfg(feature = "diagram")]
    {
        use motion::diagram::{Label, LabelLayout};
        let labels = (0..200)
            .map(|index| {
                Label::new(
                    Point::new(f64::from(index % 20) * 30.0, f64::from(index / 20) * 30.0),
                    Size::new(40.0, 14.0),
                )
                .priority(index % 7)
            })
            .collect::<Vec<_>>();
        let layout = LabelLayout::new(Rect::new(0.0, 0.0, 640.0, 360.0)).gap(2.0);
        let measured = support::measure(
            || Ok(layout.place(black_box(&labels))?.len() as u64),
            settings,
            5,
        )?;
        report.record("micro/diagram/label-layout-200", measured, json!({}))?;
    }
    Ok(())
}
