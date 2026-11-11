//! Project registration and command-line use of declarative JSON documents.
#![cfg(feature = "declarative")]

use motion::declarative::{Document, Element, LookSpec, SceneSpec, TrackSpec};
use motion::prelude::*;

const EXAMPLE: &str = concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/../../examples/documents/intro.json"
);

const ORBIT: &str = concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/../../examples/documents/orbit.json"
);

fn arguments(values: &[&str]) -> Vec<String> {
    values.iter().map(|value| value.to_string()).collect()
}

#[test]
fn bundled_example_document_compiles() -> motion::Result {
    let compiled = motion::declarative::load(EXAMPLE)?;
    assert_eq!(compiled.timeline().size(), Size::new(640.0, 360.0));
    let still = render_still(
        compiled.timeline(),
        1.seconds().after_origin(),
        &RenderOptions::default(),
    )?;
    assert_eq!(still.size().width, 640);
    Ok(())
}

#[test]
fn projects_register_and_render_documents() -> motion::Result {
    let directory = tempfile::tempdir()?;
    let path = directory.path().join("film.json");
    Document::new([8.0, 8.0], 4)
        .look(LookSpec::Clean)
        .track(TrackSpec::new().scene(SceneSpec::new(
            1.0,
            Element::rect([0.0, 0.0, 8.0, 8.0]).fill(Color::WHITE),
        )))
        .write(&path)?;
    let project = Project::new("documents").document("film", &path).build()?;
    assert!(project.list()[0].defaults.is_none());
    let still = project.render_still("film", None, Time::ZERO, &RenderOptions::default())?;
    let compiled = motion::declarative::load(&path)?;
    let graded = compiled.render_options(RenderOptions::default());
    let expected = render_still(compiled.timeline(), Time::ZERO, &graded)?;
    let ungraded = render_still(compiled.timeline(), Time::ZERO, &RenderOptions::default())?;
    assert_eq!(still.bytes(), expected.bytes());
    assert_eq!(&ungraded.bytes()[..4], &[255, 255, 255, 255]);
    assert_ne!(
        still.bytes(),
        ungraded.bytes(),
        "look display stage applies"
    );
    assert!(project.build("film", Some(serde_json::json!({}))).is_ok());
    assert!(
        project
            .build("film", Some(serde_json::json!({"size": 2})))
            .is_err()
    );

    let output = directory.path().join("cli.png");
    let outcome = Project::default().cli(arguments(&[
        "still",
        path.to_str().unwrap(),
        "--backend",
        "cpu",
        "--output",
        output.to_str().unwrap(),
    ]))?;
    assert!(matches!(outcome, ProjectOutcome::Still(written) if written == output));
    assert!(output.exists());
    Ok(())
}

#[test]
fn cli_prints_the_document_schema() -> motion::Result {
    let ProjectOutcome::Schema(schema) = Project::default().cli(arguments(&["schema"]))? else {
        panic!("schema outcome expected");
    };
    assert_eq!(schema, motion::declarative::schema());
    assert!(
        Project::default()
            .cli(arguments(&["schema", "extra"]))
            .is_err()
    );
    Ok(())
}

#[test]
fn invalid_documents_report_paths_through_the_project() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("broken.json");
    std::fs::write(
        &path,
        r#"{"version": 1, "size": [8, 8], "fps": 4, "tracks": [{"items": [
            {"scene": {"duration": 1, "content": {"circle": {"center": [4, 4], "radius": -1}}}}]}]}"#,
    )
    .unwrap();
    let error = Project::default()
        .build(path.to_str().unwrap(), None)
        .unwrap_err();
    assert!(
        error
            .to_string()
            .contains("tracks[0].items[0].scene.content"),
        "{error}"
    );
}

#[test]
fn committed_schema_matches_the_format() {
    let path = concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../../examples/documents/motion.schema.json"
    );
    let committed: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(path).unwrap()).unwrap();
    assert_eq!(
        committed,
        motion::declarative::schema(),
        "regenerate with: cargo run -p motion-cli -- schema > examples/documents/motion.schema.json"
    );
}

#[test]
fn parametric_example_follows_overrides() -> motion::Result {
    let project = Project::default();
    let still = |parameters: serde_json::Value| -> motion::Result<Vec<u8>> {
        let bitmap = project.render_still(
            ORBIT,
            Some(parameters),
            1.seconds().after_origin(),
            &RenderOptions::default(),
        )?;
        Ok(bitmap.bytes().to_vec())
    };
    let wide = still(serde_json::json!({}))?;
    let narrow = still(serde_json::json!({"radius": 40}))?;
    assert_ne!(wide, narrow, "the orbit radius parameter moves the planet");
    assert!(still(serde_json::json!({"orbit": 1})).is_err());
    Ok(())
}

#[test]
fn cli_overrides_document_parameters() -> motion::Result {
    let directory = tempfile::tempdir()?;
    let render = |parameters: &str, name: &str| -> motion::Result<Vec<u8>> {
        let output = directory.path().join(name);
        Project::default().cli(arguments(&[
            "still",
            ORBIT,
            "--time",
            "1",
            "--backend",
            "cpu",
            "--params",
            parameters,
            "--output",
            output.to_str().unwrap(),
        ]))?;
        Ok(std::fs::read(output)?)
    };
    assert_ne!(
        render("{}", "wide.png")?,
        render(r#"{"radius": 30}"#, "narrow.png")?
    );
    assert!(render(r#"{"nope": 1}"#, "bad.png").is_err());
    Ok(())
}

fn write_document(path: &std::path::Path, content: &str) {
    std::fs::write(
        path,
        format!(
            r#"{{"version": 1, "size": [16, 16], "fps": 4, "tracks": [{{"items": [
                {{"scene": {{"duration": 2, "content": {content}}}}}]}}]}}"#
        ),
    )
    .unwrap();
}

#[test]
fn validate_checks_documents_without_rendering() -> motion::Result {
    let directory = tempfile::tempdir()?;
    let path = directory.path().join("film.json");
    write_document(
        &path,
        r##"{"rect": {"bounds": [0, 0, 16, 16]}, "fill": "#fff"}"##,
    );
    let report = Project::default().check(path.to_str().unwrap(), None)?;
    assert_eq!(report.frames, 8);
    assert_eq!(report.checked_frames, [0, 4, 7]);
    assert_eq!(report.resources.len(), 1, "only the document itself");
    let line = report.to_string();
    assert!(line.contains("valid (16x16, 4 fps, 8 frames"), "{line}");

    let outcome = Project::default().cli(arguments(&["validate", path.to_str().unwrap()]))?;
    assert!(matches!(outcome, ProjectOutcome::Validated(report) if report.frames == 8));
    for flag in [
        ["--time", "1"].as_slice(),
        ["--output", "x.png"].as_slice(),
        ["--codec", "h264"].as_slice(),
    ] {
        let mut args = vec!["validate", path.to_str().unwrap()];
        args.extend(flag);
        assert!(
            Project::default().cli(arguments(&args)).is_err(),
            "{flag:?}"
        );
    }
    assert!(
        !directory.path().join("film.png").exists(),
        "nothing rendered"
    );

    let project = Project::new("p")
        .timeline("code", || {
            Timeline::new(Size::new(4.0, 4.0), motion::fps!(2))
                .track(|track| track.scene(1.seconds(), |_| Node::empty()))
        })
        .build()?;
    assert!(matches!(
        project.cli(arguments(&["validate", "code"]))?,
        ProjectOutcome::Validated(report) if report.resources.is_empty() && report.frames == 2
    ));
    Ok(())
}

#[test]
fn validate_reports_located_errors_and_missing_files() -> motion::Result {
    let directory = tempfile::tempdir()?;
    let path = directory.path().join("film.json");
    write_document(&path, r#"{"circle": {"center": [8, 8], "radius": 2}}"#);
    let error = Project::default()
        .check(path.to_str().unwrap(), None)
        .unwrap_err()
        .to_string();
    assert!(
        error.contains("requires a fill or a stroke") && error.contains("(line "),
        "{error}"
    );

    write_document(
        &path,
        r##"{"children": [{"rect": {"bounds": [0, 0, 16, 16]}, "fill": "#fff"}]}"##,
    );
    let mut document = std::fs::read_to_string(&path)?;
    document = document.replace(
        r#""tracks""#,
        r#""audio": [{"source": "gone.wav"}], "tracks""#,
    );
    std::fs::write(&path, document)?;
    let error = Project::default()
        .check(path.to_str().unwrap(), None)
        .unwrap_err()
        .to_string();
    assert!(
        error.contains("Missing file") && error.contains("gone.wav"),
        "{error}"
    );
    Ok(())
}

#[cfg(feature = "studio")]
#[test]
fn studio_scenes_for_documents_are_reloadable() -> motion::Result {
    let directory = tempfile::tempdir()?;
    let path = directory.path().join("film.json");
    write_document(
        &path,
        r##"{"rect": {"bounds": [0, 0, 16, 16]}, "fill": "#fff"}"##,
    );
    let project = Project::new("motion").document("film", &path).build()?;
    let scene = project.studio_scene("film")?;
    assert!(scene.is_reloadable());
    let code = Project::new("p")
        .timeline("code", || {
            Timeline::new(Size::new(4.0, 4.0), motion::fps!(2))
                .track(|track| track.scene(1.seconds(), |_| Node::empty()))
        })
        .build()?;
    assert!(!code.studio_scene("code")?.is_reloadable());
    Ok(())
}
