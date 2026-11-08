//! Round-trip, schema, diagnostics and rendering tests for declarative documents.

use motion_core::{Color, ErrorKind, Time, ValidationError};
use motion_render::{RenderOptions, render_still};
use motion_scene::*;

fn pixel(document: &Document, seconds: f64, x: u32, y: u32) -> [u8; 4] {
    let compiled = document.compile(&CompileOptions::default()).unwrap();
    let bitmap = render_still(
        compiled.timeline(),
        Time::from_seconds(seconds),
        &compiled.render_options(RenderOptions::default()),
    )
    .unwrap();
    let offset = (y as usize * bitmap.size().width as usize + x as usize) * 4;
    bitmap.bytes()[offset..offset + 4].try_into().unwrap()
}

fn errors(document: &Document) -> Vec<ValidationError> {
    let error = document.compile(&CompileOptions::default()).err().unwrap();
    if error.build_errors().is_empty() {
        return vec![error.downcast_ref::<ValidationError>().unwrap().clone()];
    }
    error
        .build_errors()
        .iter()
        .map(|error| error.downcast_ref::<ValidationError>().unwrap().clone())
        .collect()
}

fn showcase() -> Document {
    let orbit = Element::circle(
        Animated::keyframes([
            KeyframeSpec::new(0.0, [20.0, 20.0]),
            KeyframeSpec::new(1.0, [60.0, 20.0]).easing(EasingSpec::BackOut { overshoot: 1.2 }),
        ]),
        NoiseSpec::new(7).amplitude(2.0).offset(8.0),
    )
    .fill(Animated::tween(
        ColorValue::from(Color::BLACK),
        ColorValue::parse("hsl(120 50% 50%)").unwrap(),
        1.0,
    ))
    .stroke(
        StrokeSpec::new(1.5, Color::WHITE)
            .cap(LineCapSpec::Round)
            .dash(vec![2.0, 1.0], 0.5),
    )
    .trim(TrimSpec::new(0.0, Animated::tween(0.0, 1.0, 1.0)))
    .filter(FilterSpec::blur(
        OscillateSpec::new(2.0).amplitude(0.5).offset(1.0),
    ))
    .clip(
        ClipSpec::new(ClipShapeSpec::Rect([0.0, 0.0, 80.0, 40.0].into()))
            .mode(ClipModeSpec::Geometric),
    );
    let group = Element::group([
        Element::rect([0.0, 0.0, 80.0, 40.0]).fill(PaintSpec::Gradient(GradientSpec::Linear(
            LinearGradientSpec::new([0.0, 0.0], [80.0, 0.0])
                .stop(0.0, Color::rgb(10, 20, 30))
                .stop(1.0, Color::rgba(200, 100, 50, 128)),
        ))),
        orbit,
        Element::path("M0 0 L10 0 L10 10 Z")
            .fill(Color::WHITE)
            .fill_rule(FillRuleSpec::EvenOdd)
            .rotate(TweenSpec::new(0.0, 90.0, 1.0).repeat(2).ping_pong())
            .pivot(PivotSpec::Relative([0.5, 0.5]))
            .z_index(-1),
    ])
    .opacity(0.9)
    .blend(BlendModeSpec::Screen)
    .mask(
        Element::rect([0.0, 0.0, 40.0, 40.0]).fill(Color::WHITE),
        MaskModeSpec::Alpha,
    );
    let rate = FrameRateSpec {
        numerator: 30_000,
        denominator: 1001,
    };
    Document::new([80.0, 40.0], rate)
        .name("showcase")
        .background(Color::BLACK)
        .look(LookSpec::Film { seed: 3 })
        .track(
            TrackSpec::new()
                .scene(SceneSpec::new(1.0, group).name("intro"))
                .transition(
                    TransitionEffect::Wipe {
                        angle: 30.0,
                        feather: 4.0,
                    },
                    0.25,
                )
                .scene(SceneSpec::new(1.0, Element::default()))
                .gap(0.5)
                .scene(SceneSpec::new(0.5, Element::default()).start(3.0)),
        )
        .audio(AudioSpec::new("music.wav"))
        .duration(4.0)
}

#[test]
fn documents_round_trip_through_json() {
    let document = showcase();
    let json = document.to_json().unwrap();
    assert_eq!(Document::from_json(&json).unwrap(), document);
    let value: serde_json::Value = serde_json::from_str(&json).unwrap();
    assert_eq!(value["fps"], "30000/1001");
    assert_eq!(value["version"], FORMAT_VERSION);
    assert_eq!(value["background"], "#000000");
    let items = &value["tracks"][0]["items"];
    assert_eq!(items[3]["gap"], 0.5);
    assert_eq!(items[1]["transition"]["effect"]["wipe"]["angle"], 30.0);
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("film.json");
    document.write(&path).unwrap();
    assert_eq!(Document::read(&path).unwrap(), document);
}

#[test]
fn showcase_compiles_and_renders() {
    let compiled = showcase().compile(&CompileOptions::default()).unwrap();
    assert_eq!(compiled.timeline().frame_count(), 120);
    for seconds in [0.0, 0.5, 0.9, 1.5, 3.2] {
        render_still(
            compiled.timeline(),
            Time::from_seconds(seconds),
            &compiled.render_options(RenderOptions::default()),
        )
        .unwrap();
    }
}

#[test]
fn concise_json_forms_parse() {
    let document = Document::from_json(
        r##"{
            "$schema": "./motion.schema.json",
            "version": 1, "size": [10, 10], "fps": 24,
            "look": "cinematic",
            "tracks": [{"items": [
                {"scene": {"duration": 1, "content": {
                    "ring": {"center": [5, 5], "inner_radius": 2, "outer_radius": 4},
                    "fill": "red",
                    "scale": {"tween": {"from": 0, "to": 1, "duration": 1, "easing": {"steps": 4}}}
                }}},
                {"transition": {"effect": {"slide": {"direction": "up"}}, "duration": 0.5}},
                {"scene": {"duration": 1, "content": {}}}
            ]}]
        }"##,
    )
    .unwrap();
    assert_eq!(document.look, Some(LookSpec::Cinematic));
    let TrackItem::Scene(scene) = &document.tracks[0].items[0] else {
        panic!("scene expected");
    };
    assert!(matches!(scene.content.content, Content::Ring(_)));
    assert_eq!(
        scene.content.scale,
        Some(
            TweenSpec::new(0.0, 1.0, 1.0)
                .easing(EasingSpec::Steps(4))
                .into()
        )
    );
    let compiled = document.compile(&CompileOptions::default()).unwrap();
    assert_eq!(compiled.timeline().frame_count(), 36);
    assert!(compiled.look().is_some());
}

#[test]
fn parse_errors_report_json_paths() {
    let error = Document::from_json(
        r#"{"version": 1, "size": [1, 1], "fps": 1, "tracks": [{"items": [
            {"scene": {"duration": 1, "content": {
                "circle": {"center": [0, 0], "radius": 1}, "rect": {"bounds": [0, 0, 1, 1]}}}}]}]}"#,
    )
    .unwrap_err();
    let diagnostic = error.downcast_ref::<ValidationError>().unwrap();
    assert_eq!(diagnostic.path, "tracks[0].items[0].scene.content");
    assert!(diagnostic.message.contains("choose one content key"));

    let error = Document::from_json(r#"{"version": 1, "size": [1, 1], "fps": 1, "colour": 1}"#)
        .unwrap_err();
    assert!(error.to_string().contains("colour"));

    let error =
        Document::from_json(r#"{"version": 1, "size": [1, 1], "fps": 1, "background": "nope"}"#)
            .unwrap_err();
    assert_eq!(
        error.downcast_ref::<ValidationError>().unwrap().path,
        "background"
    );

    let error = Document::from_json(r#"{"version": 2, "size": [1, 1], "fps": 1}"#).unwrap_err();
    assert_eq!(error.kind(), ErrorKind::Validation);
    assert!(error.to_string().contains("version 2"));
}

#[test]
fn compile_errors_are_collected_with_paths() {
    let signal =
        Animated::<ColorValue>::Motion(Box::new(MotionSpec::Oscillate(OscillateSpec::new(1.0))));
    let document = Document::new([10.0, 10.0], 0).track(
        TrackSpec::new().scene(SceneSpec::new(
            -1.0,
            Element::group([
                Element::circle([0.0, 0.0], f64::NAN).fill(Color::WHITE),
                Element::rect([0.0, 0.0, 1.0, 1.0]),
                Element::default().fill(Color::WHITE),
                Element::rect([0.0, 0.0, 1.0, 1.0])
                    .fill(Color::WHITE)
                    .opacity(Animated::keyframes([
                        KeyframeSpec::new(1.0, 0.0),
                        KeyframeSpec::new(1.0, 1.0),
                    ])),
                Element::rect([0.0, 0.0, 1.0, 1.0]).fill(signal),
            ]),
        )),
    );
    let paths: Vec<_> = errors(&document)
        .into_iter()
        .map(|error| format!("{}:{}", error.path, error.field))
        .collect();
    let content = "tracks[0].items[0].scene.content.children";
    for expected in [
        "fps:fps".to_string(),
        "tracks[0].items[0].scene:duration".into(),
        format!("{content}[0].circle.radius:value"),
        format!("{content}[1]:fill"),
        format!("{content}[2]:fill"),
        format!("{content}[3].opacity.keyframes.keys[1]:time"),
        format!("{content}[4].fill.oscillate:motion"),
    ] {
        assert!(
            paths.contains(&expected),
            "{expected} missing from {paths:?}"
        );
    }
}

#[test]
fn compiled_documents_render_animated_properties() {
    let document = Document::new([40.0, 20.0], 10).track(
        TrackSpec::new().scene(SceneSpec::new(
            2.0,
            Element::group([
                Element::rect([0.0, 0.0, 40.0, 20.0]).fill(Color::BLACK),
                Element::rect([0.0, 0.0, 10.0, 20.0])
                    .fill(Color::WHITE)
                    .translate(Animated::tween([0.0, 0.0], [30.0, 0.0], 2.0)),
            ]),
        )),
    );
    assert_eq!(pixel(&document, 0.0, 5, 10), [255, 255, 255, 255]);
    assert_eq!(pixel(&document, 0.0, 35, 10), [0, 0, 0, 255]);
    assert_eq!(pixel(&document, 1.9, 35, 10), [255, 255, 255, 255]);
    assert_eq!(pixel(&document, 1.9, 5, 10), [0, 0, 0, 255]);
}

#[test]
fn transitions_gaps_and_placement_build_the_timeline() {
    let solid = |color: Color| Element::rect([0.0, 0.0, 4.0, 4.0]).fill(color);
    let document = Document::new([4.0, 4.0], 4).background(Color::BLACK).track(
        TrackSpec::new()
            .scene(SceneSpec::new(1.0, solid(Color::rgb(255, 0, 0))))
            .transition(TransitionEffect::CrossFade, 0.5)
            .scene(SceneSpec::new(1.0, solid(Color::rgb(0, 0, 255))))
            .gap(1.0)
            .scene(SceneSpec::new(0.5, solid(Color::WHITE)).start(3.0)),
    );
    let compiled = document.compile(&CompileOptions::default()).unwrap();
    assert_eq!(
        compiled.timeline().total_duration(),
        Time::from_seconds(3.5)
    );
    assert_eq!(pixel(&document, 0.25, 1, 1), [255, 0, 0, 255]);
    assert_eq!(pixel(&document, 1.25, 1, 1), [0, 0, 255, 255]);
    assert_eq!(pixel(&document, 2.75, 1, 1), [0, 0, 0, 255]);
    assert_eq!(pixel(&document, 3.25, 1, 1), [255, 255, 255, 255]);
}

#[test]
fn transition_overlap_errors_are_located() {
    let document = Document::new([4.0, 4.0], 4).track(
        TrackSpec::new()
            .scene(SceneSpec::new(0.25, Element::default()))
            .transition(TransitionEffect::ZoomBlur, 1.0)
            .scene(SceneSpec::new(1.0, Element::default())),
    );
    let error = document.compile(&CompileOptions::default()).unwrap_err();
    let diagnostic = error.downcast_ref::<ValidationError>().unwrap();
    assert!(
        diagnostic.path.starts_with("tracks[0].items["),
        "{diagnostic:?}"
    );
}

#[test]
fn resource_paths_reject_traversal_and_resolve_against_base() {
    let document = Document::new([4.0, 4.0], 4)
        .track(TrackSpec::new().scene(SceneSpec::new(1.0, Element::default())))
        .audio(AudioSpec::new("../secret.wav"));
    assert_eq!(errors(&document)[0].path, "audio[0].source");
}

/// Returns the schema inside the `anyOf` wrapper that admits `$param` references.
fn unwrap(schema: &serde_json::Value) -> &serde_json::Value {
    &schema["anyOf"][0]
}

#[test]
fn schema_describes_every_public_shape() {
    let schema = schema();
    let definitions = schema["$defs"].as_object().unwrap();
    for name in [
        "Element",
        "Paint",
        "Color",
        "FrameRate",
        "TrackItem",
        "FilterSpec",
        "EasingSpec",
        "ParamRef",
    ] {
        assert!(definitions.contains_key(name), "{name} missing");
    }
    let properties = unwrap(&definitions["Element"])["properties"]
        .as_object()
        .unwrap();
    for key in [
        "id",
        "children",
        "rect",
        "text",
        "image",
        "svg",
        "particles",
        "trajectory",
        "ruler",
        "callout",
        "line_chart",
        "captions",
        "formula",
        "morph",
        "svg_morph",
        "fill",
        "filters",
        "mask",
    ] {
        assert!(properties.contains_key(key), "{key} missing");
    }
    assert_eq!(unwrap(&schema["properties"]["version"])["type"], "integer");
    assert!(schema["properties"]["params"].is_object());
}

#[cfg(feature = "text")]
#[test]
fn text_documents_bind_document_fonts() {
    let fonts = concat!(env!("CARGO_MANIFEST_DIR"), "/../../tests/fixtures/fonts");
    let document = Document::new([120.0, 40.0], 10)
        .font("NotoSans-Regular.ttf")
        .track(
            TrackSpec::new().scene(SceneSpec::new(
                1.0,
                Element::text(TextSpec::new("Hello").family("Noto Sans").size(24.0))
                    .fill(Color::WHITE)
                    .translate([4.0, 30.0]),
            )),
        );
    let options = CompileOptions::default().base_dir(fonts);
    let compiled = document.compile(&options).unwrap();
    let bitmap = render_still(compiled.timeline(), Time::ZERO, &RenderOptions::default()).unwrap();
    assert!(bitmap.bytes().chunks(4).any(|pixel| pixel[3] > 200));
    let missing = document.compile(&CompileOptions::default().base_dir("/nonexistent"));
    assert!(missing.is_err());
}
