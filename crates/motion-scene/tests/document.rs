//! Round-trip, schema, diagnostics and rendering tests for declarative documents.

use motion_content::{CollisionResponse, Fit, ParticleEventKind, WriteOptions};
use motion_core::{
    BlendMode, ClipMode, Color, ErrorKind, FillRule, LineCap, LineJoin, MaskMode, Pivot, Point,
    Rect, Time, ValidationError,
};
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
    error
        .errors()
        .iter()
        .map(|error| error.downcast_ref::<ValidationError>().unwrap().clone())
        .collect()
}

#[test]
fn shared_content_policies_preserve_json_without_content_features() {
    use serde_json::json;

    let source = json!({"version": 1, "size": [8, 8], "fps": 4, "tracks": [{
        "items": [{"scene": {"duration": 1, "content": {"children": [
            {"image": {"source": "image.png", "fit": "cover"}},
            {"text": {"content": "Shared", "alignment": "center", "split": "words"}},
            {"svg": {"source": "drawing.svg", "write_options": {
                "width": 2, "outline_fraction": 0.75
            }}},
            {"particles": {"seed": 7, "blend": "screen", "duration": 1,
                "preset": {"kind": "logo_assemble", "bounds": [0, 0, 8, 8]}
            }},
            {"particles": {"seed": 8, "sub_emitters": [{
                "on": {"age_reached": 0.5}, "emitter": {"point": [0, 0]},
                "inherit": "all", "probability": 1
            }], "modifiers": [{"operator": {"color_field": {
                "coordinate": "speed", "range": [0, 10],
                "colors": [[0, 0, 0, 1], [1, 1, 1, 1]]
            }}}]}}
        ]}}}]
    }]});
    let document = Document::from_json(&source.to_string()).unwrap();
    let output = document.to_json().unwrap();
    assert_eq!(Document::from_json(&output).unwrap(), document);
    let output: serde_json::Value = serde_json::from_str(&output).unwrap();
    let children = &output["tracks"][0]["items"][0]["scene"]["content"]["children"];
    assert_eq!(children[0]["image"]["fit"], "cover");
    assert_eq!(children[1]["text"]["alignment"], "center");
    assert_eq!(children[1]["text"]["split"], "words");
    assert_eq!(
        children[2]["svg"]["write_options"],
        json!({
            "width": 2.0, "outline_fraction": 0.75
        })
    );
    assert_eq!(children[3]["particles"]["preset"]["kind"], "logo_assemble");
    assert_eq!(children[3]["particles"]["blend"], "screen");
    let event = &children[4]["particles"]["sub_emitters"][0];
    assert_eq!(event["on"], json!({"age_reached": 0.5}));
    assert_eq!(event["inherit"], "all");
}

#[test]
fn shared_content_options_keep_required_fields_and_reject_unknown_fields() {
    use serde_json::json;

    for value in [
        json!({"restitution": 0.5, "kill_on_hit": false}),
        json!({"restitution": 0.5, "friction": 0.25, "kill_on_hit": false, "extra": 1}),
    ] {
        assert!(serde_json::from_value::<CollisionResponse>(value).is_err());
    }
    for value in [
        json!({"width": 2}),
        json!({"width": 2, "outline_fraction": 0.75, "extra": 1}),
    ] {
        assert!(serde_json::from_value::<WriteOptions>(value).is_err());
    }
    assert!(serde_json::from_value::<Fit>(json!("stretch")).is_err());
    assert!(serde_json::from_value::<ParticleEventKind>(json!("age_reached")).is_err());
}

#[test]
fn atlas_gradients_animate_replay_and_reject_invalid_geometry() {
    let make = |gradient| {
        Document::new([16.0, 16.0], 30).track(TrackSpec::new().scene(SceneSpec::new(
            2.0,
            Element::rect([0.0, 0.0, 16.0, 16.0]).fill(PaintSpec::Gradient(gradient)),
        )))
    };
    let mut linear = LinearGradientSpec::new(Point::new(0.0, 0.0), Point::new(8.0, 0.0))
        .stop(0.0, Color::BLACK)
        .stop(1.0, Color::WHITE);
    linear.end = Animated::exprs(["8 + t * 8", "0"]);
    linear.options.units = Some(motion_core::GradientUnits::UserSpace);
    let mut radial = RadialGradientSpec::new(Point::new(8.0, 8.0), 8.0)
        .stop(0.0, Color::BLACK)
        .stop(1.0, Color::WHITE);
    radial.radius = Animated::expr("8 + t * 8");
    radial.options.units = Some(motion_core::GradientUnits::UserSpace);
    for gradient in [GradientSpec::Linear(linear), GradientSpec::Radial(radial)] {
        let document = make(gradient);
        assert_eq!(
            Document::from_json(&document.to_json().unwrap()).unwrap(),
            document
        );
        assert_ne!(pixel(&document, 0.0, 4, 4), pixel(&document, 1.0, 4, 4));
        assert_eq!(pixel(&document, 0.5, 4, 4), pixel(&document, 0.5, 4, 4));
    }
    let mut sweep = SweepGradientSpec::new(Point::new(0.5, 0.5), 0.0, 360.0)
        .stop(0.0, Color::BLACK)
        .stop(1.0, Color::WHITE);
    sweep.start_angle = Animated::expr("degrees(2.8 + t)");
    sweep.end_angle = Animated::expr("degrees(2.8 + t + tau)");
    let document = make(GradientSpec::Sweep(sweep.clone()));
    for time in [0.0, 0.25, 0.5, 1.0] {
        pixel(&document, time, 4, 4);
    }
    sweep.end_angle = Animated::expr("degrees(2.8 + t + tau) + 0.000001");
    assert!(!errors(&make(GradientSpec::Sweep(sweep))).is_empty());
    let mut invalid = RadialGradientSpec::new(Point::new(0.5, 0.5), 1.0)
        .stop(0.0, Color::BLACK)
        .stop(1.0, Color::WHITE);
    invalid.radius = Animated::expr("1 - t");
    let compiled = make(GradientSpec::Radial(invalid))
        .compile(&CompileOptions::default())
        .unwrap();
    assert!(
        render_still(
            compiled.timeline(),
            Time::from_seconds(1.5),
            &RenderOptions::default()
        )
        .is_err()
    );
}

#[test]
fn atlas_animated_points_match_fixed_geometry_and_enforce_limits() {
    let make = |points| {
        Document::new([16.0, 16.0], 30).track(TrackSpec::new().scene(SceneSpec::new(
            2.0,
            Element::polyline(points).stroke(StrokeSpec::new(2.0, Color::WHITE)),
        )))
    };
    let mut points = PointsSpec::new([]);
    points.animated = Some(vec![
        Animated::exprs(["2 + t * 4", "2"]),
        [10.0, 10.0].into(),
    ]);
    let document = make(points.clone());
    let compiled = document.compile(&CompileOptions::default()).unwrap();
    assert_eq!(
        Document::from_json(&document.to_json().unwrap()).unwrap(),
        document
    );
    for seconds in [1.0, 0.0, 0.5, 1.0] {
        let fixed = make(PointsSpec::new([
            Point::new(2.0 + seconds * 4.0, 2.0),
            Point::new(10.0, 10.0),
        ]))
        .compile(&CompileOptions::default())
        .unwrap();
        let time = Time::from_seconds(seconds);
        assert_eq!(
            render_still(compiled.timeline(), time, &RenderOptions::default())
                .unwrap()
                .bytes(),
            render_still(fixed.timeline(), time, &RenderOptions::default())
                .unwrap()
                .bytes()
        );
    }
    let mut mixed = points.clone();
    mixed.points = vec![Point::new(0.0, 0.0)];
    assert!(!errors(&make(mixed)).is_empty());
    let mut sampled = points.clone();
    sampled.sample = Some(SampledPointsSpec::new(
        Animated::exprs(["t", "t"]),
        [0.0, 1.0],
        2,
    ));
    assert!(!errors(&make(sampled)).is_empty());
    for count in [0, 1, 65_537] {
        let mut invalid = points.clone();
        invalid.animated = Some(vec![[0.0, 0.0].into(); count]);
        assert!(!errors(&make(invalid)).is_empty());
    }
    let polygon = Document::new([16.0, 16.0], 30).track(TrackSpec::new().scene(SceneSpec::new(
        1.0,
        Element::new(Content::Polygon(points)).fill(Color::WHITE),
    )));
    assert!(!errors(&polygon).is_empty());
}

#[test]
fn atlas_optical_filters_preserve_core_validation() {
    use serde_json::json;
    for spec in [
        json!({"radial_blur":{"center":[8,8],"strength":0.5,"samples":0,"decay":0.9}}),
        json!({"radial_blur":{"center":[8,8],"strength":0.5,"samples":65,"decay":0.9}}),
        json!({"lens_flare":{"center":[8,8],"ghosts":9,"threshold":0.5,"intensity":1,"tint":"#fff"}}),
        json!({"bokeh":{"radius":2,"shape":{"polygon":{"sides":2,"rotation":0}},"highlight_boost":1}}),
        json!({"relight":{"height_from":"luma","light_dir":[0,0,0],"color":"#fff","ambient":0.2,"specular":1}}),
    ] {
        let filter = serde_json::from_value(json!({"optics":spec})).unwrap();
        let document = Document::new([16.0, 16.0], 30).track(
            TrackSpec::new().scene(SceneSpec::new(
                1.0,
                Element::rect([0.0, 0.0, 16.0, 16.0])
                    .fill(Color::WHITE)
                    .filter(filter),
            )),
        );
        assert!(
            errors(&document)
                .iter()
                .any(|error| error.path.contains("filters[0].optics"))
        );
    }
    let document = Document::new([16.0, 16.0], 30).track(
        TrackSpec::new().scene(SceneSpec::new(
            1.0,
            Element::rect([0.0, 0.0, 16.0, 16.0])
                .fill(Color::WHITE)
                .filter(FilterSpec::Halation {
                    strength: (-1.0).into(),
                    radius: 2.0,
                }),
        )),
    );
    let errors = errors(&document);
    assert!(
        errors
            .iter()
            .any(|error| error.path.ends_with("filters[0].halation") && error.field == "parameters"),
        "{errors:?}"
    );
}

#[test]
fn animated_filters_fail_the_frame_where_they_become_invalid() {
    let make = |filter| {
        Document::new([16.0, 16.0], 30).track(
            TrackSpec::new().scene(SceneSpec::new(
                2.0,
                Element::rect([0.0, 0.0, 16.0, 16.0])
                    .fill(Color::WHITE)
                    .filter(filter),
            )),
        )
    };
    for filter in [
        FilterSpec::BlurXy {
            sigma_x: Animated::expr("1 - t"),
            sigma_y: 1.0.into(),
        },
        FilterSpec::Grain {
            seed: 7,
            strength: Animated::expr("0.5 - t"),
        },
    ] {
        let compiled = make(filter).compile(&CompileOptions::default()).unwrap();
        let render = |seconds| {
            render_still(
                compiled.timeline(),
                Time::from_seconds(seconds),
                &RenderOptions::default(),
            )
        };
        render(0.25).unwrap();
        let error = render(1.5).unwrap_err().to_string();
        assert!(error.contains("filters[0]"), "{error}");
    }
    let invalid = make(FilterSpec::BlurXy {
        sigma_x: (-1.0).into(),
        sigma_y: 1.0.into(),
    });
    assert!(
        errors(&invalid)
            .iter()
            .any(|error| error.path.contains("filters[0]"))
    );
}

#[cfg(feature = "svg")]
#[test]
fn atlas_inline_svg_writes_morphs_and_validates_without_files() {
    use serde_json::json;
    let source = json!({"data":r##"<svg xmlns="http://www.w3.org/2000/svg" width="16" height="16"><rect id="mark" x="2" y="2" width="12" height="12" fill="#fff"/></svg>"##});
    let target = json!({"data":r##"<svg xmlns="http://www.w3.org/2000/svg" width="16" height="16"><rect id="mark" x="6" y="6" width="4" height="4" fill="#fff"/></svg>"##});
    let make = |element: serde_json::Value| {
        Document::new([16.0, 16.0], 30).track(TrackSpec::new().scene(SceneSpec::new(
            2.0,
            serde_json::from_value::<Element>(element).unwrap(),
        )))
    };
    let document = make(
        json!({"svg":{"source":source,"write":{"expr":"t"},"write_options":{"width":3,"outline_fraction":0.8}}}),
    );
    assert_eq!(
        Document::from_json(&document.to_json().unwrap()).unwrap(),
        document
    );
    let compiled = document.compile(&CompileOptions::default()).unwrap();
    assert!(compiled.resources().is_empty());
    assert_eq!(pixel(&document, 0.0, 8, 8), [0, 0, 0, 0]);
    assert_eq!(pixel(&document, 1.0, 8, 8), [255, 255, 255, 255]);
    assert_eq!(pixel(&document, 0.5, 2, 2), pixel(&document, 0.5, 2, 2));
    let morph = make(json!({"svg_morph":{"from":source,"to":target,"progress":{"expr":"t"}}}));
    assert!(
        morph
            .compile(&CompileOptions::default())
            .unwrap()
            .resources()
            .is_empty()
    );
    assert_ne!(pixel(&morph, 0.0, 3, 3), pixel(&morph, 1.0, 3, 3));
    for options in [
        json!({"width":0,"outline_fraction":0.8}),
        json!({"width":3,"outline_fraction":1.1}),
    ] {
        assert!(
            !errors(&make(
                json!({"svg":{"source":source,"write":0.5,"write_options":options}})
            ))
            .is_empty()
        );
    }
    assert!(!errors(&make(json!({"svg":{"source":{"data":"not SVG"}}}))).is_empty());
    for (source, expected) in [
        (json!(3), "an SVG file path string or an object"),
        (json!({"path": "a.svg"}), "unknown field `path`"),
        (json!({}), "missing field `data`"),
    ] {
        let error = serde_json::from_value::<Element>(json!({"svg": {"source": source}}))
            .unwrap_err()
            .to_string();
        assert!(error.contains(expected), "{error}");
    }
}

#[test]
fn sampled_polylines_compile_once_and_match_fixed_points() {
    let element = |points| Element::polyline(points).stroke(StrokeSpec::new(1.0, Color::WHITE));
    let make = |points| {
        Document::new([16.0, 16.0], 30)
            .background(Color::BLACK)
            .track(TrackSpec::new().scene(SceneSpec::new(1.0, element(points))))
    };
    let sampled = PointsSpec::sampled(SampledPointsSpec::new(
        Animated::components([Animated::expr("2 + t * 8"), Animated::expr("2 + t * 8")]),
        [0.0, 1.0],
        2,
    ));
    let document = make(sampled.clone());
    let fixed = make(PointsSpec::new([
        Point::new(2.0, 2.0),
        Point::new(6.0, 6.0),
        Point::new(10.0, 10.0),
    ]));
    assert_eq!(
        Document::from_json(&document.to_json().unwrap()).unwrap(),
        document
    );
    for time in [0.0, 0.5] {
        assert_eq!(pixel(&document, time, 6, 6), pixel(&fixed, time, 6, 6));
        assert_ne!(pixel(&document, time, 6, 6), [0, 0, 0, 255]);
    }
    for segments in [0, 65536] {
        let mut broken = sampled.clone();
        broken.sample.as_mut().unwrap().segments = segments;
        assert!(!errors(&make(broken)).is_empty());
    }
    let mut mixed = sampled.clone();
    mixed.points = vec![Point::new(0.0, 0.0)];
    assert!(!errors(&make(mixed)).is_empty());
    let mut invalid = sampled;
    invalid.sample.as_mut().unwrap().source = Animated::exprs(["t", "1 / 0"]);
    assert!(!errors(&make(invalid)).is_empty());
}

#[test]
fn rounded_rect_clips_round_trip_animate_and_replay() {
    let make = |spec, mode| {
        Document::new([16.0, 16.0], 30)
            .background(Color::BLACK)
            .track(
                TrackSpec::new().scene(SceneSpec::new(
                    2.0,
                    Element::rect([0.0, 0.0, 16.0, 16.0])
                        .fill(Color::WHITE)
                        .clip(ClipSpec::new(ClipShapeSpec::RoundedRect(spec)).mode(mode)),
                )),
            )
    };
    for mode in [ClipMode::Coverage, ClipMode::Geometric] {
        let document = make(
            RectSpec::new(Animated::tween(
                Rect::new(0.0, 0.0, 16.0, 16.0),
                Rect::new(4.0, 4.0, 12.0, 12.0),
                1.0,
            ))
            .corner_radius(Animated::tween(0.0, 4.0, 1.0)),
            mode,
        );
        assert_eq!(
            Document::from_json(&document.to_json().unwrap()).unwrap(),
            document
        );
        let compiled = document.compile(&CompileOptions::default()).unwrap();
        let options = compiled.render_options(RenderOptions::default());
        for seconds in [1.0, 0.0, 0.5, 1.0] {
            let inset = seconds * 4.0;
            let fixed = make(
                RectSpec::new([inset, inset, 16.0 - inset, 16.0 - inset]).corner_radius(inset),
                mode,
            )
            .compile(&CompileOptions::default())
            .unwrap();
            let time = Time::from_seconds(seconds);
            let actual = render_still(compiled.timeline(), time, &options).unwrap();
            let expected = render_still(fixed.timeline(), time, &options).unwrap();
            assert_eq!(actual.bytes(), expected.bytes());
            let replay = render_still(compiled.timeline(), time, &options).unwrap();
            assert_eq!(actual.bytes(), replay.bytes());
        }
        assert_eq!(pixel(&document, 0.0, 0, 0), [255, 255, 255, 255]);
        assert_eq!(pixel(&document, 1.0, 4, 4), [0, 0, 0, 255]);
        assert_eq!(pixel(&document, 1.0, 8, 8), [255, 255, 255, 255]);
        let square = make(RectSpec::new([0.0, 0.0, 16.0, 16.0]), mode);
        assert_eq!(pixel(&square, 0.0, 0, 0), [255, 255, 255, 255]);
    }
    let invalid = make(
        RectSpec::new([0.0, 0.0, 16.0, 16.0]).corner_radius(-1.0),
        ClipMode::Geometric,
    );
    let compiled = invalid.compile(&CompileOptions::default()).unwrap();
    assert!(render_still(compiled.timeline(), Time::ZERO, &RenderOptions::default()).is_err());
    let nonfinite = make(
        RectSpec::new([0.0, 0.0, 16.0, 16.0]).corner_radius(f64::NAN),
        ClipMode::Geometric,
    );
    assert!(
        errors(&nonfinite)
            .iter()
            .any(|error| error.path.ends_with("rounded_rect.corner_radius"))
    );
}

fn showcase() -> Document {
    let orbit = Element::circle(
        Animated::keyframes([
            KeyframeSpec::new(0.0, Point::new(20.0, 20.0)),
            KeyframeSpec::new(1.0, Point::new(60.0, 20.0))
                .easing(EasingSpec::BackOut { overshoot: 1.2 }),
        ]),
        NoiseSpec::new(7).amplitude(2.0).offset(8.0),
    )
    .fill(Animated::tween(
        Color::BLACK,
        Color::parse("hsl(120 50% 50%)").unwrap(),
        1.0,
    ))
    .stroke(
        StrokeSpec::new(1.5, Color::WHITE)
            .cap(LineCap::Round)
            .dash(vec![2.0, 1.0], 0.5),
    )
    .trim(TrimSpec::new(0.0, Animated::tween(0.0, 1.0, 1.0)))
    .filter(FilterSpec::blur(
        OscillateSpec::new(2.0).amplitude(0.5).offset(1.0),
    ))
    .clip(
        ClipSpec::new(ClipShapeSpec::Rect([0.0, 0.0, 80.0, 40.0].into())).mode(ClipMode::Geometric),
    );
    let group = Element::group([
        Element::rect([0.0, 0.0, 80.0, 40.0]).fill(PaintSpec::Gradient(GradientSpec::Linear(
            LinearGradientSpec::new(Point::new(0.0, 0.0), Point::new(80.0, 0.0))
                .stop(0.0, Color::rgb(10, 20, 30))
                .stop(1.0, Color::rgba(200, 100, 50, 128)),
        ))),
        orbit,
        Element::path("M0 0 L10 0 L10 10 Z")
            .fill(Color::WHITE)
            .fill_rule(FillRule::EvenOdd)
            .rotate(TweenSpec::new(0.0, 90.0, 1.0).repeat(2).ping_pong())
            .pivot(Pivot::Relative(0.5, 0.5))
            .z_index(-1),
    ])
    .opacity(0.9)
    .blend(BlendMode::Screen)
    .mask(
        Element::rect([0.0, 0.0, 40.0, 40.0]).fill(Color::WHITE),
        MaskMode::Alpha,
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
    let options = compiled
        .render_options(RenderOptions::default().backend(motion_render::BackendPreference::Cpu));
    for seconds in [0.0, 0.5, 0.9, 1.5, 3.2] {
        render_still(compiled.timeline(), Time::from_seconds(seconds), &options).unwrap();
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
fn core_value_types_define_the_json_forms() {
    let document = Document::from_json(
        r##"{
            "version": 1, "size": [100, 100], "fps": 24, "background": "#0a141e80",
            "tracks": [{"items": [{"scene": {"duration": 1, "content": {
                "line": {"from": [10, 10], "to": [90, 90]},
                "stroke": {"width": 4, "paint": {"linear": {"start": [0, 0], "end": [100, 0],
                    "stops": [{"offset": 0, "color": "white"}, {"offset": 1, "color": "#14a082"}]}},
                    "join": {"miter": {"limit": 2}}},
                "pivot": {"point": [5, 6]}
            }}}]}]
        }"##,
    )
    .unwrap();
    let TrackItem::Scene(scene) = &document.tracks[0].items[0] else {
        panic!("scene expected");
    };
    let element = &scene.content;
    assert_eq!(element.pivot, Some(Pivot::Point(Point::new(5.0, 6.0))));
    assert_eq!(
        element.stroke.as_ref().unwrap().join,
        Some(LineJoin::Miter { limit: 2.0 })
    );
    assert_eq!(document.background, Some(Color::rgba(10, 20, 30, 128)));
    let json: serde_json::Value = serde_json::from_str(&document.to_json().unwrap()).unwrap();
    let element = &json["tracks"][0]["items"][0]["scene"]["content"];
    assert_eq!(element["pivot"], serde_json::json!({"point": [5.0, 6.0]}));
    assert_eq!(element["line"]["from"], serde_json::json!([10.0, 10.0]));
    assert_eq!(
        element["stroke"]["paint"]["linear"]["stops"][0]["color"],
        "#ffffff"
    );
    assert_eq!(
        Document::from_json(&document.to_json().unwrap()).unwrap(),
        document
    );
    document.compile(&CompileOptions::default()).unwrap();
}

#[test]
fn stroke_joins_are_checked_by_the_core_stroke_style() {
    let element = Element::line([0.0, 0.0], [10.0, 10.0])
        .stroke(StrokeSpec::new(1.0, Color::WHITE).join(LineJoin::Miter { limit: 0.5 }));
    let document =
        Document::new([10.0, 10.0], 24).track(TrackSpec::new().scene(SceneSpec::new(1.0, element)));
    let errors = errors(&document);
    assert_eq!(errors.len(), 1);
    assert_eq!(errors[0].path, "tracks[0].items[0].scene.content.stroke");
    assert!(errors[0].message.contains("miter limit"));
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
        Animated::<Color>::Motion(Box::new(MotionSpec::Oscillate(OscillateSpec::new(1.0))));
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
        compiled.timeline().duration(),
        motion_core::Duration::from_seconds(3.5)
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
    let diagnostic = error.errors()[0].downcast_ref::<ValidationError>().unwrap();
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
fn particle_schema_constrains_static_inputs() {
    let schema = schema();
    let preset = &unwrap(&schema["$defs"]["ParticlePresetSpec"])["properties"];
    for field in ["density", "speed"] {
        assert_eq!(unwrap(&preset[field])["exclusiveMinimum"], 0);
        assert_eq!(unwrap(&preset[field])["maximum"], 16.0);
    }
    assert_eq!(unwrap(&preset["intensity"])["minimum"], 0.0);
    assert_eq!(unwrap(&preset["intensity"])["maximum"], 64.0);
    assert_eq!(unwrap(&preset["logo"])["maxItems"], 256);
    let particles = &unwrap(&schema["$defs"]["ParticlesSpec"])["properties"];
    assert_eq!(unwrap(&particles["duration"])["exclusiveMinimum"], 0);
    assert_eq!(unwrap(&particles["step"])["exclusiveMinimum"], 0);
    assert_eq!(unwrap(&particles["max_particles"])["minimum"], 1);
    assert_eq!(unwrap(&particles["max_particles"])["maximum"], 1_000_000);
    assert_eq!(unwrap(&particles["sample_time"])["minimum"], 0.0);
    assert_eq!(unwrap(&particles["sample_time"])["maximum"], 1e6);
    let conditional = &unwrap(&schema["$defs"]["ParticlesSpec"])["allOf"][0];
    assert_eq!(
        conditional["if"]["properties"]["preset"]["required"],
        serde_json::json!(["kind"])
    );
    assert_eq!(
        conditional["then"]["required"],
        serde_json::json!(["duration"])
    );
    assert_eq!(
        conditional["then"]["properties"]["duration"]["maximum"],
        3600
    );
    assert_eq!(unwrap(&schema["properties"]["version"])["minimum"], 1);
    assert_eq!(unwrap(&schema["properties"]["version"])["maximum"], 1);
    assert_eq!(
        unwrap(&schema["properties"]["duration"])["exclusiveMinimum"],
        0
    );
    assert_eq!(unwrap(&schema["properties"]["duration"])["maximum"], 1e6);
    let scene = &unwrap(&schema["$defs"]["SceneSpec"])["properties"];
    assert_eq!(unwrap(&scene["duration"])["exclusiveMinimum"], 0);
    assert_eq!(unwrap(&scene["duration"])["maximum"], 1e6);
    assert_eq!(unwrap(&scene["start"])["minimum"], 0.0);
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
        "UseRef",
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
    assert!(!properties.contains_key("switch"));
    assert!(!definitions.contains_key("SwitchSpec"));
    assert!(serde_json::from_str::<Element>(r#"{"switch":{"index":0,"cases":[]}}"#).is_err());
    assert_eq!(unwrap(&schema["properties"]["version"])["type"], "integer");
    assert!(schema["properties"]["params"].is_object());
    assert_eq!(schema["properties"]["defs"]["type"], "object");
    assert_eq!(schema["properties"]["defs"]["maxProperties"], 256);
    assert_eq!(
        schema["properties"]["background"]["anyOf"][1]["$ref"],
        "#/$defs/ValueRef"
    );
    assert_eq!(
        definitions["ValueRef"]["anyOf"],
        serde_json::json!([
            {"$ref":"#/$defs/ParamRef"},
            {"$ref":"#/$defs/UseRef"},
            {"$ref":"#/$defs/CalcRef"}
        ])
    );
    assert_eq!(
        definitions["UseRef"]["required"],
        serde_json::json!(["use"])
    );
}

#[test]
fn reference_keys_are_reserved_in_every_document_shape() {
    fn visit(schema: &serde_json::Value, path: &str, found: &mut Vec<String>) {
        match schema {
            serde_json::Value::Object(object) => {
                if let Some(properties) =
                    object.get("properties").and_then(|value| value.as_object())
                {
                    for key in properties.keys() {
                        if ["use", "$param", "$calc"].contains(&key.as_str()) {
                            found.push(format!("{path}.{key}"));
                        }
                    }
                }
                for (key, child) in object {
                    visit(child, &format!("{path}.{key}"), found);
                }
            }
            serde_json::Value::Array(items) => {
                for (index, child) in items.iter().enumerate() {
                    visit(child, &format!("{path}[{index}]"), found);
                }
            }
            _ => {}
        }
    }

    let mut schema = schema();
    let definitions = schema["$defs"].as_object_mut().unwrap();
    for reference in ["ParamRef", "UseRef", "CalcRef"] {
        assert!(
            definitions.remove(reference).is_some(),
            "{reference} missing"
        );
    }
    let mut found = Vec::new();
    visit(&schema, "schema", &mut found);
    assert!(
        found.is_empty(),
        "reserved reference keys used as fields: {found:?}"
    );
}

#[test]
fn scenes_select_content_and_validate_every_scene() {
    let mut document = Document::new([4.0, 4.0], 2).track(
        TrackSpec::new()
            .scene(SceneSpec::new(
                1.0,
                Element::rect([0.0, 0.0, 4.0, 4.0]).fill(Color::WHITE),
            ))
            .scene(SceneSpec::new(
                1.0,
                Element::rect([0.0, 0.0, 4.0, 4.0]).fill(Color::BLACK),
            )),
    );
    assert_eq!(pixel(&document, 0.5, 1, 1), [255, 255, 255, 255]);
    assert_eq!(pixel(&document, 1.5, 1, 1), [0, 0, 0, 255]);
    assert_eq!(
        Document::from_json(&document.to_json().unwrap()).unwrap(),
        document
    );
    let TrackItem::Scene(scene) = &mut document.tracks[0].items[1] else {
        panic!("scene expected")
    };
    scene.content = Element::circle([0.0, 0.0], f64::NAN).fill(Color::WHITE);
    assert!(
        errors(&document)
            .iter()
            .any(|error| error.path.contains("items[1].scene.content.circle.radius"))
    );
}

#[cfg(feature = "text")]
#[test]
fn numeric_text_enforces_format_limits_and_rejects_mixed_content() {
    let readout = |precision, prefix: String, content: &str| {
        let mut spec = TextSpec::new(content);
        spec.number = Some(NumberTextSpec {
            value: Animated::expr("t"),
            precision,
            prefix,
            suffix: String::new(),
        });
        Document::new([40.0, 20.0], 2).track(
            TrackSpec::new().scene(SceneSpec::new(2.0, Element::text(spec).fill(Color::WHITE))),
        )
    };
    assert!(
        readout(3, "t ".into(), "")
            .compile(&Default::default())
            .is_ok()
    );
    for document in [
        readout(17, String::new(), ""),
        readout(3, "x".repeat(4097), ""),
        readout(3, String::new(), "mixed"),
    ] {
        assert!(!errors(&document).is_empty());
    }
}

#[cfg(feature = "text")]
#[test]
fn constant_numeric_text_renders_like_its_formatted_content() {
    let fonts = concat!(env!("CARGO_MANIFEST_DIR"), "/../../tests/fixtures/fonts");
    let render = |mut spec: TextSpec, number: Option<NumberTextSpec>| {
        spec.number = number;
        let document = Document::new([120.0, 40.0], 10)
            .font("NotoSans-Regular.ttf")
            .track(
                TrackSpec::new().scene(SceneSpec::new(
                    1.0,
                    Element::text(spec.family("Noto Sans").size(20.0))
                        .fill(Color::WHITE)
                        .translate([4.0, 30.0]),
                )),
            );
        let compiled = document
            .compile(&CompileOptions::default().base_dir(fonts))
            .unwrap();
        render_still(
            compiled.timeline(),
            Time::from_seconds(0.5),
            &RenderOptions::default(),
        )
        .unwrap()
    };
    let readout = NumberTextSpec {
        value: 12.3456.into(),
        precision: 2,
        prefix: "n ".into(),
        suffix: "!".into(),
    };
    let number = render(TextSpec::new(""), Some(readout));
    let content = render(TextSpec::new("n 12.35!"), None);
    assert!(number.bytes().chunks(4).any(|pixel| pixel[3] > 200));
    assert_eq!(number.bytes(), content.bytes());
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
