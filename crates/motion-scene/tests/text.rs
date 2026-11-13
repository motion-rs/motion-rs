//! Rich text, parts and reveal in declarative documents.
#![cfg(feature = "text")]

use motion_core::{Color, Time, ValidationError};
use motion_render::{FrameBuffer, RenderOptions, render_still};
use motion_scene::*;

const FONTS: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/../../tests/fixtures/fonts");

fn render(element: Element, seconds: f64) -> FrameBuffer {
    let document = Document::new([240.0, 60.0], 10)
        .font("NotoSans-Regular.ttf")
        .track(TrackSpec::new().scene(SceneSpec::new(2.0, element)));
    let film = document
        .compile(&CompileOptions::default().base_dir(FONTS))
        .unwrap();
    render_still(
        film.timeline(),
        Time::from_seconds(seconds),
        &RenderOptions::default(),
    )
    .unwrap()
}

fn ink(bitmap: &FrameBuffer) -> usize {
    bitmap
        .bytes()
        .chunks(4)
        .filter(|pixel| pixel[3] > 128)
        .count()
}

fn text(spec: TextSpec) -> Element {
    Element::text(spec.family("Noto Sans").size(32.0))
        .fill(Color::WHITE)
        .translate([4.0, 40.0])
}

#[test]
fn rich_spans_style_each_run() {
    let plain = render(text(TextSpec::new("Hello world")), 0.0);
    let rich = render(
        text(TextSpec::rich([
            TextSpanSpec::new("Hello ").fill(Color::rgb(255, 0, 0)),
            TextSpanSpec::new("world").size(16.0).italic(),
        ])),
        0.0,
    );
    let red = rich
        .bytes()
        .chunks(4)
        .filter(|pixel| pixel[3] > 128 && pixel[0] > 200 && pixel[1] < 60)
        .count();
    assert!(red > 100, "red run drew {red} pixels");
    assert!(
        ink(&rich) < ink(&plain),
        "the smaller second run draws less"
    );
}

#[test]
fn visible_parts_reveal_like_a_typewriter() {
    let reveal = text(TextSpec::new("Typewriter").visible_parts(Animated::tween(0.0, 10.0, 1.0)));
    let none = ink(&render(reveal.clone(), 0.0));
    let half = ink(&render(reveal.clone(), 0.5));
    let all = ink(&render(reveal, 1.5));
    assert_eq!(none, 0);
    assert!(0 < half && half < all, "{none} {half} {all}");
}

#[test]
fn parts_stagger_opacity_by_delay() {
    let parts = text(
        TextSpec::new("Staggered")
            .split(TextSplitSpec::Characters)
            .parts(PartsSpec::new(0.1).opacity(Animated::tween(0.0, 1.0, 0.2))),
    );
    let start = ink(&render(parts.clone(), 0.0));
    let middle = ink(&render(parts.clone(), 0.3));
    let end = ink(&render(parts, 1.5));
    assert!(start < middle && middle < end, "{start} {middle} {end}");
}

#[test]
fn text_documents_round_trip_and_report_errors() {
    let element = text(
        TextSpec::rich([TextSpanSpec::new("a").weight(700)])
            .split(TextSplitSpec::Words)
            .visible_parts(Animated::expr("floor(t * 2)"))
            .parts(PartsSpec::new(0.05).offset(Animated::tween([0.0, 8.0], [0.0, 0.0], 0.3))),
    );
    let document =
        Document::new([10.0, 10.0], 4).track(TrackSpec::new().scene(SceneSpec::new(1.0, element)));
    let json = document.to_json().unwrap();
    assert_eq!(Document::from_json(&json).unwrap(), document);

    let broken = Document::new([10.0, 10.0], 4).track(
        TrackSpec::new()
            .scene(SceneSpec::new(
                1.0,
                Element::text(TextSpec {
                    spans: vec![TextSpanSpec::new("y")],
                    ..TextSpec::new("both")
                })
                .fill(Color::WHITE),
            ))
            .scene(SceneSpec::new(
                1.0,
                Element::text(TextSpec::new("")).fill(Color::WHITE),
            )),
    );
    let error = broken.compile(&CompileOptions::default()).unwrap_err();
    let messages: Vec<String> = error
        .errors()
        .iter()
        .map(|error| {
            error
                .downcast_ref::<ValidationError>()
                .unwrap()
                .message
                .clone()
        })
        .collect();
    assert_eq!(
        messages,
        [
            "Give exactly one of `content`, `spans` or `number`",
            "Give exactly one of `content`, `spans` or `number`",
        ]
    );
}
