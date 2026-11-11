//! Opens a rotating rectangle document in Motion Studio.

use motion::declarative::{
    Animated, CompileOptions, Document, Element, RectSpec, SceneSpec, TrackSpec,
};
use motion::prelude::*;
use motion::render::Composition;

fn main() -> motion::Result {
    let document = Document::new([640.0, 360.0], FrameRate::NTSC).track(
        TrackSpec::new().scene(SceneSpec::new(
            4.0,
            Element::new(RectSpec::new([180.0, 100.0, 460.0, 260.0]).corner_radius(12.0))
                .fill(Color::rgb(35, 155, 130))
                .rotate(Animated::expr("degrees(t * 0.6)")),
        )),
    );
    let film = document.compile(&CompileOptions::default())?;
    motion::studio::run(motion::studio::StudioScene::timeline(
        "timeline",
        Composition::new(
            640,
            360,
            motion::render::FrameClock::new(motion::fps!(30), 150),
        ),
        film.timeline().clone(),
    )?)
}
