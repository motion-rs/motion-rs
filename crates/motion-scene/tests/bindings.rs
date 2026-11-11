//! Parameters, expressions and element bindings in declarative documents.

use motion_core::{Time, ValidationError};
use motion_render::{RenderOptions, render_still};
use motion_scene::*;
use serde_json::json;

const WHITE: [u8; 4] = [255, 255, 255, 255];

const BLACK: [u8; 4] = [0, 0, 0, 255];

fn load(
    text: &str,
    overrides: Option<serde_json::Value>,
) -> Result<CompiledDocument, motion_core::ErrorReport> {
    Document::from_json_with(text, overrides.as_ref())?.compile(&CompileOptions::default())
}

fn pixel(film: &CompiledDocument, seconds: f64, x: u32, y: u32) -> [u8; 4] {
    let bitmap = render_still(
        film.timeline(),
        Time::from_seconds(seconds),
        &RenderOptions::default(),
    )
    .unwrap();
    let offset = (y as usize * bitmap.size().width as usize + x as usize) * 4;
    bitmap.bytes()[offset..offset + 4].try_into().unwrap()
}

fn diagnostics(error: motion_core::ErrorReport) -> Vec<ValidationError> {
    error
        .errors()
        .iter()
        .map(|error| error.downcast_ref::<ValidationError>().unwrap().clone())
        .collect()
}

fn scene(content: &str, params: &str) -> String {
    format!(
        r#"{{"version": 1, "size": [40, 20], "fps": 10, "background": "black",
            "params": {params},
            "tracks": [{{"items": [{{"scene": {{"duration": 2, "content": {content}}}}}]}}]}}"#
    )
}

#[test]
fn parameters_substitute_anywhere_and_accept_overrides() {
    let text = scene(
        r##"{"circle": {"center": [20, 10], "radius": {"$param": "radius"}},
             "fill": {"$param": "tint"}}"##,
        r##"{"radius": 8, "tint": "#ffffff"}"##,
    );
    let film = load(&text, None).unwrap();
    assert_eq!(pixel(&film, 0.0, 20, 10), WHITE);
    assert_eq!(pixel(&film, 0.0, 20, 1), BLACK);
    let film = load(&text, Some(json!({"radius": 2, "tint": "#ff0000"}))).unwrap();
    assert_eq!(pixel(&film, 0.0, 20, 10), [255, 0, 0, 255]);
    assert_eq!(pixel(&film, 0.0, 20, 5), BLACK);
}

#[test]
fn parameters_can_supply_subtrees_and_are_recorded_after_resolution() {
    let text = scene(
        r##"{"children": {"$param": "shapes"}}"##,
        r##"{"shapes": [{"rect": {"bounds": [0, 0, 40, 20]}, "fill": "white"}]}"##,
    );
    let document = Document::from_json(&text).unwrap();
    assert_eq!(document.params.len(), 1);
    let json = document.to_json().unwrap();
    assert!(!json.contains("$param"));
    assert_eq!(Document::from_json(&json).unwrap(), document);
    let film = document.compile(&CompileOptions::default()).unwrap();
    assert_eq!(pixel(&film, 0.0, 3, 3), WHITE);
}

#[test]
fn parameter_errors_are_located() {
    let text = scene(
        r#"{"circle": {"center": [1, 1], "radius": {"$param": "missing"}}, "fill": "white"}"#,
        "{}",
    );
    let error = Document::from_json(&text).unwrap_err();
    let diagnostic = error.downcast_ref::<ValidationError>().unwrap();
    assert!(diagnostic.path.ends_with("circle.radius"), "{diagnostic:?}");
    assert!(diagnostic.message.contains("missing"));

    let text = scene(r#"{"children": []}"#, r#"{"size": 1}"#);
    let error = Document::from_json_with(&text, Some(&json!({"sizes": 2}))).unwrap_err();
    assert!(error.to_string().contains("Unknown parameter: sizes"));
    let error = Document::from_json_with(&text, Some(&json!([1]))).unwrap_err();
    assert!(error.to_string().contains("must be a JSON object"));

    let text = scene(r#"{"children": []}"#, r#"{"a": {"$param": "a"}}"#);
    assert!(
        Document::from_json(&text).is_ok(),
        "params are not substituted"
    );
}

#[test]
fn expressions_drive_properties_from_time_and_parameters() {
    let text = scene(
        r#"{"rect": {"bounds": [0, 0, 10, 20]}, "fill": "white",
            "translate": {"expr": ["t * $speed", "0"]}}"#,
        r#"{"speed": 10}"#,
    );
    let film = load(&text, None).unwrap();
    assert_eq!(pixel(&film, 0.0, 5, 10), WHITE);
    assert_eq!(pixel(&film, 0.0, 20, 10), BLACK);
    assert_eq!(pixel(&film, 1.5, 5, 10), BLACK);
    assert_eq!(pixel(&film, 1.5, 20, 10), WHITE);
    let film = load(&text, Some(json!({"speed": 0}))).unwrap();
    assert_eq!(pixel(&film, 1.5, 5, 10), WHITE);

    let text = scene(
        r#"{"rect": {"bounds": [0, 0, 40, 20]}, "fill": "white",
            "opacity": {"expr": "if(t < 1, 1, 0)"}}"#,
        "{}",
    );
    let film = load(&text, None).unwrap();
    assert_eq!(pixel(&film, 0.5, 5, 5), WHITE);
    assert_eq!(pixel(&film, 1.5, 5, 5), BLACK);
}

#[test]
fn bindings_follow_other_elements() {
    let text = scene(
        r#"{"children": [
            {"id": "lead", "rect": {"bounds": [0, 0, 4, 4]}, "fill": "white",
             "position": {"tween": {"from": [2, 2], "to": [22, 2], "duration": 2}}},
            {"rect": {"bounds": [0, 0, 4, 4]}, "fill": "red",
             "position": {"expr": ["@lead.position.x", "@lead.position.y + 10"]}},
            {"id": "ring", "circle": {"center": [30, 14], "radius": {"expr": "2 + t"}},
             "fill": "blue"},
            {"circle": {"center": [30, 4], "radius": {"expr": "@ring.circle.radius / 2"}}, "fill": "green"}
        ]}"#,
        "{}",
    );
    let film = load(&text, None).unwrap();
    assert_eq!(pixel(&film, 1.0, 12, 2), WHITE);
    assert_eq!(pixel(&film, 1.0, 12, 12), [255, 0, 0, 255]);
    assert_eq!(pixel(&film, 0.0, 2, 12), [255, 0, 0, 255]);
    assert_eq!(pixel(&film, 0.0, 12, 12), BLACK);
    // The ring grows as 2 + t and the green circle follows at half its radius.
    assert_eq!(pixel(&film, 1.0, 30, 4), [0, 128, 0, 255]);
    assert_eq!(pixel(&film, 1.0, 30, 14), [0, 0, 255, 255]);
}

#[test]
fn binding_errors_are_reported() {
    let cycle = scene(
        r#"{"children": [
            {"id": "a", "circle": {"center": [1, 1], "radius": {"expr": "@b.circle.radius"}}, "fill": "white"},
            {"id": "b", "circle": {"center": [1, 1], "radius": {"expr": "@a.circle.radius"}}, "fill": "white"}]}"#,
        "{}",
    );
    let messages: Vec<_> = diagnostics(load(&cycle, None).unwrap_err())
        .into_iter()
        .map(|error| error.message)
        .collect();
    assert!(
        messages
            .iter()
            .any(|message| message.contains("Binding cycle")),
        "{messages:?}"
    );

    let broken = scene(
        r#"{"children": [
            {"id": "a", "rect": {"bounds": [0, 0, 1, 1]}, "fill": "white"},
            {"id": "a", "rect": {"bounds": [0, 0, 1, 1]}, "fill": "white",
             "opacity": {"expr": "@ghost.opacity"}},
            {"id": "9x", "rect": {"bounds": [0, 0, 1, 1]}, "fill": "white",
             "scale": {"expr": "@a.fill"}},
            {"rect": {"bounds": [0, 0, 1, 1]}, "fill": "white",
             "rotate": {"expr": "sin(1, 2)"},
             "translate": {"expr": "t"}}]}"#,
        "{}",
    );
    let found: Vec<_> = diagnostics(load(&broken, None).unwrap_err())
        .into_iter()
        .map(|error| format!("{} {}", error.field, error.message))
        .collect();
    for expected in [
        "Duplicate element id",
        "No element with id `ghost`",
        "Ids are 1..=128",
        "does not name a numeric property",
        "`sin` takes 1 argument(s)",
        "Expected 2 expression(s), found 1",
    ] {
        assert!(
            found.iter().any(|message| message.contains(expected)),
            "{expected}: {found:?}"
        );
    }
}

#[test]
fn bindings_are_scoped_to_their_scene() {
    let text = r#"{"version": 1, "size": [4, 4], "fps": 4, "tracks": [{"items": [
        {"scene": {"duration": 1, "content": {"id": "a", "rect": {"bounds": [0, 0, 1, 1]}, "fill": "white"}}},
        {"scene": {"duration": 1, "content": {"rect": {"bounds": [0, 0, 1, 1]}, "fill": "white",
                                              "opacity": {"expr": "@a.opacity"}}}}]}]}"#;
    let found = diagnostics(load(text, None).unwrap_err());
    assert!(found[0].message.contains("No element with id `a`"));
}

#[test]
fn expressions_round_trip_and_describe_in_the_schema() {
    let document = Document::new([8.0, 8.0], 4).track(
        TrackSpec::new().scene(SceneSpec::new(
            1.0,
            Element::circle(Animated::exprs(["4 + t", "4"]), Animated::expr("$r"))
                .fill(motion_core::Color::WHITE)
                .id("dot"),
        )),
    );
    let json = document.to_json().unwrap();
    assert!(json.contains(r#""expr": ["#));
    assert_eq!(Document::from_json(&json).unwrap(), document);

    let schema = schema();
    let definitions = schema["$defs"].as_object().unwrap();
    assert!(definitions.contains_key("ParamRef"));
    let element = &definitions["Element"];
    let radius = serde_json::to_string(&element["anyOf"]).unwrap();
    assert!(radius.contains("ParamRef"), "{radius}");
    assert!(definitions.contains_key("ExprSpec"));
}
