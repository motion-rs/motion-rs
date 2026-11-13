//! Particles, diagrams, captions, formulas and morphs in declarative documents.

use motion_core::{Color, Pivot, Point, Rect, Time, ValidationError};
use motion_render::{FrameBuffer, RenderOptions, render_still};
use motion_scene::*;

const FONTS: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/../../tests/fixtures/fonts");

fn film(element: Element) -> CompiledDocument {
    document(element)
        .compile(&CompileOptions::default().base_dir(FONTS))
        .unwrap()
}

fn document(element: Element) -> Document {
    let document = Document::new([240.0, 120.0], 10)
        .track(TrackSpec::new().scene(SceneSpec::new(2.0, element)));
    if cfg!(feature = "text") {
        document.font("NotoSans-Regular.ttf")
    } else {
        document
    }
}

fn render(film: &CompiledDocument, seconds: f64) -> FrameBuffer {
    render_still(
        film.timeline(),
        Time::from_seconds(seconds),
        &film.render_options(RenderOptions::default()),
    )
    .unwrap()
}

#[cfg(any(
    feature = "particles",
    feature = "diagram",
    feature = "captions",
    feature = "formula"
))]
fn ink(bitmap: &FrameBuffer) -> usize {
    bitmap
        .bytes()
        .chunks(4)
        .filter(|pixel| pixel[3] > 40)
        .count()
}

fn messages(document: &Document) -> Vec<String> {
    let error = document
        .compile(&CompileOptions::default().base_dir(FONTS))
        .unwrap_err();
    error
        .errors()
        .iter()
        .map(|error| {
            let error = error.downcast_ref::<ValidationError>().unwrap();
            format!("{} {}", error.path, error.message)
        })
        .collect()
}

#[cfg(feature = "particles")]
fn burst_simulation() -> ParticlesSpec {
    ParticlesSpec::new(7)
        .emitter(
            EmitterSpec::new(EmitterShape::Burst {
                time: 0.0,
                count: 60,
                position: Point::new(120.0, 60.0),
            })
            .speed(40.0)
            .lifetime(1.5)
            .color(Color::rgb(255, 200, 100)),
        )
        .modifier(ModifierSpec::Gravity([0.0, 30.0]))
        .modifier(ModifierSpec::Drag(0.2))
        .modifier(ModifierSpec::SizeOverLifetime {
            start: 3.0,
            end: 1.0,
        })
        .style(ParticleStyleSpec::Dots { radius: 2.0 })
}

#[test]
fn particle_presets_parse_and_round_trip_without_optional_features() {
    let source = serde_json::json!({"seed":u64::MAX,"duration":4,
        "preset":{"kind":"sparks","bounds":[0,0,64,48],"density":0.1},
        "sample_time":{"expr":"0.12 + t"}});
    let spec: ParticlesSpec = serde_json::from_value(source).unwrap();
    assert_eq!(spec.seed, u64::MAX);
    assert_eq!(
        serde_json::from_str::<ParticlesSpec>(&serde_json::to_string(&spec).unwrap()).unwrap(),
        spec
    );
    if !cfg!(feature = "particles") {
        assert!(
            messages(&document(Element::new(Content::Particles(spec))))
                .iter()
                .any(|message| message.contains("feature `particles`"))
        );
    }
}

#[cfg(feature = "particles")]
#[test]
fn particle_presets_match_shared_recipes_and_seek_exactly() {
    use motion_content::particles::presets::{Preset, PresetOptions};
    use motion_core::{Duration, Rect, Size, Timeline};
    use serde_json::json;
    let kinds = [
        "sparks",
        "embers",
        "dust",
        "motes",
        "snow",
        "rain",
        "smoke",
        "fire",
        "confetti",
        "fireworks",
        "fireflies",
        "logo_dissolve",
        "logo_assemble",
    ];
    let data = "M22.4 14.4 L41.6 14.4 L41.6 33.6 L22.4 33.6 Z";
    let fill = json!({"linear":{"start":[0,0],"end":[1,0],"stops":[
        {"offset":0,"color":"#25bb90"},{"offset":1,"color":"#ff4060"}]}});
    let logo = vec![ParticleVectorPathSpec {
        data: data.into(),
        fill: serde_json::from_value(fill.clone()).unwrap(),
    }];
    for (kind, preset) in kinds.into_iter().zip(Preset::ALL) {
        let spec: ParticlesSpec = serde_json::from_value(json!({"seed":u64::MAX,"duration":4,
            "preset":{"kind":kind,"bounds":[0,0,64,48],"density":0.1,"logo":logo},
            "sample_time":{"expr":"0.12 + t * 0.5"}}))
        .unwrap();
        let actual = film(Element::new(Content::Particles(spec)));
        let mut recipe = PresetOptions::new(
            u64::MAX,
            Duration::from_seconds(4.0),
            Rect::new(0.0, 0.0, 64.0, 48.0),
        );
        recipe.density = 0.1;
        recipe.logo = Some(std::sync::Arc::new(
            motion_content::VectorTree::new(
                recipe.bounds,
                motion_content::VectorGroup {
                    children: vec![motion_content::VectorItem::Path(
                        motion_content::VectorPath::filled(
                            motion_core::Path2D::parse_svg(data).unwrap(),
                            motion_core::LinearGradient::new(
                                motion_core::Point::ORIGIN,
                                motion_core::Point::new(1.0, 0.0),
                            )
                            .stop(0.0, Color::from_hex(0x25bb90))
                            .stop(1.0, Color::from_hex(0xff4060)),
                        ),
                    )],
                    ..Default::default()
                },
            )
            .unwrap(),
        ));
        let simulation = preset.simulation(recipe).unwrap().build().unwrap();
        let expected = Timeline::new(Size::new(240.0, 120.0), motion_core::fps!(10))
            .track(|track| {
                track.scene(Duration::from_seconds(2.0), move |context| {
                    let sample =
                        Duration::from_seconds(0.12 + context.elapsed().as_seconds() * 0.5)
                            .after_origin();
                    let sampled = context
                        .try_shifted(context.time() - sample)?
                        .with_clip_duration(Duration::from_seconds(4.0));
                    simulation.try_render(&sampled)
                })
            })
            .build()
            .unwrap();
        let options = actual.render_options(
            RenderOptions::default().backend(motion_render::BackendPreference::Cpu),
        );
        for seconds in [0.8, 0.0, 0.4, 0.8] {
            let time = Time::from_seconds(seconds);
            assert_eq!(
                render_still(actual.timeline(), time, &options)
                    .unwrap()
                    .bytes(),
                render_still(&expected, time, &options).unwrap().bytes(),
                "{kind} at {seconds}"
            );
        }
    }
}

#[cfg(feature = "particles")]
#[test]
fn particle_presets_reject_conflicts_limits_and_invalid_sample_times() {
    use serde_json::json;
    let base = json!({"seed":17,"duration":4,"preset":{"kind":"sparks","bounds":[0,0,64,48]}});
    let make = |source| {
        document(Element::new(Content::Particles(
            serde_json::from_value(source).unwrap(),
        )))
    };
    for changes in [
        json!({"emitters":[{"point":[0,0]}]}),
        json!({"modifiers":[{"drag":1}]}),
        json!({"style":{"dots":{"radius":1}}}),
        json!({"blend":"plus"}),
        json!({"duration":0}),
        json!({"step":0}),
        json!({"max_particles":0}),
        json!({"sample_time":-1}),
        json!({"sample_time":1000001}),
        json!({"preset":{"kind":"sparks","bounds":[0,0,64,48],"density":0}}),
        json!({"preset":{"kind":"sparks","bounds":[0,0,64,48],"speed":17}}),
        json!({"preset":{"kind":"sparks","bounds":[0,0,64,48],"intensity":65}}),
        json!({"preset":{"kind":"sparks","bounds":[0,0,0,48]}}),
        json!({"preset":{"kind":"logo_dissolve","bounds":[0,0,64,48]}}),
        json!({"preset":{"kind":"sparks","bounds":[0,0,64,48],"logo":vec![json!({"data":"M0 0 L1 0 L1 1 Z","fill":"#fff"});257]}}),
        json!({"preset":{"kind":"sparks","bounds":[0,0,64,48],"logo":[{"data":"M0 0 L1 0 L1 1 Z","fill":{"tween":{"from":"#fff","to":"#000","duration":1}}}]}}),
    ] {
        let mut source = base.clone();
        source
            .as_object_mut()
            .unwrap()
            .extend(changes.as_object().unwrap().clone());
        assert!(!messages(&make(source)).is_empty());
    }
    let mut missing = base.clone();
    missing.as_object_mut().unwrap().remove("duration");
    assert!(!messages(&make(missing)).is_empty());
    for expression in ["0.1 - t", "1 / (0.5 - t)"] {
        let mut source = base.clone();
        source["sample_time"] = json!({"expr":expression});
        let compiled = make(source)
            .compile(&CompileOptions::default().base_dir(FONTS))
            .unwrap();
        assert!(
            render_still(
                compiled.timeline(),
                Time::from_seconds(0.5),
                &RenderOptions::default()
            )
            .is_err()
        );
    }
}

#[cfg(feature = "particles")]
#[test]
fn particle_simulations_are_deterministic_and_animate() {
    let film = film(Element::new(Content::Particles(burst_simulation())));
    let early = render(&film, 0.2);
    assert!(ink(&early) > 20, "burst should be visible: {}", ink(&early));
    assert_eq!(early.bytes(), render(&film, 0.2).bytes(), "replay is exact");
    assert_ne!(early.bytes(), render(&film, 0.8).bytes());
    assert_eq!(
        ink(&render(&film, 1.9)),
        0,
        "particles die after their lifetime"
    );
}

#[cfg(feature = "particles")]
#[test]
fn particle_documents_parse_round_trip_and_validate() {
    let emitter: EmitterSpec = serde_json::from_str(
        r##"{"disk": {"center": [10, 10], "radius": 4}, "rate": 20, "color": "#fff"}"##,
    )
    .unwrap();
    assert_eq!(
        serde_json::from_str::<EmitterSpec>(&serde_json::to_string(&emitter).unwrap()).unwrap(),
        emitter
    );
    assert!(
        serde_json::from_str::<EmitterSpec>(r#"{"point": [1, 1], "rect": [0, 0, 1, 1]}"#)
            .unwrap_err()
            .to_string()
            .contains("choose one shape")
    );
    assert!(serde_json::from_str::<EmitterSpec>(r#"{"rate": 1}"#).is_err());

    let broken = document(Element::new(Content::Particles(
        ParticlesSpec::new(1)
            .emitter(
                EmitterSpec::new(EmitterShape::Burst {
                    time: 0.0,
                    count: 5,
                    position: Point::new(0.0, 0.0),
                })
                .rate(3.0),
            )
            .modifier(ModifierSpec::Seek {
                targets: vec![],
                strength: 1.0,
            }),
    )));
    let found = messages(&broken).join("\n");
    assert!(found.contains("take no rate"), "{found}");
    assert!(found.contains("1 to 65536 targets"), "{found}");
    assert!(
        messages(&document(Element::new(Content::Particles(
            ParticlesSpec::new(1)
        ))))
        .join("\n")
        .contains("at least one emitter")
    );
}

#[cfg(feature = "particles")]
#[test]
fn trajectories_render_closed_form_bursts() {
    let element = Element::new(Content::Trajectory(TrajectorySpec {
        model: TrajectoryModelSpec::Ballistic {
            center: Point::new(120.0, 60.0),
            count: 80,
            gravity: Some([0.0, 60.0]),
            drag: Some(0.5),
            speed: Some([20.0, 60.0]),
            seed: 3,
            lifetime: Some(1.5),
        },
        sample_time: None,
        style: Some(ParticleStyleSpec::GlowDots { radius: 2.0 }),
        blend: Some(ParticleBlendSpec::Plus),
    }));
    assert!(ink(&render(&film(element), 0.5)) > 20);
}

#[cfg(feature = "particles")]
#[test]
fn convergence_uses_explicit_sample_time_and_validates_points() {
    let source = serde_json::json!({
        "model": {"converge": {"from": [[20, 60], [40, 60]],
            "to": [[180, 60], [200, 60]], "duration": 2}},
        "sample_time": 1,
        "style": {"dots": {"radius": 3}}
    });
    let spec: TrajectorySpec = serde_json::from_value(source.clone()).unwrap();
    assert_eq!(serde_json::to_value(&spec).unwrap()["sample_time"], 1.0);
    let actual = film(Element::new(Content::Trajectory(spec)));
    assert_eq!(render(&actual, 0.0).bytes(), render(&actual, 1.5).bytes());
    let expected = film(Element::new(Content::Trajectory(
        serde_json::from_value(serde_json::json!({
            "model": source["model"], "style": source["style"]
        }))
        .unwrap(),
    )));
    assert_eq!(render(&actual, 0.0).bytes(), render(&expected, 1.0).bytes());
    for invalid in [
        serde_json::json!({"model": {"converge": {
            "from": [[20, 60]], "to": [], "duration": 2}}}),
        serde_json::json!({"model": source["model"], "sample_time": -1}),
    ] {
        let spec = serde_json::from_value(invalid).unwrap();
        assert!(!messages(&document(Element::new(Content::Trajectory(spec)))).is_empty());
    }
}

#[cfg(feature = "particles")]
#[test]
fn extended_particle_inputs_reject_invalid_configuration_and_materials() {
    use serde_json::json;
    let source = json!({"seed": 17, "duration": 4, "emitters": [{
        "burst": {"time": 0, "count": 8, "position": [120, 60]}, "lifetime": 4
    }]});
    for patch in [
        json!({"configuration": {"workers": 0}}),
        json!({"configuration": {"loop_period": 0}}),
        json!({"configuration": {"time_remap": [-1, 3]}}),
        json!({"configuration": {"max_work": 0}}),
        json!({"configuration": {"trails": {"segments": 0}}}),
        json!({"modifiers": [{"intensity_over_lifetime": {"stops": [[0, 1], [0, 2]]}}]}),
        json!({"emitters": [{"image": {"image": {"size": [2, 2], "rgba": []},
            "bounds": [0, 0, 20, 20]}}]}),
        json!({"emitters": [{"point": [120, 60], "path_progress": 0.5}]}),
        json!({"emitters": [{"point": [120, 60], "premultiplied_color": [1, 1, 1, 2]}]}),
        json!({"modifiers": [{"operator": {"custom_slot": {"slot": 4, "modulus": null, "scale": 1}}}]}),
        json!({"sub_emitters": [{"on": "birth", "emitter": {"burst": {"time": 0,
            "count": 2, "position": [0, 0]}}, "inherit": "position", "probability": 2}]}),
        json!({"style": {"sprites": {"seed": 17, "size": 32, "padded": true,
            "mode": "custom", "frames": [{"material": "star", "size": 257}]}}}),
        json!({"preset": {"kind": "sparks", "bounds": [0, 0, 240, 120]}, "emitters": [],
            "configuration": {"flip": true}}),
    ] {
        let mut candidate = source.clone();
        candidate
            .as_object_mut()
            .unwrap()
            .extend(patch.as_object().unwrap().clone());
        let spec: ParticlesSpec = serde_json::from_value(candidate.clone()).unwrap();
        assert!(
            !messages(&document(Element::new(Content::Particles(spec)))).is_empty(),
            "Accepted {candidate}"
        );
    }
    for (patch, expected) in [
        (
            json!({"modifiers": [{"operator": {"custom_slot": {"slot": 3, "modulus": null, "scale": 1}}}]}),
            "slot 3 is reserved",
        ),
        (
            json!({"modifiers": [{"curl_noise": {"frequency": 0.02, "strength": 6,
                "phase_speed": 0, "seed": 4_294_967_296_u64}}]}),
            "Noise field seeds",
        ),
        (
            json!({"modifiers": [{"operator": {"curl_flow": {"seed": u64::MAX,
                "frequency": 0.02, "strength": 6}}}]}),
            "Noise field seeds",
        ),
    ] {
        let mut candidate = source.clone();
        candidate
            .as_object_mut()
            .unwrap()
            .extend(patch.as_object().unwrap().clone());
        let spec: ParticlesSpec = serde_json::from_value(candidate).unwrap();
        let messages = messages(&document(Element::new(Content::Particles(spec))));
        assert!(
            messages.iter().any(|message| message.contains(expected)),
            "{messages:?}"
        );
    }
}

#[cfg(all(feature = "particles", feature = "text"))]
#[test]
#[ignore = "requires explicit fixture font; checks font-snapshot changes after cached failure"]
fn glyph_particle_cache_distinguishes_font_snapshots_after_failure() {
    let spec: ParticlesSpec = serde_json::from_value(serde_json::json!({
        "seed": 17, "duration": 2, "sample_time": 1,
        "emitters": [{"text": {"content": "motion", "family": "", "size": 24,
            "fill": "white"}, "rate": 50, "speed": 0, "lifetime": 2}]
    }))
    .unwrap();
    let document = Document::new([240.0, 120.0], 10)
        .track(TrackSpec::new().scene(SceneSpec::new(2.0, Element::new(Content::Particles(spec)))));
    let compiled = document.compile(&Default::default()).unwrap();
    let assets = motion_core::Assets::new(FONTS).unwrap();
    let options = RenderOptions::default().resources(assets.clone());
    assert!(render_still(compiled.timeline(), Time::ZERO, &options).is_err());
    let mut fonts = motion_content::FontLibrary::new();
    fonts
        .load_file(std::path::Path::new(FONTS).join("NotoSans-Regular.ttf"))
        .unwrap();
    let options = options.resources(assets.with_fonts(fonts));
    let first = render_still(compiled.timeline(), Time::ZERO, &options).unwrap();
    assert!(ink(&first) > 20);
    assert_eq!(
        first.bytes(),
        render_still(compiled.timeline(), Time::ZERO, &options)
            .unwrap()
            .bytes()
    );
}

#[cfg(feature = "diagram")]
#[test]
fn rulers_callouts_and_charts_draw_with_document_fonts() {
    let ruler = Element::new(Content::Ruler(RulerSpec {
        ticks: Some(5),
        nice: true,
        font_size: Some(10.0),
        label_size: Some([30.0, 12.0]),
        color: Some(Color::WHITE),
        ..RulerSpec::horizontal(100.0, ScaleSpec::linear([0.0, 100.0]).range([20.0, 220.0]))
    }));
    let ruler_ink = ink(&render(&film(ruler), 0.0));
    assert!(ruler_ink > 200, "ruler drew {ruler_ink} pixels");

    let callout = Element::new(Content::Callout(CalloutSpec {
        text: "Peak".into(),
        target: Point::new(120.0, 100.0),
        bounds: Rect::new(20.0, 10.0, 100.0, 40.0),
        color: Some(Color::WHITE),
        font_size: Some(14.0),
        leader_width: Some(2.0),
    }));
    assert!(ink(&render(&film(callout), 0.0)) > 100);

    let chart = Element::new(Content::LineChart(LineChartSpec {
        bounds: Rect::new(20.0, 10.0, 220.0, 100.0),
        horizontal: ScaleSpec::linear([0.0, 10.0]),
        vertical: ScaleSpec::linear([0.0, 100.0]),
        ticks: Some(4),
        series: vec![SeriesSpec {
            points: vec![
                Some(Point::new(0.0, 10.0)),
                Some(Point::new(5.0, 80.0)),
                None,
                Some(Point::new(9.0, 30.0)),
                Some(Point::new(10.0, 40.0)),
            ],
            color: Color::rgb(255, 100, 100),
            width: Some(2.0),
            cap: None,
            join: None,
        }],
    }));
    let chart = film(chart);
    let red = render(&chart, 0.0)
        .bytes()
        .chunks(4)
        .filter(|pixel| pixel[3] > 128 && pixel[0] > 200 && pixel[1] < 140)
        .count();
    assert!(red > 100, "series drew {red} red pixels");

    let broken = document(Element::new(Content::Ruler(RulerSpec::horizontal(
        1.0,
        ScaleSpec::linear([0.0, 1.0]),
    ))));
    assert!(messages(&broken).join("\n").contains("requires a `range`"));
}

#[cfg(feature = "captions")]
#[test]
fn captions_select_cues_by_scene_time() {
    let element = Element::new(Content::Captions(CaptionsSpec {
        font_size: Some(18.0),
        keywords: vec![KeywordSpec {
            word: "Rust".into(),
            color: Color::rgb(255, 220, 0),
        }],
        ..CaptionsSpec::new([
            CueSpec::new(0.5, 1.0, "Hello Rust"),
            CueSpec::new(1.0, 1.5, "Second cue"),
        ])
    }));
    let film = film(element);
    assert_eq!(ink(&render(&film, 0.1)), 0);
    let first = render(&film, 0.7);
    assert!(ink(&first) > 100);
    let yellow = first
        .bytes()
        .chunks(4)
        .filter(|pixel| pixel[3] > 128 && pixel[0] > 200 && pixel[2] < 80)
        .count();
    assert!(yellow > 20, "keyword drew {yellow} pixels");
    assert_ne!(first.bytes(), render(&film, 1.2).bytes());
    assert_eq!(ink(&render(&film, 1.8)), 0);

    let bare = Document::new([240.0, 120.0], 10).track(TrackSpec::new().scene(SceneSpec::new(
        1.0,
        Element::new(Content::Captions(CaptionsSpec::new([CueSpec::new(
            0.0, 1.0, "x",
        )]))),
    )));
    assert!(messages(&bare).join("\n").contains("declare `fonts`"));
}

#[test]
fn paths_morph_between_shapes() {
    let element = Element::new(Content::Morph(MorphSpec {
        from: "M10 10 H50 V50 H10 Z".into(),
        to: "M170 10 H210 V50 H170 Z".into(),
        progress: Animated::tween(0.0, 1.0, 2.0),
        samples: None,
    }))
    .fill(Color::WHITE)
    .pivot(Pivot::TopLeft);
    let film = film(element);
    let start = render(&film, 0.0);
    let end = render(&film, 1.99);
    let at = |bitmap: &FrameBuffer, x: usize, y: usize| bitmap.bytes()[(y * 240 + x) * 4 + 3];
    assert_eq!(at(&start, 30, 30), 255);
    assert_eq!(at(&start, 190, 30), 0);
    assert_eq!(at(&end, 190, 30), 255);
    assert_eq!(at(&end, 30, 30), 0);

    let missing_fill = document(Element::new(Content::Morph(MorphSpec {
        from: "M0 0 H1 V1 Z".into(),
        to: "M0 0 H2 V2 Z".into(),
        progress: 0.5.into(),
        samples: Some(2),
    })));
    let found = messages(&missing_fill).join("\n");
    assert!(found.contains("fill or a stroke"), "{found}");
    assert!(found.contains("4..=256"), "{found}");
}

#[cfg(feature = "formula")]
#[test]
fn formulas_draw_write_and_morph() {
    let plain = film(Element::new(Content::Formula(FormulaSpec {
        size: Some(32.0),
        ..FormulaSpec::new("x^2 + 1")
    })));
    let full = ink(&render(&plain, 0.0));
    assert!(full > 100);

    let written = film(Element::new(Content::Formula(
        FormulaSpec::new("x^2 + 1")
            .size_of(32.0)
            .write_with(Animated::tween(0.0, 1.0, 1.0)),
    )));
    assert!(ink(&render(&written, 0.3)) < ink(&render(&written, 1.5)));

    let morphing = film(Element::new(Content::Formula(
        FormulaSpec::new("a + b").morph_to("b + a", Animated::tween(0.0, 1.0, 1.0)),
    )));
    assert_ne!(
        render(&morphing, 0.1).bytes(),
        render(&morphing, 0.9).bytes()
    );

    let bad = document(Element::new(Content::Formula(FormulaSpec::new("frac(("))));
    assert!(messages(&bad).join("\n").contains("source"));
}

#[cfg(feature = "svg")]
#[test]
fn svgs_morph_between_files() {
    let directory = tempfile::tempdir().unwrap();
    let svg = |x: u32| {
        format!(
            r##"<svg xmlns="http://www.w3.org/2000/svg" width="240" height="120" viewBox="0 0 240 120">
                <rect id="box" x="{x}" y="20" width="40" height="40" fill="#ffffff"/></svg>"##
        )
    };
    std::fs::write(directory.path().join("a.svg"), svg(10)).unwrap();
    std::fs::write(directory.path().join("b.svg"), svg(170)).unwrap();
    let element = Element::new(Content::SvgMorph(SvgMorphSpec {
        from: "a.svg".into(),
        to: "b.svg".into(),
        progress: Animated::tween(0.0, 1.0, 2.0),
    }));
    let film = Document::new([240.0, 120.0], 10)
        .track(TrackSpec::new().scene(SceneSpec::new(2.0, element)))
        .compile(&CompileOptions::default().base_dir(directory.path()))
        .unwrap();
    let at = |bitmap: &FrameBuffer, x: usize, y: usize| bitmap.bytes()[(y * 240 + x) * 4 + 3];
    let start = render(&film, 0.0);
    let end = render(&film, 1.99);
    assert_eq!(at(&start, 30, 40), 255);
    assert_eq!(at(&start, 190, 40), 0);
    assert_eq!(at(&end, 190, 40), 255);
}
