//! Minimal project: one animated circle, described declaratively and driven by the project command line.

use motion::declarative::{Animated, CompileOptions, Document, Element, SceneSpec, TrackSpec};
use motion::prelude::*;

fn document() -> Document {
    Document::new([640.0, 360.0], 30).track(
        TrackSpec::new().scene(SceneSpec::new(
            3.0,
            Element::circle(Animated::tween([80.0, 180.0], [560.0, 180.0], 3.0), 32.0)
                .fill(Color::rgb(20, 160, 130)),
        )),
    )
}

fn main() -> std::process::ExitCode {
    Project::new("motion")
        .timeline("motion", || {
            document()
                .compile(&CompileOptions::default())
                .expect("valid document")
                .timeline()
                .clone()
        })
        .run()
}
