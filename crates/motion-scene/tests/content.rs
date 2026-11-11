//! Particles, diagrams, captions, formulas and morphs in declarative documents.

use motion_core::{Color, Time, ValidationError};
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
                position: [120.0, 60.0],
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
                    position: [0.0, 0.0],
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
            center: [120.0, 60.0],
            count: 80,
            gravity: Some([0.0, 60.0]),
            drag: Some(0.5),
            speed: Some([20.0, 60.0]),
            seed: 3,
            lifetime: Some(1.5),
        },
        style: Some(ParticleStyleSpec::GlowDots { radius: 2.0 }),
        blend: Some(ParticleBlendSpec::Plus),
    }));
    assert!(ink(&render(&film(element), 0.5)) > 20);
}

#[cfg(feature = "diagram")]
#[test]
fn rulers_callouts_and_charts_draw_with_document_fonts() {
    let ruler = Element::new(Content::Ruler(RulerSpec {
        ticks: Some(5),
        nice: true,
        font_size: Some(10.0),
        label_size: Some([30.0, 12.0]),
        color: Some(Color::WHITE.into()),
        ..RulerSpec::horizontal(100.0, ScaleSpec::linear([0.0, 100.0]).range([20.0, 220.0]))
    }));
    let ruler_ink = ink(&render(&film(ruler), 0.0));
    assert!(ruler_ink > 200, "ruler drew {ruler_ink} pixels");

    let callout = Element::new(Content::Callout(CalloutSpec {
        text: "Peak".into(),
        target: [120.0, 100.0],
        bounds: [20.0, 10.0, 100.0, 40.0],
        color: Some(Color::WHITE.into()),
        font_size: Some(14.0),
        leader_width: Some(2.0),
    }));
    assert!(ink(&render(&film(callout), 0.0)) > 100);

    let chart = Element::new(Content::LineChart(LineChartSpec {
        bounds: [20.0, 10.0, 220.0, 100.0],
        horizontal: ScaleSpec::linear([0.0, 10.0]),
        vertical: ScaleSpec::linear([0.0, 100.0]),
        ticks: Some(4),
        series: vec![SeriesSpec {
            points: vec![
                Some([0.0, 10.0]),
                Some([5.0, 80.0]),
                None,
                Some([9.0, 30.0]),
                Some([10.0, 40.0]),
            ],
            color: Color::rgb(255, 100, 100).into(),
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
            color: Color::rgb(255, 220, 0).into(),
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
    .pivot(PivotSpec::TopLeft);
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
