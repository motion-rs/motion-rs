//! Visual GPU replay checks for the performance benchmark scenes.

#![cfg(all(
    feature = "gpu",
    feature = "image",
    feature = "text",
    feature = "svg",
    feature = "particles",
    feature = "ink"
))]

#[path = "../benches/performance/artwork.rs"]
mod artwork;
#[path = "../benches/performance/replay.rs"]
mod replay;
#[expect(
    dead_code,
    reason = "reuse the renderer's visual comparator and named tiers"
)]
#[path = "../../motion-render/src/gpu/visual.rs"]
mod visual;

use motion::prelude::*;
use motion::render::{
    BackendKind, BackendPreference, Composition, FrameAt, FrameBuffer, FrameClock, FrameRequest,
    PixelFormat, RasterLimits, RenderOptions, RenderSession, RenderTarget,
};
use visual::VisualTolerance;

#[test]
fn replay_uses_backend_specific_acceptance() -> motion::Result {
    let frame = |value| {
        let mut pixels = vec![255; 16 * 16 * 4];
        pixels[0] = value;
        FrameBuffer::from_rgba8(PixelSize::new(16, 16), pixels, RasterLimits::default())
    };
    let original = frame(255)?;
    assert!(replay::verify(&original, &original, BackendKind::Cpu).is_ok());
    assert!(replay::verify(&original, &frame(254)?, BackendKind::Cpu).is_err());
    assert!(replay::verify(&original, &frame(254)?, BackendKind::Gpu).is_ok());
    assert!(replay::verify(&original, &frame(0)?, BackendKind::Gpu).is_err());
    Ok(())
}

fn visual_session(backend: BackendPreference, text: bool) -> motion::Result<RenderSession> {
    let mut fonts = FontLibrary::new();
    if text {
        let font = std::env::var_os("MOTION_TEST_FONT").ok_or_else(|| {
            std::io::Error::new(std::io::ErrorKind::InvalidInput, "Set MOTION_TEST_FONT")
        })?;
        fonts.load_file(font)?;
    }
    let limits = RasterLimits {
        max_bytes: 1024 * 1024 * 1024,
        max_work: 20_000_000_000,
        ..RasterLimits::default()
    };
    RenderSession::new(
        Size::new(640.0, 360.0),
        RenderOptions::default()
            .backend(backend)
            .quality(RenderQuality::Final)
            .resources(Assets::new(".")?.with_fonts(fonts))
            .limits(limits),
    )
}

fn visual_frame(
    session: &mut RenderSession,
    case: &str,
    index: u32,
) -> motion::Result<Vec<[u8; 4]>> {
    let limits = RasterLimits {
        max_bytes: 1024 * 1024 * 1024,
        max_work: 20_000_000_000,
        ..RasterLimits::default()
    };
    let mut output = FrameBuffer::new(PixelSize::new(640, 360), limits)?;
    session.render(
        FrameRequest {
            at: FrameAt::Index(index),
            frame_rate: motion::fps!(30),
            duration: 3.seconds(),
            background: None,
            target: RenderTarget::Memory(&mut output),
            format: PixelFormat::SrgbRgba8 {
                premultiplied: false,
            },
            device_packing: true,
        },
        |context| artwork::artwork(case, context),
    )?;
    Ok(output.planes()[0].bytes().as_chunks::<4>().0.to_vec())
}

#[test]
#[ignore = "requires a native GPU and the explicit benchmark font"]
fn gpu_text_seek_replay_meets_visual_tier() -> motion::Result {
    let mut gpu = visual_session(BackendPreference::gpu(), true)?;
    let mut fonts = FontLibrary::new();
    fonts.load_file(std::env::var_os("MOTION_TEST_FONT").ok_or_else(|| {
        std::io::Error::new(std::io::ErrorKind::InvalidInput, "Set MOTION_TEST_FONT")
    })?)?;
    let composition = Composition::new(640, 360, FrameClock::new(motion::fps!(30), 90))
        .with_options(
            RenderOptions::default()
                .backend(BackendPreference::gpu())
                .quality(RenderQuality::Final)
                .resources(Assets::new(".")?.with_fonts(fonts))
                .limits(RasterLimits {
                    max_bytes: 1024 * 1024 * 1024,
                    max_work: 20_000_000_000,
                    ..RasterLimits::default()
                }),
        )?;
    let mut opaque = composition.frame_renderer(Size::new(640.0, 360.0))?;
    for index in [0, 45, 17, 89] {
        let first = visual_frame(&mut gpu, "text", index)?;
        assert!(gpu.report().gpu_preparation.unwrap().segment_count > 0);
        visual_frame(&mut gpu, "text", (index + 31) % 90)?;
        let replay = visual_frame(&mut gpu, "text", index)?;
        let difference = visual::compare_rgba8(&replay, &first, 640, VisualTolerance::REPLAY)
            .unwrap_or_else(|error| panic!("text/{index}: {error}"));
        eprintln!("text/{index} transparent Replay: {difference:?}");
        let first = opaque.render_frame(index, |context| artwork::artwork("text", context))?;
        opaque.render_frame((index + 31) % 90, |context| {
            artwork::artwork("text", context)
        })?;
        let replay = opaque.render_frame(index, |context| artwork::artwork("text", context))?;
        let rgba = |buffer: &FrameBuffer| buffer.planes()[0].bytes().as_chunks::<4>().0.to_vec();
        let difference =
            visual::compare_rgba8(&rgba(&replay), &rgba(&first), 640, VisualTolerance::REPLAY)
                .unwrap_or_else(|error| panic!("opaque text/{index}: {error}"));
        eprintln!("text/{index} opaque Replay: {difference:?}");
    }
    Ok(())
}

#[test]
#[ignore = "requires a native GPU and the explicit benchmark font"]
fn gpu_text_matches_cpu_visual_coverage_tier() -> motion::Result {
    let mut cpu = visual_session(BackendPreference::Cpu, true)?;
    let mut gpu = visual_session(BackendPreference::gpu(), true)?;
    let mut failures = Vec::new();
    for index in [0, 45, 17, 89] {
        let reference = visual_frame(&mut cpu, "text", index)?;
        let actual = visual_frame(&mut gpu, "text", index)?;
        assert!(gpu.report().gpu_preparation.unwrap().segment_count > 0);
        let partial = actual
            .iter()
            .filter(|pixel| (1..255).contains(&pixel[3]))
            .count();
        assert!(
            partial > 0,
            "text/{index} has no antialiased glyph coverage"
        );
        match visual::compare_rgba8(&actual, &reference, 640, VisualTolerance::COVERAGE) {
            Ok(difference) => eprintln!("text/{index} Coverage: {difference:?}"),
            Err(error) => failures.push(format!("text/{index}: {error}")),
        }
    }
    assert!(failures.is_empty(), "{}", failures.join("\n"));
    Ok(())
}
