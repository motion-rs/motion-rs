//! Compiles a Typst formula and renders it to a still image.

use clap::Parser;
use motion::eager::FormulaCompiler;
use motion::prelude::*;

#[derive(Parser)]
struct Args {
    /// Destination PNG
    #[arg(default_value = "formula.png")]
    output: std::path::PathBuf,
}

fn main() -> motion::Result {
    let args = Args::parse();
    let compiler = FormulaCompiler::new();
    let formula = compiler.compile(
        "E = m c^2",
        &FormulaOptions {
            size: 40.0,
            fill: Color::rgb(20, 140, 120),
            ..FormulaOptions::default()
        },
    )?;
    let selected = formula
        .select_symbol("c")?
        .apply(|node| node.translate(0.0, -8.0));
    let timeline = Timeline::try_new(Size::new(480.0, 180.0), 30)?
        .try_track(|track| {
            track.try_push(
                (move |_: &SceneContext<'_>| selected.clone().translate(40.0, 55.0))
                    .with_duration(1.seconds()),
            )
        })?
        .build()?;
    let bitmap = render_still(&timeline, Time::ZERO, &RenderOptions::default())?;
    bitmap.save_png(args.output)
}
