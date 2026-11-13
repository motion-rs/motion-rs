//! Source locations and suggestions in document diagnostics.

use motion_scene::*;
use serde_json::json;

fn message(report: motion_core::ErrorReport) -> String {
    report
        .errors()
        .iter()
        .map(|error| error.to_string())
        .collect::<Vec<_>>()
        .join("\n")
}

const HEAD: &str = r#"{"version": 1, "size": [8, 8], "fps": 4, "tracks": [{"items": ["#;

fn scene(content: &str) -> String {
    format!(r#"{HEAD}{{"scene": {{"duration": 1, "content": {content}}}}}]}}]}}"#)
}

#[test]
fn unknown_keys_and_variants_suggest_the_closest_name() {
    let text = format!(
        "{HEAD}\n  {{\"scene\": {{\"duration\": 1, \"content\": {{\n    \"rect\": {{\"bounds\": [0, 0, 1, 1]}},\n    \"fil\": \"white\"\n  }}}}}}]}}]}}"
    );
    let error = message(Document::from_json(&text).unwrap_err().into());
    assert!(error.contains("unknown field `fil`"), "{error}");
    assert!(error.contains("(line 4, column 5)"), "{error}");
    assert!(error.ends_with("Did you mean `fill`?"), "{error}");

    let text = scene(
        r#"{"circle": {"center": [1, 1], "radius": {"tween": {"from": 0, "to": 1, "duration": 1, "easing": "cubic_ot"}}}, "fill": "white"}"#,
    );
    let error = message(Document::from_json(&text).unwrap_err().into());
    assert!(error.contains("Did you mean `cubic_out`?"), "{error}");
    assert!(error.contains("line 1"), "{error}");
}

#[test]
fn syntax_errors_keep_serde_positions() {
    let error = message(
        Document::from_json("{\n  \"version\": 1,\n  oops\n}")
            .unwrap_err()
            .into(),
    );
    assert!(error.contains("line 3"), "{error}");
}

#[test]
fn parameter_and_expression_typos_are_suggested() {
    let text = r#"{"version": 1, "size": [8, 8], "fps": 4, "params": {"radius": 3},
            "tracks": [{"items": [{"scene": {"duration": 1, "content": {
              "circle": {"center": [1, 1], "radius": {"$param": "radis"}}, "fill": "white"}}}]}]}"#;
    let error = message(Document::from_json(text).unwrap_err().into());
    assert!(error.contains("Did you mean `radius`?"), "{error}");

    let error = message(
        Document::from_json_with(&text.replace("radis", "radius"), Some(&json!({"radis": 1})))
            .unwrap_err()
            .into(),
    );
    assert!(error.contains("Did you mean `radius`?"), "{error}");

    let document = Document::from_json(&scene(
        r#"{"circle": {"center": [1, 1], "radius": {"expr": "sqr(4)"}}, "fill": "white"}"#,
    ))
    .unwrap();
    let error = message(document.compile(&CompileOptions::default()).unwrap_err());
    assert!(error.contains("Did you mean `sqrt`?"), "{error}");

    let document = Document::from_json(&scene(
        r#"{"children": [
            {"id": "planet", "rect": {"bounds": [0, 0, 1, 1]}, "fill": "white"},
            {"rect": {"bounds": [0, 0, 1, 1]}, "fill": "white", "opacity": {"expr": "@planit.opacity"}}]}"#,
    ))
    .unwrap();
    let error = message(document.compile(&CompileOptions::default()).unwrap_err());
    assert!(error.contains("Did you mean `planet`?"), "{error}");
}

#[test]
fn loaded_documents_report_lines_for_compile_errors_and_list_resources() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("film.json");
    std::fs::write(
        &path,
        format!(
            "{HEAD}\n{{\"scene\": {{\"duration\": 1, \"content\": {{\n  \"children\": [\n    {{\"circle\": {{\"center\": [1, 1], \"radius\": 2}}}}\n  ]\n}}}}}}]}}],\n \"audio\": [{{\"source\": \"a.wav\"}}, {{\"source\": \"a.wav\"}}]}}"
        ),
    )
    .unwrap();
    let error = message(load(&path).unwrap_err());
    assert!(error.contains("requires a fill or a stroke"), "{error}");
    assert!(error.contains("(line 4, column 5)"), "{error}");

    std::fs::write(
        &path,
        std::fs::read_to_string(&path)
            .unwrap()
            .replace(r#""radius": 2}}"#, r#""radius": 2}, "fill": "white"}"#),
    )
    .unwrap();
    let film = load(&path).unwrap();
    let path = std::path::absolute(&path).unwrap();
    assert_eq!(film.resources()[0], path);
    assert_eq!(
        film.resources()
            .iter()
            .filter(|file| file.ends_with("a.wav"))
            .count(),
        1,
        "resources are listed once: {:?}",
        film.resources()
    );
}

#[cfg(feature = "particles")]
#[test]
fn particle_preset_diagnostics_identify_inputs_and_source_lines() {
    for (changes, field) in [
        (json!({"density": 0}), "density"),
        (json!({"speed": 17}), "speed"),
        (json!({"intensity": 65}), "intensity"),
        (json!({"bounds": [0, 0, 0, 48]}), "bounds"),
        (json!({"kind": "logo_assemble"}), "logo"),
    ] {
        let mut preset = json!({"kind":"sparks", "bounds":[0,0,64,48]});
        preset
            .as_object_mut()
            .unwrap()
            .extend(changes.as_object().unwrap().clone());
        let text =
            scene(&json!({"particles":{"seed":17,"duration":4,"preset":preset}}).to_string());
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("film.json");
        std::fs::write(&path, text).unwrap();
        let error = load(&path).unwrap_err();
        let expected = format!("tracks[0].items[0].scene.content.particles.preset.{field}");
        assert!(
            error
                .errors()
                .iter()
                .filter_map(|error| error.downcast_ref::<motion_core::ValidationError>())
                .any(|error| format!("{}.{}", error.path, error.field) == expected),
            "{}",
            message(error)
        );
    }
    let text = scene(
        r#"{"particles":{"seed":17,"duration":4,"preset":{
      "kind":"sparks",
      "bounds":[0,0,64,48],
      "density":0
    }}}"#,
    );
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("film.json");
    std::fs::write(&path, text).unwrap();
    let error = message(load(&path).unwrap_err());
    assert!(error.contains("preset.density"), "{error}");
    assert!(error.contains("(line 4,"), "{error}");
}
