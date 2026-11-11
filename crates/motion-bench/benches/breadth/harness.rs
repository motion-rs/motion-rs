//! Breadth benchmark harness: render contexts and compositions, raster limits, environment
//! flags and size lists, resource loading, checksums and replay verification.

use super::scenes::artwork;
use super::{MAX_NODES, invalid, support};
use motion::prelude::*;
use motion::render::Renderer;
use motion::render::{Composition, RasterLimits, RenderContext, ValidationLimits};

pub(super) fn checksum(pixels: &[u32]) -> u64 {
    pixels.iter().fold(0, |state, pixel| {
        support::fold_checksum(state, u64::from(*pixel))
    })
}

pub(super) fn ppm(snapshot: &motion::render::FrameBuffer) -> Vec<u8> {
    let size = snapshot.size();
    let mut bytes = format!("P6\n{} {}\n255\n", size.width, size.height).into_bytes();
    snapshot
        .write_rgb(&mut bytes)
        .expect("benchmark output is opaque SrgbRgba8");
    bytes
}

pub(super) fn limits() -> RasterLimits {
    RasterLimits {
        max_bytes: 1024 * 1024 * 1024,
        max_work: 20_000_000_000,
        ..RasterLimits::default()
    }
}

fn render_context() -> motion::Result<RenderContext> {
    Ok(
        RenderContext::new().with_validation_limits(ValidationLimits {
            max_nodes: MAX_NODES,
            ..ValidationLimits::default()
        })?,
    )
}

pub(super) fn composition(
    size: PixelSize,
    renderer: Renderer,
    resources: &Assets,
) -> motion::Result<Composition> {
    Composition::new(
        size.width as usize,
        size.height as usize,
        motion::render::FrameClock::new(30, 90),
    )
    .with_options(
        motion::render::RenderOptions::default()
            .backend(renderer.into())
            .quality(motion::RenderQuality::Final)
            .context(render_context()?)
            .resources(resources.clone())
            .limits(limits()),
    )
}

pub(super) fn parse_sizes(value: &str) -> motion::Result<Vec<PixelSize>> {
    value
        .split(',')
        .map(|item| {
            let (width, height) = item
                .split_once('x')
                .ok_or_else(|| invalid("Expected WxH,WxH"))?;
            let parse = |value: &str| {
                value
                    .parse::<u32>()
                    .ok()
                    .filter(|value| (16..=4096).contains(value))
                    .ok_or_else(|| invalid("Size dimensions require 16..=4096"))
            };
            Ok(PixelSize::new(parse(width)?, parse(height)?))
        })
        .collect()
}

pub(super) fn flag(name: &str, default: bool) -> bool {
    std::env::var(name).map_or(default, |value| value == "1")
}

pub(super) fn load_resources() -> motion::Result<(Assets, String)> {
    let path = std::env::var_os("MOTION_BENCH_FONT").map_or_else(
        || {
            std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
                .join("../../tests/fixtures/fonts/NotoSans-Regular.ttf")
        },
        Into::into,
    );
    let fingerprint = support::fingerprint(&std::fs::read(&path)?);
    let mut fonts = FontLibrary::new();
    fonts.load_file(path)?;
    Ok((Assets::new(".")?.with_fonts(fonts), fingerprint))
}

/// Replays two frames and a seek to prove determinism before anything is timed.
pub(super) fn verify_replay(
    session: &mut motion::render::FrameRenderer,
    name: &str,
    case: &str,
) -> motion::Result {
    for index in [0, 45, 89] {
        let first = session.render_frame(index, |context| artwork(case, context))?;
        session.render_frame((index + 31) % 90, |context| artwork(case, context))?;
        let replay = session.render_frame(index, |context| artwork(case, context))?;
        if first != replay {
            return Err(invalid(format!("Seek replay differs: {name}/{index}")));
        }
        if first
            .packed_rgb()
            .iter()
            .all(|pixel| *pixel == first.packed_rgb()[0])
            && index == 45
        {
            return Err(invalid(format!("Case renders a flat image: {name}")));
        }
    }
    Ok(())
}
