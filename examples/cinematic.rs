//! Renders the cinematic example with the `clean` or `film` `Look` as a video or a still.

use clap::{Parser, ValueEnum};
use motion::prelude::*;

#[derive(Clone, Copy, ValueEnum)]
enum Mode {
    Clean,
    Film,
}

#[derive(Parser)]
struct Args {
    /// Look applied to the artwork
    #[arg(value_enum, default_value = "film")]
    mode: Mode,
    /// Render a still to docs/assets instead of a video
    #[arg(long)]
    still: bool,
}

fn artwork(context: &SceneContext<'_>) -> Node {
    let horizontal = context.sample(&Tween::new(90.0, 550.0, 4.seconds()));
    let emitter = Circle::new(Point::new(horizontal, 160.0), 22.0)
        .expect("valid emitter")
        .fill(Color::rgb(255, 215, 150))
        .filter(Filter::ColorAdjust {
            exposure: 4.0,
            contrast: 1.0,
            saturation: 1.0,
            hue_degrees: 0.0,
        });
    Node::group([
        Rect::new(0.0, 0.0, 640.0, 360.0)
            .fill(Color::rgb(12, 19, 18))
            .into(),
        Rect::new(70.0, 205.0, 570.0, 290.0)
            .fill(Color::rgb(36, 134, 110))
            .layered_shadow(Color::BLACK.with_alpha(0.5), 18.0, Vector2::new(0.0, 8.0)),
        emitter,
    ])
}

fn main() -> motion::Result {
    let args = Args::parse();
    let (mode, look) = match args.mode {
        Mode::Clean => ("clean", Look::clean()),
        Mode::Film => ("film", Look::film(42)),
    };
    let scene_look = look.clone();
    let timeline = Timeline::new(Size::new(640.0, 360.0), 24)
        .track(move |track| {
            track.scene(4.seconds(), move |context| {
                scene_look
                    .scene(context, PixelSize::new(640, 360), artwork)
                    .expect("valid look")
            })
        })
        .build()?;
    if args.still {
        std::fs::create_dir_all("docs/assets")?;
        let frame = render_still(
            &timeline,
            1.seconds().after_origin(),
            &look.render_options(RenderOptions::default()),
        )?;
        frame.save_png(format!("docs/assets/cinematic-{mode}.png"))?;
        return Ok(());
    }
    let options = ExportOptions::new(format!("cinematic-{mode}.mp4"))
        .codec(Codec::H265 {
            quality: 18,
            preset: EncoderPreset::Medium,
            bit_depth: BitDepth::Ten,
            params: None,
        })
        .render(|options| look.render_options(options));
    export(&timeline, &options, &motion::encode::NoProgress)?;
    Ok(())
}
