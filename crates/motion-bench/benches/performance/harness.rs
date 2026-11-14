//! Performance benchmark harness: compositions and raster limits, environment parsing,
//! resource loading, checksums, reference images and replay verification.

use super::{MAX_NODES, invalid, replay, scene, support};
use motion::prelude::*;
use motion::render::BackendKind;
use motion::render::{
    Composition, FrameBuffer, FrameRenderer, RasterLimits, RenderContext, ValidationLimits,
};

/// Frame indices rendered, replayed after an unrelated seek and stored as reference images.
pub(super) const REFERENCE_FRAMES: [u32; 4] = [0, 45, 17, 89];

pub(super) fn checksum(pixels: &[u32]) -> u64 {
    pixels.iter().fold(0, |state, pixel| {
        support::fold_checksum(state, u64::from(*pixel))
    })
}

pub(super) fn ppm(snapshot: &FrameBuffer) -> Vec<u8> {
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
    renderer: BackendKind,
    resources: &Assets,
) -> motion::Result<Composition> {
    Composition::new(
        size.width as usize,
        size.height as usize,
        motion::render::FrameClock::new(motion::fps!(30), 90),
    )
    .with_options(
        motion::render::RenderOptions::default()
            .backend(motion::render::BackendPreference::from_kind(renderer))
            .quality(motion::RenderQuality::Final)
            .context(render_context()?)
            .resources(resources.clone())
            .limits(limits()),
    )
}

/// Returns the composition a case is measured with; `display-transform` adds tone mapping
/// and seeded grain to the shared composition.
pub(super) fn case_composition(base: &Composition, case: &str) -> Composition {
    if case != "display-transform" {
        return base.clone();
    }
    base.clone().configure(|options| {
        options.display(DisplayTransform {
            tone_map: ToneMap::AgX,
            grain: Some(FilmGrain {
                seed: 9,
                amount: 0.01,
                size: 1.4,
                luma_response: 1.0,
                chroma: 0.1,
            }),
            ..DisplayTransform::default()
        })
    })
}

pub(super) fn parse_count(value: &str, maximum: usize) -> motion::Result<usize> {
    let count = value
        .parse::<usize>()
        .map_err(|_| invalid("Expected a positive benchmark count"))?;
    if !(1..=maximum).contains(&count) {
        return Err(invalid(format!("Benchmark count must be in 1..={maximum}")));
    }
    Ok(count)
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

/// Renders every reference frame, replays it after an unrelated seek and requires the replay
/// to match under the backend's acceptance rule. Returns the first render of each frame.
pub(super) fn replay_frames(
    session: &mut FrameRenderer,
    renderer: BackendKind,
    name: &str,
    case: &str,
    particle_count: usize,
) -> motion::Result<Vec<(u32, FrameBuffer)>> {
    let mut frames = Vec::new();
    for index in REFERENCE_FRAMES {
        let first = session.render_frame(index, |context| scene(case, context, particle_count))?;
        session.render_frame((index + 31) % 90, |context| {
            scene(case, context, particle_count)
        })?;
        let replay = session.render_frame(index, |context| scene(case, context, particle_count))?;
        replay::verify(&first, &replay, renderer)
            .map_err(|error| invalid(format!("Seek replay differs: {name}/{index}: {error}")))?;
        if index == 45 && case != "empty" {
            let pixels = first.packed_rgb();
            if pixels.iter().all(|pixel| *pixel == pixels[0]) {
                return Err(invalid(format!("Case renders a flat image: {name}")));
            }
        }
        frames.push((index, first));
    }
    Ok(frames)
}
