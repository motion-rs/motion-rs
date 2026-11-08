//! Explicit still-frame comparison generator, never a benchmark.

use clap::Parser;

#[derive(Parser)]
struct Args {
    /// Directory receiving the comparison frames
    output_directory: std::path::PathBuf,
}

fn main() -> motion::Result {
    let args = Args::parse();
    motion_gallery::comparison_frames(&args.output_directory, &motion::RenderOptions::default())
}
