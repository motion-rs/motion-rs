//! Exports a short video that mixes a voice track over music, described declaratively.
//!
//! Usage: `cargo run --example audio_export -- voice.wav music.mp3 output.mp4`.

use clap::Parser;
use motion::declarative::{
    Animated, AudioSpec, CompileOptions, Document, Element, SceneSpec, TrackSpec,
};
use motion::encode::NoProgress;
use motion::prelude::*;
use std::path::PathBuf;

#[derive(Parser)]
struct Args {
    /// Narration audio file
    voice: PathBuf,
    /// Background music file
    music: PathBuf,
    /// Destination video
    output: PathBuf,
}

fn main() -> motion::Result {
    let Args {
        voice,
        music,
        output,
    } = Args::parse();
    let narration = AudioSpec {
        start: Some(1.0),
        duration: Some(4.0),
        fade_in: Some(0.1),
        fade_out: Some(0.2),
        ..AudioSpec::new(voice.to_string_lossy())
    };
    let music = AudioSpec {
        offset: Some(2.0),
        duration: Some(6.0),
        fade_in: Some(0.5),
        fade_out: Some(1.0),
        ..AudioSpec::new(music.to_string_lossy()).volume(0.12)
    };
    let scene = Element::group([
        Element::rect([48.0, 130.0, 592.0, 146.0]).fill(Color::rgb(20, 150, 125)),
        Element::rect([48.0, 210.0, 592.0, 226.0]).fill(Color::rgb(50, 125, 195)),
        Element::rect(Animated::exprs([
            "48 + t * 90",
            "110",
            "50 + t * 90",
            "246",
        ]))
        .fill(Color::WHITE),
    ]);
    let film = Document::new([640.0, 360.0], 30)
        .audio(narration)
        .audio(music)
        .track(TrackSpec::new().scene(SceneSpec::new(6.0, scene)))
        .duration(6.0)
        .compile(&CompileOptions::default())?;
    let options = ExportOptions::new(output).render(|options| film.render_options(options));
    export(film.timeline(), &options, &NoProgress).map(|_| ())
}
