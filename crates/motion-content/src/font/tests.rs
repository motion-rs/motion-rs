use super::*;
use crate::Text;
use crate::asset::AssetsExt;
#[cfg(feature = "gpu")]
use motion_core::Rect;
use motion_core::{
    Assets, Color, Node, PixelSize, RasterLimits, ResourceStore, Size, ValidationLimits,
};
use motion_render::RenderPrepare;

#[test]
fn explicit_font_validation_rejects_bad_sources_and_indices() {
    assert!(FontFace::from_bytes(Vec::new(), 0).is_err());
    assert!(FontFace::from_bytes(vec![0; MAX_FONT_FACE_BYTES + 1], 0).is_err());
    let face = FontFace {
        bytes: Arc::from([]),
        index: 0,
    };
    for size in [0.0, -1.0, f64::NAN, f64::INFINITY, 4097.0] {
        assert_eq!(
            Text::from_font("x", &face, size, Color::WHITE)
                .unwrap_err()
                .field,
            "font_size"
        );
    }
}

#[test]
#[ignore = "requires an explicit MOTION_TEST_FONT fixture; no font discovery"]
fn explicit_fonts_use_paragraph_layout_and_keep_registry_snapshots() -> motion_core::Result {
    let bytes = std::fs::read(std::env::var("MOTION_TEST_FONT")?)?;
    let face = FontFace::from_bytes(bytes.clone(), 0)?;
    assert!(FontFace::from_bytes(bytes, u32::MAX).is_err());
    let store =
        Assets::new(".")?.with_store(ResourceStore::with_limits(motion_core::ResourceLimits {
            max_entries: 4096,
            max_bytes: face.byte_len(),
        })?);
    store.preload("title", Arc::new(face.clone()))?;
    let registered = store.font("title").get()?;
    let text = Text::from_font("office AV", &registered, 24.0, Color::WHITE)?;
    let layout = text.layout(&face.library()?)?;
    assert!(layout.baseline() > 0.0 && !layout.glyphs().is_empty());
    let snapshot = store.snapshot();
    assert!(store.remove::<FontFace>("title"));
    assert!(snapshot.font("title").get().is_ok());
    let prepared = Node::from(text)
        .compile(
            Size::new(128.0, 64.0),
            PixelSize::new(128, 64),
            ValidationLimits::default(),
        )?
        .prepare(ValidationLimits::default(), RasterLimits::default())?;
    let output = prepared.render_linear(RasterLimits::default())?;
    assert!(output.linear_pixels().any(|pixel| pixel[3] > 0.5));
    assert_eq!(prepared.render_linear(RasterLimits::default())?, output);
    Ok(())
}

#[cfg(feature = "gpu")]
#[test]
#[ignore = "requires MOTION_TEST_FONT and a native GPU; no window"]
fn explicit_font_gpu_matches_cpu_and_replays() -> motion_core::Result {
    use motion_core::NodeModifiers;
    use motion_render::{Composition, Renderer};
    let font = FontFace::from_bytes(std::fs::read(std::env::var("MOTION_TEST_FONT")?)?, 0)?;
    let latin = Text::from_font("office AV", &font, 16.0, Color::WHITE)?;
    let arabic = Text::from_font(
        "\u{633}\u{644}\u{627}\u{645}",
        &font,
        24.0,
        Color::from_hex(0x24a49b),
    )?;
    let row = motion_core::Row::new()
        .gap(4.0)
        .alignment(motion_core::Alignment::Baseline)
        .children([latin, arabic]);
    let composition = Composition::new(
        128,
        64,
        motion_render::FrameClock::new(motion_core::fps!(30), 3),
    )
    .with_options(motion_render::RenderOptions::default().background(motion_core::Color::BLACK))?;
    let mut cpu = composition.frame_renderer(Size::new(128.0, 64.0))?;
    let mut gpu = composition
        .configure(|options| options.backend(Renderer::Gpu.into()))
        .frame_renderer(Size::new(128.0, 64.0))?;
    let build = |context: &motion_core::SceneContext<'_>| {
        row.clone()
            .translate(context.seconds() * 24.0, 4.0)
            .opacity(0.75)
            .clip_with(
                Rect::new(0.5, 0.25, 127.5, 63.75),
                motion_core::ClipMode::Geometric,
            )
    };
    let mut replay = None;
    let mut maximum = 0;
    for index in [2, 0, 1, 2] {
        let expected = cpu.render_frame(index, build)?;
        let actual = gpu.render_frame(index, build)?;
        assert!(actual.packed_rgb().iter().any(|pixel| *pixel != 0));
        for (actual, expected) in actual.packed_rgb().iter().zip(expected.packed_rgb()) {
            for shift in [0, 8, 16] {
                let difference =
                    (((actual >> shift) & 255) as i32 - ((expected >> shift) & 255) as i32).abs();
                maximum = maximum.max(difference);
                assert!(difference <= 1);
            }
        }
        if index == 2 {
            if let Some(previous) = &replay {
                assert_eq!(actual.packed_rgb(), *previous);
            }
            replay = Some(actual.packed_rgb());
        }
    }
    let info = gpu.gpu_info().unwrap();
    println!(
        "_text_gpu adapter={} backend={:?} requests=4 size=128x64 max_rgb_difference={maximum} replay=pass",
        info.name, info.backend
    );
    Ok(())
}
