//! Declarative ink validation, native equivalence and explicit-time replay.

use motion_core::{Color, Time};
use motion_scene::{CompileOptions, Document, Element, SceneSpec, TrackSpec};
use serde_json::{Value, json};

fn document(content: Value) -> Document {
    Document::new([64.0, 48.0], 30).track(TrackSpec::new().scene(SceneSpec::new(
        2.0,
        serde_json::from_value::<Element>(content).unwrap(),
    )))
}

fn brush() -> Value {
    json!({"path":"M8 32 Q32 8 56 32","width":6,"paint":"#167d71",
        "pressure":[0.2,1.2,0.1],"nib_aspect":0.42,"nib_mode":{"fixed":-20},
        "ink":{"seed":17,"dryness":0.1,"fibers":0.75,"granulation":0.2,
            "paper":{"procedural":{"seed":71,"strength":0.2,"scale":1.2}},
            "edge_pooling":[0.22,0.8]},
        "write":{"expr":"clamp(t,0,1)"}})
}

#[test]
fn ink_parses_without_preparation_features_and_rejects_ambiguous_content() {
    let original = document(json!({"brush_stroke":brush()}));
    assert_eq!(
        Document::from_json(&original.to_json().unwrap()).unwrap(),
        original
    );
    assert!(serde_json::from_value::<Element>(
        json!({"brush_stroke":brush(),"ink_fill":{"path":"M0 0L8 0L8 8Z","paint":"black","ink":{}}})
    ).is_err());
    if !cfg!(feature = "ink") {
        for content in [
            json!({"brush_stroke":brush()}),
            json!({"ink_fill":{"path":"M0 0L8 0L8 8Z","paint":"black","ink":{}}}),
            json!({"rect":{"bounds":[0,0,8,8]},"fill":{"pigment":{
                "first":"yellow","second":"blue","amount":0.5,"mode":"kubelka_munk"}}}),
        ] {
            let errors = document(content)
                .compile(&CompileOptions::default())
                .unwrap_err();
            assert!(errors.to_string().contains("feature `ink`"), "{errors}");
        }
    }
}

#[cfg(feature = "ink")]
fn pixels(timeline: &motion_core::Timeline, time: f64) -> Vec<u8> {
    motion_render::render_still(
        timeline,
        Time::from_seconds(time),
        &motion_render::RenderOptions::default(),
    )
    .unwrap()
    .bytes()
    .to_vec()
}

#[cfg(feature = "ink")]
#[test]
fn ink_brush_matches_native_material_and_replays_after_seek() {
    use motion_content::vector::ink::{BrushStroke, Ink, Paper};
    use motion_core::{Duration, Node, Path2D, Size, Timeline};
    let actual = document(json!({"brush_stroke":brush()}))
        .compile(&CompileOptions::default())
        .unwrap();
    let stroke = BrushStroke::builder(
        Path2D::parse_svg("M8 32 Q32 8 56 32").unwrap(),
        6.0,
        Color::from_hex(0x167d71),
    )
    .pressure(0.2, 1.2, 0.1)
    .nib(0.42, -20_f64.to_radians())
    .ink(
        Ink::new(17)
            .dryness(0.1)
            .fibers(0.75)
            .granulation(0.2)
            .paper(Paper::Procedural {
                seed: 71,
                strength: 0.2,
                scale: 1.2,
            })
            .edge_pooling(0.22, 0.8),
    )
    .build()
    .unwrap();
    let expected = Timeline::new(Size::new(64.0, 48.0), motion_core::fps!(30))
        .track(|track| {
            track.scene(Duration::from_seconds(2.0), move |context| {
                Node::from(stroke.clone().write(context.seconds().clamp(0.0, 1.0)))
            })
        })
        .build()
        .unwrap();
    let first = pixels(actual.timeline(), 0.5);
    assert!(first.as_chunks::<4>().0.iter().any(|pixel| pixel[3] > 0));
    for time in [0.5, 1.5, 0.0, 0.5] {
        assert_eq!(
            pixels(actual.timeline(), time),
            pixels(&expected, time),
            "time={time}"
        );
    }
    assert_eq!(pixels(actual.timeline(), 0.5), first);
}

#[cfg(feature = "ink")]
#[test]
fn ink_recorded_pause_and_timed_pen_lifts_match_native() {
    use motion_content::vector::ink::{BrushStroke, BrushStrokeBuilder, BrushTiming, InputSample};
    use motion_core::{Duration, Path2D};
    let samples = [
        InputSample {
            position: motion_core::Point::new(8.0, 24.0),
            pressure: 0.2,
            tilt: 0.0,
            time: Time::ZERO,
        },
        InputSample {
            position: motion_core::Point::new(32.0, 24.0),
            pressure: 1.0,
            tilt: 0.0,
            time: Time::from_seconds(0.5),
        },
        InputSample {
            position: motion_core::Point::new(32.0, 24.0),
            pressure: 1.0,
            tilt: 0.0,
            time: Time::from_seconds(1.0),
        },
        InputSample {
            position: motion_core::Point::new(56.0, 24.0),
            pressure: 0.2,
            tilt: 0.0,
            time: Time::from_seconds(1.5),
        },
    ];
    let recording = BrushStrokeBuilder::from_samples(&samples, 6.0, Color::BLACK)
        .unwrap()
        .build()
        .unwrap();
    let actual = document(json!({"brush_stroke":{
        "width":6,"paint":"black","samples":samples.iter().map(|sample|json!({
            "position":sample.position,"pressure":sample.pressure,"tilt":0,"time":sample.time.as_seconds()
        })).collect::<Vec<_>>(),"write_time":{"expr":"t"}
    }})).compile(&CompileOptions::default()).unwrap();
    assert_eq!(
        pixels(actual.timeline(), 0.6),
        pixels(actual.timeline(), 0.9)
    );
    let timed_path = "M8 16L28 16 M36 32L56 32";
    let timed = document(
        json!({"brush_stroke":{"path":timed_path,"width":6,"paint":"black",
        "write_time":{"expr":"t"},"timing":{"duration":1,"lift_pause":0.5}}}),
    )
    .compile(&CompileOptions::default())
    .unwrap();
    let stroke = BrushStroke::builder(Path2D::parse_svg(timed_path).unwrap(), 6.0, Color::BLACK)
        .build()
        .unwrap();
    let mut timing = BrushTiming::new(Duration::from_seconds(1.0));
    timing.lift_pause = Duration::from_seconds(0.5);
    for (film, source, timing) in [
        (actual.timeline(), recording, None),
        (timed.timeline(), stroke, Some(timing)),
    ] {
        for time in [0.25, 0.6, 0.9, 1.4, 0.25] {
            let revealed = match &timing {
                Some(timing) => source
                    .clone()
                    .write_timed(Time::from_seconds(time), timing)
                    .unwrap(),
                None => source.clone().write_with(Time::from_seconds(time)).unwrap(),
            };
            let expected = motion_core::Timeline::new(
                motion_core::Size::new(64.0, 48.0),
                motion_core::fps!(30),
            )
            .track(|track| {
                track.scene(Duration::from_seconds(2.0), move |_| {
                    motion_core::Node::from(revealed.clone())
                })
            })
            .build()
            .unwrap();
            assert_eq!(pixels(film, time), pixels(&expected, time), "time={time}");
        }
    }
}

#[cfg(feature = "ink")]
#[test]
fn ink_fill_and_pigment_paints_match_native() {
    use motion_content::vector::ink::{Ink, InkFill, PigmentMix};
    use motion_core::{Duration, Node, Path2D, Size, Timeline};
    let path = "M8 8L56 8L56 40L8 40Z M24 16L40 16L40 32L24 32Z";
    let first = Color::from_hex(0xffda28);
    let second = Color::from_hex(0x2466cc);
    let actual = document(json!({"ink_fill":{
        "path":path,
        "paint":{"pigment":{"first":first,"second":second,"amount":{"expr":"clamp(t,0,1)"},"mode":"kubelka_munk"}},
        "ink":{"seed":17,"granulation":0.2,"edge_pooling":[0.3,1]}
    }})).compile(&CompileOptions::default()).unwrap();
    let geometry = Path2D::parse_svg(path).unwrap();
    let expected = Timeline::new(Size::new(64.0, 48.0), motion_core::fps!(30))
        .track(|track| {
            track.scene(Duration::from_seconds(2.0), move |context| {
                let paint = PigmentMix::KubelkaMunk
                    .mix(first, second, context.seconds().clamp(0.0, 1.0))
                    .unwrap();
                Node::from(InkFill::new(
                    geometry.clone(),
                    paint,
                    Ink::new(17).granulation(0.2).edge_pooling(0.3, 1.0),
                ))
            })
        })
        .build()
        .unwrap();
    for time in [0.0, 0.5, 1.5, 0.5] {
        assert_eq!(
            pixels(actual.timeline(), time),
            pixels(&expected, time),
            "time={time}"
        );
    }
}

#[cfg(feature = "ink")]
#[test]
fn ink_rejects_invalid_recipes_and_reports_sampled_errors() {
    for patch in [
        json!({"points":{"points":[[0,0],[1,1]]}}),
        json!({"width":513}),
        json!({"pressure_curve":[[0,1],[1,1]]}),
        json!({"sample_limit":513}),
        json!({"bristles":{"count":65,"spread":1,"jitter":0,"seed":1}}),
        json!({"write_time":0}),
        json!({"wobble":{"amplitude":1,"frequency":1,"seed":1,"frame":-1}}),
        json!({"wetness":{"age":0,"radius":1,"drying":0}}),
        json!({"ink":{"paper":{"noise_texture":{"size":[513,1],"seed":1,"grain_scale":1,"absorbency":1}}}}),
    ] {
        let mut spec = brush();
        spec.as_object_mut()
            .unwrap()
            .extend(patch.as_object().unwrap().clone());
        let errors = document(json!({"brush_stroke":spec}))
            .compile(&CompileOptions::default())
            .unwrap_err();
        assert!(errors.to_string().contains("brush_stroke"), "{errors}");
    }
    for patch in [
        json!({"write":{"expr":"2*t"}}),
        json!({"wetness":{"age":{"expr":"1-t"},"radius":1,"drying":1}}),
        json!({"wobble":{"amplitude":1,"frequency":1,"seed":1,"frame":{"expr":"1-t"}}}),
    ] {
        let mut spec = brush();
        spec.as_object_mut()
            .unwrap()
            .extend(patch.as_object().unwrap().clone());
        let film = document(json!({"brush_stroke":spec}))
            .compile(&CompileOptions::default())
            .unwrap();
        let error = motion_render::render_still(
            film.timeline(),
            Time::from_seconds(1.5),
            &Default::default(),
        )
        .unwrap_err();
        assert!(error.to_string().contains("brush_stroke"), "{error}");
    }
}

#[test]
fn frame_seeded_grain_matches_full_width_native_seed() {
    use motion_core::{Duration, Filter, NodeModifiers, Rect, Shape, Size, Timeline};
    let actual = document(json!({"rect":{"bounds":[0,0,64,48]},"fill":"gray",
        "filters":[{"grain":{"seed":u64::MAX,"strength":0.1,"frame":{"expr":"floor(t*30)"}}}]}))
    .compile(&CompileOptions::default())
    .unwrap();
    let expected = Timeline::new(Size::new(64.0, 48.0), motion_core::fps!(30))
        .track(|track| {
            track.scene(Duration::from_seconds(2.0), |context| {
                Rect::new(0.0, 0.0, 64.0, 48.0)
                    .fill(Color::rgb(128, 128, 128))
                    .filter(Filter::grain(
                        u64::MAX ^ context.time().to_frame(motion_core::fps!(30)) as u64,
                        0.1,
                    ))
            })
        })
        .build()
        .unwrap();
    for time in [0.0, 0.5, 1.0, 0.5] {
        let render = |timeline| {
            motion_render::render_still(timeline, Time::from_seconds(time), &Default::default())
                .unwrap()
                .bytes()
                .to_vec()
        };
        assert_eq!(render(actual.timeline()), render(&expected));
    }
}
