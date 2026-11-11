//! Integration tests for documented authoring patterns, such as reusing text layouts across frames.

#[cfg(feature = "text")]
#[test]
fn fresh_texts_reuse_actual_session_layouts_across_out_of_order_frames() -> motion::Result {
    use motion::prelude::*;
    use motion::render::{
        FrameAt, FrameBuffer, FrameRequest, PixelFormat, RasterLimits, RenderSession, RenderTarget,
    };
    let mut fonts = FontLibrary::new();
    fonts.load_file(
        std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../tests/fixtures/fonts/NotoSans-Regular.ttf"),
    )?;
    let identity = fonts.snapshot_id();
    let assets = Assets::new(".")?.with_fonts(fonts);
    let size = Size::new(128.0, 48.0);
    let mut session = RenderSession::new(size, RenderOptions::default().resources(assets))?;
    let mut target = FrameBuffer::new(PixelSize::new(128, 48), RasterLimits::default())?;
    assert_eq!(session.cached_layout_bytes(), 0);
    let mut retained = 0;
    for index in [0, 1, 0] {
        session.render(
            FrameRequest {
                at: FrameAt::Index(index),
                frame_rate: FrameRate::from(30),
                duration: 1.seconds(),
                background: None,
                target: RenderTarget::Memory(&mut target),
                format: PixelFormat::SrgbRgba8 {
                    premultiplied: false,
                },
                device_packing: false,
            },
            |context| {
                assert_eq!(context.assets().fonts().unwrap().snapshot_id(), identity);
                Text::new("Session text")
                    .size(16.0)
                    .fill(Color::WHITE)
                    .translate(4.0, 4.0)
            },
        )?;
        let bytes = session.cached_layout_bytes();
        assert!(bytes > 0 && bytes <= 8 * 1024 * 1024);
        if retained == 0 {
            retained = bytes;
        } else {
            assert_eq!(bytes, retained);
        }
        assert!(!target.planes().is_empty());
    }
    Ok(())
}

#[test]
fn authoring_example_1() -> Result<(), Box<dyn std::error::Error>> {
    use motion::extend::*;
    use motion::prelude::*;
    use motion::render::RasterLimits;

    let tile = || Node::from(Rect::new(0.0, 0.0, 1.0, 1.0).fill(Color::WHITE)).opacity(0.5);
    let scene = Node::group([tile(), tile().blend_mode(BlendMode::Multiply)]);
    let prepared = scene
        .compile(
            Size::new(1.0, 1.0),
            PixelSize::new(1, 1),
            ValidationLimits::default(),
        )?
        .prepare(ValidationLimits::default(), RasterLimits::default())?;
    assert_eq!(
        prepared
            .render_linear(RasterLimits::default())?
            .linear_pixel(0, 0),
        Some([0.75; 4])
    );
    assert_eq!(
        scene
            .preflight_with(&DrawContext::default(), ValidationLimits::default())?
            .nodes,
        3
    );
    Ok(())
}

#[test]
fn authoring_example_2() -> Result<(), Box<dyn std::error::Error>> {
    use motion::prelude::*;
    use motion::render::RasterLimits;
    use motion::render::*;
    let node = Node::from(Rect::new(0.0, 0.0, 2.0, 2.0).fill(Color::WHITE))
        .filter(Filter::Blur { sigma: 0.5 });
    let output = node
        .compile(
            Size::new(4.0, 4.0),
            PixelSize::new(4, 4),
            ValidationLimits::default(),
        )?
        .prepare(ValidationLimits::default(), RasterLimits::default())?
        .render_linear(RasterLimits::default())?;
    assert!(output.linear_pixel(2, 0).unwrap()[3] > 0.0);
    Ok(())
}

#[test]
fn authoring_example_3() -> Result<(), Box<dyn std::error::Error>> {
    use motion::extend::*;
    use motion::prelude::*;
    use motion::render::RasterLimits;

    let tile = || Node::from(Rect::new(0.0, 0.0, 2.0, 1.0).fill(Color::WHITE));
    let path = Path2D::from_rect(Rect::new(0.5, 0.0, 2.0, 1.0))?;
    let node = Node::group([tile(), tile()]).clip(path);
    let prepared = node
        .compile(
            Size::new(2.0, 1.0),
            PixelSize::new(2, 1),
            ValidationLimits::default(),
        )?
        .prepare(ValidationLimits::default(), RasterLimits::default())?;
    assert_eq!(prepared.clips().len(), 1);
    let output = prepared.render_linear(RasterLimits::default())?;
    assert_eq!(output.linear_pixel(0, 0), Some([128.0 / 255.0; 4]));
    assert_eq!(output.linear_pixel(1, 0), Some([1.0; 4]));
    Ok(())
}

#[test]
fn authoring_example_4() -> Result<(), Box<dyn std::error::Error>> {
    use motion::prelude::*;
    use motion::render::RasterLimits;
    use motion::render::*;
    let rect = Rect::new(0.5, 0.0, 2.0, 1.0);
    let inner = Node::from(Rect::new(0.0, 0.0, 2.0, 1.0).fill(Color::WHITE))
        .clip_with(rect, motion::ClipMode::Geometric);
    let frame = Node::group([inner])
        .clip_with(rect, motion::ClipMode::Geometric)
        .compile(
            Size::new(2.0, 1.0),
            PixelSize::new(2, 1),
            ValidationLimits::default(),
        )?
        .prepare(ValidationLimits::default(), RasterLimits::default())?
        .render_linear(RasterLimits::default())?;
    assert_eq!(frame.linear_pixel(0, 0), Some([128.0 / 255.0; 4]));
    assert_eq!(frame.linear_pixel(1, 0), Some([1.0; 4]));
    Ok(())
}

#[test]
fn authoring_example_5() -> Result<(), Box<dyn std::error::Error>> {
    use motion::prelude::*;
    use motion::render::RasterLimits;
    use motion::render::*;
    let triangle = Path2D::polyline([Point::ORIGIN, Point::new(1.0, 0.0), Point::new(0.0, 1.0)])?;
    let inner = Node::from(Rect::new(0.0, 0.0, 1.0, 1.0).fill(Color::WHITE))
        .clip_with(triangle.clone(), motion::ClipMode::Geometric);
    let frame = Node::group([inner])
        .clip_with(triangle, motion::ClipMode::Geometric)
        .compile(
            Size::new(1.0, 1.0),
            PixelSize::new(1, 1),
            ValidationLimits::default(),
        )?
        .prepare(ValidationLimits::default(), RasterLimits::default())?
        .render_linear(RasterLimits::default())?;
    assert_eq!(frame.linear_pixel(0, 0), Some([128.0 / 255.0; 4]));
    Ok(())
}

#[test]
fn authoring_example_6() -> Result<(), Box<dyn std::error::Error>> {
    use motion::prelude::*;
    use motion::render::RasterLimits;
    use motion::render::*;
    let tile = |color| Node::from(Rect::new(0.0, 0.0, 1.0, 1.0).fill(color));
    let node = tile(Color::WHITE).mask(
        tile(Color::from_hex(0xff0000).with_alpha(0.5)),
        MaskMode::Luminance,
    );
    let frame = node
        .compile(
            Size::new(1.0, 1.0),
            PixelSize::new(1, 1),
            ValidationLimits::default(),
        )?
        .prepare(ValidationLimits::default(), RasterLimits::default())?;
    assert_eq!(
        frame
            .render_linear(RasterLimits::default())?
            .linear_pixel(0, 0),
        Some([0.1063; 4])
    );
    Ok(())
}

#[test]
fn authoring_example_7() -> Result<(), Box<dyn std::error::Error>> {
    use motion::prelude::*;
    use motion::render::RasterLimits;
    use motion::render::*;
    let source = Node::from(Rect::new(1.0, 0.0, 3.0, 1.0).fill(Color::WHITE));
    let mask = Node::from(Rect::new(1.0, 0.0, 2.0, 1.0).fill(Color::WHITE));
    let frame = source
        .mask(mask, MaskMode::InvertedAlpha)
        .compile(
            Size::new(4.0, 1.0),
            PixelSize::new(4, 1),
            ValidationLimits::default(),
        )?
        .prepare(ValidationLimits::default(), RasterLimits::default())?
        .render_linear(RasterLimits::default())?;
    assert_eq!(
        frame.linear_pixels().collect::<Vec<_>>(),
        [[0.0; 4], [0.0; 4], [1.0; 4], [0.0; 4]]
    );
    Ok(())
}

#[test]
fn authoring_example_8() -> Result<(), Box<dyn std::error::Error>> {
    use motion::prelude::*;
    use motion::render::*;

    let composition = Composition::new(8, 4, motion::render::FrameClock::new(30, 3)).with_options(
        motion::render::RenderOptions::default().motion_blur(MotionBlur {
            shutter_angle: 180.0,
            samples: 4,
        }),
    )?;
    let frame = composition.render_frame_sampled(1, Size::new(8.0, 4.0), |context| {
        Node::from(
            Rect::new(context.seconds() * 4.0, 0.0, 4.0, 4.0).fill(Color::from_hex(0x24a49b)),
        )
    })?;
    assert_eq!(frame.size().width as usize, 8);
    // A whole-frame shutter and a subtree shutter are never combined.
    let nested = composition.render_frame_sampled(1, Size::new(8.0, 4.0), |context| {
        context
            .try_motion_blur(2.milliseconds(), 2, |sample| {
                Node::from(
                    Rect::new(sample.seconds() * 4.0, 0.0, 4.0, 4.0)
                        .fill(Color::from_hex(0x24a49b)),
                )
            })
            .expect("bounded finite local shutter")
    });
    assert!(nested.is_err());
    Ok(())
}

#[test]
fn authoring_example_9() -> Result<(), Box<dyn std::error::Error>> {
    use motion::prelude::*;
    use motion::render::RasterLimits;
    use motion::render::*;

    let child = (|context: &SceneContext<'_>| {
        assert_eq!(context.time(), 1.seconds().after_origin());
        assert_eq!(context.global_time(), 2.seconds().after_origin());
        Node::from(Rect::new(0.0, 0.0, 32.0, 24.0).fill(Color::rgb(20, 150, 125)))
    })
    .with_duration(2.seconds());
    let cache = SceneCache::new(0);
    let context = SceneContext::new(
        2.seconds().after_origin(),
        FrameRate::from(30),
        Size::new(640.0, 360.0),
        &cache,
    );
    let node = context.during(Time::ZERO, 4.seconds(), |local| {
        local.embed(&child, 1.seconds().after_origin())
    });
    let frame = node
        .compile(
            context.size(),
            PixelSize::new(64, 36),
            ValidationLimits::default(),
        )?
        .prepare(ValidationLimits::default(), RasterLimits::default())?
        .render_linear(RasterLimits::default())?;
    assert_eq!(frame.size(), PixelSize::new(64, 36));
    Ok(())
}

#[test]
fn authoring_example_10() -> Result<(), Box<dyn std::error::Error>> {
    use motion::authoring::{Scene as _, timeline::Timeline};
    use motion::prelude::*;
    use motion::render::*;

    let tile =
        (|_: &SceneContext<'_>| Node::from(Rect::new(0.0, 0.0, 8.0, 4.0).fill(Color::WHITE)))
            .with_duration(1.seconds());
    let timeline = Timeline::try_new(Size::new(8.0, 4.0), 30)?
        .try_track(|track| track.try_push(tile)?.try_gap(500.milliseconds()))?
        .build()?;
    assert_eq!(timeline.frame_count(), 45);
    let frame = Composition::new(
        8,
        4,
        FrameClock::new(timeline.frame_rate(), timeline.frame_count()),
    )
    .render_frame(0, timeline.size(), |context| timeline.render(context))?;
    assert!(frame.packed_rgb().iter().all(|pixel| *pixel == 0xffffff));
    Ok(())
}

#[test]
fn authoring_example_11() -> Result<(), Box<dyn std::error::Error>> {
    use motion::authoring::Scene as _;

    use motion::prelude::*;
    use motion::render::*;

    let first = (|context: &SceneContext<'_>| Node::from(context.bounds().fill(Color::WHITE)))
        .with_duration(1.seconds());
    let next = (|context: &SceneContext<'_>| Node::from(context.bounds().fill(Color::BLACK)))
        .with_duration(1.seconds());
    let timeline = Timeline::try_new(Size::new(8.0, 4.0), 30)?
        .try_track(|track| {
            track
                .try_push(first)?
                .try_transition(CrossFade, 500.milliseconds())?
                .try_push(next)
        })?
        .build()?;
    assert_eq!(timeline.frame_count(), 45);
    Composition::new(8, 4, motion::render::FrameClock::new(30, 45)).render_frame(
        20,
        timeline.size(),
        |context| timeline.render(context),
    )?;
    Ok(())
}

#[test]
fn authoring_example_12() -> Result<(), Box<dyn std::error::Error>> {
    use motion::prelude::*;
    use motion::render::RasterLimits;
    use motion::render::*;
    let path = Path2D::from_rect(Rect::new(0.0, 0.0, 1.0, 1.0))?;
    let shape: &dyn Shape = &path;
    let mut clip = ShapeClip::new(shape)?;
    clip.rule = FillRule::EvenOdd;
    clip.transform = Transform::translate(0.5, 0.0);
    let frame = Node::from(Rect::new(0.0, 0.0, 2.0, 1.0).fill(Color::WHITE))
        .clip_with(clip, motion::ClipMode::Geometric)
        .compile(
            Size::new(2.0, 1.0),
            PixelSize::new(2, 1),
            ValidationLimits::default(),
        )?
        .prepare(ValidationLimits::default(), RasterLimits::default())?
        .render_linear(RasterLimits::default())?;
    assert_eq!(
        frame.linear_pixels().collect::<Vec<_>>(),
        [[128.0 / 255.0; 4]; 2]
    );
    Ok(())
}

#[test]
fn authoring_example_13() -> Result<(), Box<dyn std::error::Error>> {
    use motion::prelude::*;
    use motion::render::*;

    let brush = LinearGradient::new(Point::ORIGIN, Point::new(1.0, 0.0))
        .stop(0.0, Color::from_hex(0x1080c0))
        .stop(1.0, Color::WHITE)
        .space(GradientSpace::Oklab);
    let image = Composition::new(8, 8, motion::render::FrameClock::new(30, 150))
        .with_options(motion::render::RenderOptions::default().blend_space(BlendSpace::Srgb))?
        .render_frame(0, Size::new(8.0, 8.0), |_| {
            Node::from(Rect::new(0.0, 0.0, 8.0, 8.0).fill(brush.clone()))
        })?;
    assert_eq!(image.size().width as usize, 8);
    Ok(())
}

#[test]
fn authoring_example_14() -> Result<(), Box<dyn std::error::Error>> {
    use motion::prelude::*;
    use motion::render::RasterLimits;
    use motion::render::*;
    let outline = Rect::new(1.0, 1.0, 7.0, 7.0)
        .fill(Color::WHITE)
        .stroke(2.0, Color::BLACK)
        .stroke_style(StrokeStyle {
            width: 2.0,
            cap: LineCap::Round,
            join: LineJoin::Round,
        });
    let list = Node::from(outline).opacity(0.5).compile(
        Size::new(8.0, 8.0),
        PixelSize::new(8, 8),
        ValidationLimits::default(),
    )?;
    let frame = list
        .prepare(ValidationLimits::default(), RasterLimits::default())?
        .render_linear(RasterLimits::default())?;
    assert_eq!(frame.linear_pixel(1, 1).unwrap()[3], 0.5);
    Ok(())
}

#[test]
fn authoring_example_15() -> Result<(), Box<dyn std::error::Error>> {
    use motion::prelude::*;
    use motion::render::RasterLimits;
    use motion::render::*;
    let path = Path2D::parse_svg("M1 1h6v6H1z")?;
    assert_eq!(path, Path2D::from_rect(Rect::new(1.0, 1.0, 7.0, 7.0))?);
    let frame = Node::from(path.fill(Color::WHITE))
        .compile(
            Size::new(8.0, 8.0),
            PixelSize::new(8, 8),
            ValidationLimits::default(),
        )?
        .prepare(ValidationLimits::default(), RasterLimits::default())?
        .render_linear(RasterLimits::default())?;
    assert_eq!(frame.linear_pixel(2, 2), Some([1.0; 4]));
    Ok(())
}

#[test]
fn authoring_example_16() -> Result<(), Box<dyn std::error::Error>> {
    use motion::prelude::*;
    use motion::render::*;

    let curve = Spline::through([
        Point::new(1.0, 6.0),
        Point::new(4.0, 1.0),
        Point::new(7.0, 6.0),
    ])?
    .stroke(0.5, Color::WHITE)
    .trim(0.0, 0.8)
    .dash(&[1.0, 0.5], -0.2);
    let frame = Composition::new(8, 8, motion::render::FrameClock::new(30, 150))
        .with_options(motion::render::RenderOptions::default().background(Color::BLACK))?
        .render_frame(0, Size::new(8.0, 8.0), |_| Node::from(curve.clone()))?;
    assert_eq!(frame.size().width as usize, 8);
    Ok(())
}

#[test]
fn authoring_example_17() -> Result<(), Box<dyn std::error::Error>> {
    use motion::prelude::*;
    use motion::render::*;
    let curve = Spline::through([
        Point::new(1.0, 6.0),
        Point::new(4.0, 1.0),
        Point::new(7.0, 6.0),
    ])?
    .stroke(0.5, Color::WHITE)
    .trim(0.0, 0.8)
    .dash(&[1.0, 0.5], -0.2);
    let frame = Composition::new(8, 8, motion::render::FrameClock::new(30, 150))
        .with_options(motion::render::RenderOptions::default().background(Color::BLACK))?
        .render_frame(0, Size::new(8.0, 8.0), |_| Node::from(curve.clone()))?;
    assert_eq!(frame.size().width as usize, 8);
    Ok(())
}
