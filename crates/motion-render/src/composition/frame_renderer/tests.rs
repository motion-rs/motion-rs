use super::*;
use crate::raster::prepare::RenderPrepare;
use crate::{RenderContext, Renderer};
use motion_core::Shape;
use motion_core::ValidationErrorKind;
use motion_core::{Color, Filter, NodeModifiers, Rect, ValidationLimits};

#[cfg(feature = "gpu")]
#[test]
#[ignore = "requires native GPU; checks public ten-bit packing and unaligned fallback without timings"]
fn public_yuv10_output_uses_packing_and_preserves_unaligned_fallback() -> Result {
    let format = crate::PixelFormat::Yuv420 { bit_depth: 10 };
    for width in [512, 514] {
        let size = Size::new(width as f64, 6.0);
        let source = |context: &SceneContext<'_>| {
            Node::from(
                context.bounds().fill(
                    motion_core::LinearGradient::new(
                        motion_core::Point::ORIGIN,
                        motion_core::Point::new(1.0, 0.0),
                    )
                    .stop(0.0, Color::BLACK)
                    .stop(1.0, Color::WHITE),
                ),
            )
        };
        let base = Composition::new(width, 6, crate::FrameClock::new(motion_core::fps!(24), 2));
        let mut cpu = base
            .clone()
            .configure(|options| options.backend(Renderer::Cpu.into()))
            .frame_renderer(size)?;
        let mut gpu = base
            .configure(|options| options.backend(Renderer::Gpu.into()))
            .frame_renderer(size)?;
        let expected = cpu.render_output_at(Time::ZERO, Some(Color::BLACK), format, source)?;
        let actual = gpu.render_output_at(Time::ZERO, Some(Color::BLACK), format, source)?;
        assert_eq!(actual.format(), format);
        assert_eq!(actual.byte_len(), width * 6 * 3);
        for (actual, expected) in actual.planes().iter().zip(expected.planes()) {
            assert_eq!(actual.stride(), expected.stride());
            assert_eq!(actual.bytes().len(), expected.bytes().len());
            for (actual, expected) in actual
                .bytes()
                .as_chunks::<2>()
                .0
                .iter()
                .zip(expected.bytes().as_chunks::<2>().0)
            {
                assert!(u16::from_le_bytes(*actual).abs_diff(u16::from_le_bytes(*expected)) <= 1);
            }
        }
        assert!(
            actual.planes()[0]
                .bytes()
                .as_chunks::<2>()
                .0
                .iter()
                .map(|sample| u16::from_le_bytes(*sample))
                .collect::<std::collections::BTreeSet<_>>()
                .len()
                > 256
        );
        assert_eq!(
            actual,
            gpu.render_output_at(Time::ZERO, Some(Color::BLACK), format, source)?
        );
        if width == 512 {
            let stats = gpu.last_stats().unwrap();
            assert_eq!(stats.linear.peak_bytes, stats.output.peak_bytes);
        }
    }
    Ok(())
}

#[test]
fn explicit_requests_preserve_exact_time_project_diagnostics_and_atomic_targets() -> Result {
    use motion_core::TimeLiteral;
    let assets = motion_core::Assets::new(".")?;
    let pool = assets.store().clone();

    struct Missing;

    impl motion_core::Asset for Missing {
        const KIND: &'static str = "request-test";

        fn decode(_: &[u8], _: &motion_core::LoadContext) -> Result<Self> {
            Ok(Self)
        }
    }
    let missing = assets.load::<Missing>("missing-runtime-request.bin");
    let size = Size::new(4.0, 2.0);
    let limits = RasterLimits::default();
    let mut session = RenderSession::new(
        size,
        crate::RenderOptions {
            size: Some(PixelSize::new(4, 2)),
            resources: Some(assets),
            ..crate::RenderOptions::default()
        },
    )?;
    let mut target = crate::FrameBuffer::new(PixelSize::new(4, 2), limits)?;

    fn request(target: &mut crate::FrameBuffer) -> FrameRequest<'_> {
        FrameRequest {
            at: FrameAt::Time(1.milliseconds().after_origin()),
            frame_rate: motion_core::FrameRate::NTSC,
            duration: 1.seconds(),
            background: None,
            target: crate::RenderTarget::Memory(target),
            format: crate::PixelFormat::SrgbRgba8 {
                premultiplied: false,
            },
            device_packing: false,
        }
    }
    session.render(request(&mut target), |context| {
        assert_eq!(context.time(), 1.milliseconds().after_origin());
        assert_eq!(context.clip_duration(), Some(1.seconds()));
        assert!(context.diagnostics().is_some());
        assert!(context.assets().store().same_store(&pool));
        Node::from(context.bounds().fill(Color::WHITE))
    })?;
    let previous = target.clone();
    assert!(
        session
            .render(request(&mut target), |context| {
                let _ = missing.get_with(context.diagnostics().unwrap());
                Node::empty()
            })
            .is_err()
    );
    assert_eq!(target, previous);
    assert!(session.last_stats().is_none());
    assert_eq!(session.asset_diagnostics().len(), 1);
    session.render(request(&mut target), |_| Node::empty())?;
    assert!(session.asset_diagnostics().is_empty());
    assert!(session.diagnostics().errors().is_empty());
    assert!(target.planes()[0].bytes().iter().all(|byte| *byte == 0));
    let duration =
        motion_core::Duration::from_frame(u64::from(u32::MAX), motion_core::FrameRate::NTSC);
    session.render(
        FrameRequest {
            duration,
            ..request(&mut target)
        },
        |context| {
            assert_eq!(context.clip_duration(), Some(duration));
            Node::empty()
        },
    )?;
    Ok(())
}

#[test]
fn global_shutter_averages_complete_frames_and_preserves_old_once_contract() -> Result {
    use motion_core::TimeLiteral;
    let design = Size::new(4.0, 2.0);
    for space in [
        motion_core::BlendSpace::LinearRgb,
        motion_core::BlendSpace::Srgb,
    ] {
        let base = Composition::new(4, 2, crate::FrameClock::new(motion_core::fps!(1), 3))
            .with_options(
                crate::RenderOptions::default()
                    .background(motion_core::Color::BLACK)
                    .blend_space(space),
            )?;
        let configured = base.clone().configure(|options| {
            options.motion_blur(motion_core::MotionBlur {
                shutter_angle: 360.0,
                samples: 4,
            })
        });
        let mut observed = Vec::new();
        let result = configured.render_frame_sampled(1, design, |context| {
            observed.push((
                context.time(),
                context.global_time(),
                context.clip_duration(),
            ));
            Node::from(
                context
                    .bounds()
                    .fill(if context.time() < 1.seconds().after_origin() {
                        Color::rgb(255, 0, 0).with_alpha(0.5)
                    } else {
                        Color::rgb(0, 0, 255).with_alpha(0.5)
                    }),
            )
        })?;
        assert_eq!(
            observed.iter().map(|sample| sample.0).collect::<Vec<_>>(),
            [
                625.milliseconds().after_origin(),
                875.milliseconds().after_origin(),
                1125.milliseconds().after_origin(),
                1375.milliseconds().after_origin()
            ]
        );
        assert!(
            observed
                .iter()
                .all(|(local, global, duration)| local == global && *duration == Some(3.seconds()))
        );
        let expected = base.render_frame(1, design, |context| {
            Node::accumulate(
                [
                    Color::rgb(255, 0, 0),
                    Color::rgb(255, 0, 0),
                    Color::rgb(0, 0, 255),
                    Color::rgb(0, 0, 255),
                ]
                .map(|color| {
                    Node::group([
                        Node::from(context.bounds().fill(Color::BLACK)),
                        Node::from(context.bounds().fill(color.with_alpha(0.5))),
                    ])
                }),
            )
            .unwrap()
        })?;
        assert_eq!(result.packed_rgb(), expected.packed_rgb());
        let mut calls = 0;
        assert!(
            configured
                .render_frame(1, design, |_| {
                    calls += 1;
                    Node::empty()
                })
                .is_err()
        );
        assert_eq!(calls, 0);
        assert!(configured.frame_renderer(design).is_ok());
        configured.render_frame_sampled(0, design, |context| {
            assert!(context.time() >= Time::ZERO);
            calls += 1;
            Node::empty()
        })?;
        assert_eq!(calls, 2);
    }
    Ok(())
}

fn composition() -> Composition {
    Composition::new(8, 4, crate::FrameClock::new(motion_core::fps!(24), 12))
        .with_options(crate::RenderOptions::default().background(Color::rgb(20, 40, 60)))
        .unwrap()
}

fn tile() -> Node {
    Rect::new(0.0, 0.0, 4.0, 4.0).fill(Color::WHITE).into()
}

fn session() -> FrameRenderer {
    composition().frame_renderer(Size::new(8.0, 4.0)).unwrap()
}

fn diagnostic(error: motion_core::Error) -> ValidationError {
    error.downcast_ref::<ValidationError>().unwrap().clone()
}

#[test]
fn rational_clock_is_exact_for_fractional_rates() -> Result {
    for rate in [
        motion_core::FrameRate::NTSC_FILM,
        motion_core::FrameRate::NTSC,
        motion_core::FrameRate::NTSC_DOUBLE,
        motion_core::FrameRate::try_new(11, 1).unwrap(),
        motion_core::FrameRate::try_new(123_457, 1_001).unwrap(),
    ] {
        let composition = composition().with_clock(crate::FrameClock::new(rate, 1001));
        assert_eq!(composition.frame_rate(), rate);
        let mut session = composition.frame_renderer(Size::new(8.0, 4.0))?;
        for index in [1000, 1, 0, 501, 1] {
            session.render_frame(index, |context| {
                assert_eq!(context.frame_rate(), rate);
                assert_eq!(context.time(), Time::from_frame(i64::from(index), rate));
                assert_eq!(context.time().to_frame(rate), i64::from(index));
                assert_eq!(context.global_time(), context.time());
                assert_eq!(
                    context.clip_duration(),
                    Some(motion_core::Duration::from_frame(1001, rate))
                );
                assert_eq!(
                    context.frames(12),
                    motion_core::Duration::from_frame(12, rate)
                );
                tile()
            })?;
        }
    }
    let composition = composition().with_clock(crate::FrameClock::new(motion_core::fps!(24), 150));
    assert_eq!(composition.frame_rate(), motion_core::fps!(24));
    for (rate, frames, field) in [
        (
            motion_core::FrameRate::try_new(u32::MAX, 1).unwrap(),
            1,
            "frame_rate",
        ),
        (
            motion_core::FrameRate::try_new(1, u32::MAX).unwrap(),
            u32::MAX,
            "frame_count",
        ),
    ] {
        let error = composition
            .clone()
            .with_clock(crate::FrameClock::new(rate, u64::from(frames)))
            .render_frame(0, Size::new(8.0, 4.0), |_| {
                panic!("Invalid time must fail before evaluation")
            })
            .err()
            .unwrap();
        assert_eq!(diagnostic(error).field, field);
    }
    Ok(())
}

#[test]
fn composition_frame_counts_are_u64_and_duration_is_checked() -> Result {
    let count = u64::from(u32::MAX) + 1;
    let composition =
        composition().with_clock(crate::FrameClock::new(motion_core::fps!(30), count));
    assert_eq!(composition.frame_count(), count);
    composition.preflight()?;
    let error = composition
        .with_clock(crate::FrameClock::new(motion_core::fps!(30), u64::MAX))
        .preflight()
        .unwrap_err();
    assert_eq!(error.field, "frame_count");
    Ok(())
}

#[test]
fn composition_raster_policy_is_inherited_and_overrides_are_local() -> Result {
    let original = composition();
    let configured_limits = RasterLimits {
        max_bytes: 512 * 1024 * 1024,
        ..RasterLimits::default()
    };
    let configured = original
        .clone()
        .configure(|options| options.limits(configured_limits));
    assert_eq!(
        original.raster_limits().max_bytes,
        RasterLimits::default().max_bytes
    );
    let size = Size::new(8.0, 4.0);
    let mut first = configured.frame_renderer(size)?;
    let second = configured.clone().frame_renderer(size)?;
    assert_eq!(first.limits.max_bytes, configured_limits.max_bytes);
    assert_eq!(second.limits.max_bytes, configured_limits.max_bytes);
    first.raster_limits(RasterLimits {
        max_bytes: 1,
        ..configured_limits
    })?;
    assert!(first.render_frame(0, |_| tile()).is_err());
    assert_eq!(
        configured.raster_limits().max_bytes,
        configured_limits.max_bytes
    );
    assert_eq!(second.limits.max_bytes, configured_limits.max_bytes);
    first.raster_limits(configured_limits)?;
    assert_eq!(
        first.render_frame(0, |_| tile())?.packed_rgb(),
        configured.render_frame(0, size, |_| tile())?.packed_rgb()
    );
    let denied = configured.clone().configure(|options| {
        options.limits(RasterLimits {
            max_bytes: 1,
            ..configured_limits
        })
    });
    assert!(denied.render_frame(0, size, |_| tile()).is_err());
    for invalid in [
        RasterLimits {
            max_pixels: 0,
            ..configured_limits
        },
        RasterLimits {
            max_pixels: 31,
            ..configured_limits
        },
        RasterLimits {
            max_bytes: 0,
            ..configured_limits
        },
        RasterLimits {
            max_work: 0,
            ..configured_limits
        },
    ] {
        assert!(
            configured
                .clone()
                .with_options(crate::RenderOptions::default().limits(invalid))
                .is_err()
        );
    }
    assert!(
        Composition::new(0, 4, crate::FrameClock::new(motion_core::fps!(30), 150))
            .with_options(crate::RenderOptions::default().limits(configured_limits))
            .is_err()
    );
    assert!(
        original
            .configure(|options| options.limits(configured_limits))
            .frame_renderer(size)?
            .gpu_info()
            .is_none()
    );
    Ok(())
}

#[test]
fn cpu_design_mapping_and_background_ignore_background_alpha() -> Result {
    let composition = Composition::new(8, 4, crate::FrameClock::new(motion_core::fps!(30), 150))
        .with_options(crate::RenderOptions::default().background(Color::rgba(0, 0, 0, 0)))?;
    let snapshot = composition.render_frame(0, Size::new(4.0, 4.0), |context| {
        assert_eq!(context.size(), Size::new(4.0, 4.0));
        tile()
    })?;
    for row in snapshot.packed_rgb().as_chunks::<8>().0 {
        assert_eq!(row, &[0, 0, 0xffffff, 0xffffff, 0xffffff, 0xffffff, 0, 0]);
    }
    Ok(())
}

#[test]
fn cpu_consumer_matches_direct_prepared_filters_clips_and_masks() -> Result {
    let node = Node::group([
        Node::from(Rect::new(-1.0, 1.0, 2.0, 3.0).fill(Color::from_rgb_float(2.0, -0.2, 0.5))),
        Node::from(Rect::new(1.0, 0.0, 5.0, 2.0).fill(Color::WHITE)).opacity(0.5),
    ])
    .filter(Filter::Blur { sigma: 0.5 })
    .clip_with(
        Rect::new(0.5, 0.0, 6.0, 4.0),
        motion_core::ClipMode::Geometric,
    )
    .mask(tile(), motion_core::MaskMode::InvertedAlpha)
    .opacity(0.7)
    .blend_mode(motion_core::BlendMode::Screen);
    let actual = session().render_frame(3, |_| node.clone())?;
    let list = node.compile(
        Size::new(8.0, 4.0),
        PixelSize::new(8, 4),
        ValidationLimits::default(),
    )?;
    let list = list.with_background(Some(Color::rgb(20, 40, 60)));
    let expected = list
        .prepare(ValidationLimits::default(), RasterLimits::default())?
        .compose_linear(RasterLimits::default())?
        .to_rgba8_frame(false, RasterLimits::default())?;
    let expected: Vec<_> = expected
        .pixels()
        .iter()
        .map(|pixel| u32::from(pixel[0]) << 16 | u32::from(pixel[1]) << 8 | u32::from(pixel[2]))
        .collect();
    assert_eq!(actual.packed_rgb(), expected);
    assert!(
        actual
            .packed_rgb()
            .iter()
            .all(|pixel| pixel & 0xff000000 == 0)
    );
    Ok(())
}

#[test]
fn cpu_repeated_and_out_of_order_frames_are_cache_independent() -> Result {
    use std::cell::Cell;
    let evaluations = Cell::new(0);
    let computations = Cell::new(0);
    let mut renderer = session();
    let mut first = None;
    for index in [5, 0, 11, 5] {
        let snapshot = renderer.render_frame(index, |context| {
            evaluations.set(evaluations.get() + 1);
            assert_eq!(
                context.time(),
                Time::from_frame(i64::from(index), motion_core::fps!(24))
            );
            assert_eq!(context.global_time(), context.time());
            let offset = context.cache("offset", || {
                computations.set(computations.get() + 1);
                1.0
            });
            tile().translate(*offset + f64::from(index % 3), 0.0)
        })?;
        if index == 5 {
            if let Some(expected) = &first {
                assert_eq!(snapshot.packed_rgb(), *expected);
            }
            first = Some(snapshot.packed_rgb());
        }
    }
    assert_eq!(evaluations.get(), 4);
    assert_eq!(computations.get(), 1);
    renderer.cache().clear();
    let snapshot = renderer.render_frame(5, |_| tile().translate(3.0, 0.0))?;
    assert_eq!(snapshot.packed_rgb(), first.unwrap());
    let single =
        composition().render_frame(5, Size::new(8.0, 4.0), |_| tile().translate(3.0, 0.0))?;
    assert_eq!(snapshot.packed_rgb(), single.packed_rgb());
    Ok(())
}

#[test]
fn cpu_shared_work_and_live_bytes_admit_exactly_then_reject_one_short() -> Result {
    let mut renderer = session();
    let build = |_: &SceneContext<'_>| tile().filter(Filter::Blur { sigma: 0.5 });
    let reference = renderer.render_frame(2, build)?;
    let stats = *renderer.last_stats().unwrap();
    assert_eq!(stats.output.work, 32);
    let exact = RasterLimits {
        max_bytes: stats.peak_bytes(),
        max_work: stats.work(),
        ..RasterLimits::default()
    };
    renderer.raster_limits(exact)?;
    assert_eq!(
        renderer.render_frame(2, build)?.packed_rgb(),
        reference.packed_rgb()
    );
    renderer.raster_limits(RasterLimits {
        max_work: exact.max_work - 1,
        ..exact
    })?;
    let error = diagnostic(renderer.render_frame(2, build).err().unwrap());
    assert_eq!(error.kind, ValidationErrorKind::StructuralLimit);
    assert_eq!((error.field.as_str(), error.frame), ("max_work", Some(2)));
    assert_eq!(error.path, "linear.output");
    assert!(renderer.last_stats().is_none());
    renderer.raster_limits(RasterLimits {
        max_bytes: exact.max_bytes - 1,
        ..exact
    })?;
    assert_eq!(
        diagnostic(renderer.render_frame(2, build).err().unwrap()).field,
        "max_bytes"
    );
    renderer.raster_limits(exact)?;
    assert_eq!(
        renderer.render_frame(2, build)?.packed_rgb(),
        reference.packed_rgb()
    );
    let single_stage = stats
        .preparation
        .work
        .max(stats.linear.work)
        .max(stats.output.work);
    renderer.raster_limits(RasterLimits {
        max_work: single_stage,
        ..exact
    })?;
    assert_eq!(
        diagnostic(renderer.render_frame(2, build).err().unwrap()).field,
        "max_work"
    );
    Ok(())
}

#[test]
fn cpu_failures_tag_the_requested_frame_and_preserve_session_reuse() -> Result {
    let mut renderer = session();
    renderer.render_frame(1, |_| tile())?;
    let invalid = diagnostic(
        renderer
            .render_frame(7, |_| tile().opacity(f64::NAN))
            .err()
            .unwrap(),
    );
    assert_eq!(
        (invalid.kind, invalid.field.as_str(), invalid.frame),
        (ValidationErrorKind::InvalidValue, "opacity", Some(7))
    );
    assert!(renderer.last_stats().is_none());

    struct Unsupported;

    impl motion_core::Drawable for Unsupported {
        fn bounds(
            &self,
            _: &motion_core::DrawContext,
        ) -> std::result::Result<motion_core::Rect, motion_core::ValidationError> {
            Ok(motion_core::Rect::new(0.0, 0.0, 1.0, 1.0))
        }

        fn draw(
            &self,
            _: &motion_core::DrawContext,
            _: &mut motion_core::Canvas<'_>,
        ) -> std::result::Result<(), motion_core::ValidationError> {
            Err(motion_core::ValidationError::unsupported(
                "native",
                "content",
                "Fixture unsupported drawable",
            ))
        }
    }
    let unsupported_frame = diagnostic(
        renderer
            .render_frame(6, |_| Node::from(Unsupported))
            .err()
            .unwrap(),
    );
    assert_eq!(
        (unsupported_frame.kind, unsupported_frame.frame),
        (ValidationErrorKind::Unsupported, Some(6))
    );
    renderer.raster_limits(RasterLimits {
        max_pixels: 32,
        ..RasterLimits::default()
    })?;
    let expanded = diagnostic(
        renderer
            .render_frame(8, |_| tile().filter(Filter::Blur { sigma: 1.0 }))
            .err()
            .unwrap(),
    );
    assert_eq!(
        (expanded.field.as_str(), expanded.frame),
        ("max_pixels", Some(8))
    );
    let output = renderer.render_frame(0, |_| tile())?;
    assert_eq!(output.packed_rgb()[0], 0xffffff);
    Ok(())
}

#[test]
fn cpu_context_policy_and_failed_limit_updates_do_not_fall_back() -> Result {
    let context = RenderContext::new().with_validation_limits(ValidationLimits {
        max_nodes: 1,
        ..ValidationLimits::default()
    })?;
    let mut renderer = composition()
        .configure(|options| options.context(context))
        .frame_renderer(Size::new(8.0, 4.0))?;
    assert_eq!(renderer.context().validation_limits().max_nodes, 1);
    let error = diagnostic(
        renderer
            .render_frame(3, |_| Node::group([tile(), tile()]))
            .err()
            .unwrap(),
    );
    assert_eq!((error.field.as_str(), error.frame), ("max_nodes", Some(3)));
    assert!(
        renderer
            .raster_limits(RasterLimits {
                max_work: 0,
                ..RasterLimits::default()
            })
            .is_err()
    );
    assert!(
        renderer
            .raster_limits(RasterLimits {
                max_bytes: 0,
                ..RasterLimits::default()
            })
            .is_err()
    );
    assert!(
        renderer
            .raster_limits(RasterLimits {
                max_pixels: 31,
                ..RasterLimits::default()
            })
            .is_err()
    );
    assert_eq!(renderer.limits.max_work, RasterLimits::default().max_work);
    Ok(())
}

#[test]
fn cpu_invalid_configuration_and_index_fail_before_evaluation() -> Result {
    for composition in [
        Composition::new(0, 4, crate::FrameClock::new(motion_core::fps!(30), 150)),
        Composition::new(8, 4, crate::FrameClock::new(motion_core::fps!(30), 0)),
    ] {
        assert!(
            composition
                .render_frame(0, Size::new(8.0, 4.0), |_| panic!("must not evaluate"))
                .is_err()
        );
    }
    for design in [
        Size::new(0.0, 4.0),
        Size::new(f64::NAN, 4.0),
        Size::new(8.0, f64::INFINITY),
    ] {
        assert!(
            composition()
                .render_frame(0, design, |_| panic!("must not evaluate"))
                .is_err()
        );
    }
    let mut renderer = session();
    renderer.render_frame(0, |_| tile())?;
    assert!(
        renderer
            .render_frame(12, |_| panic!("must not evaluate"))
            .is_err()
    );
    assert!(renderer.last_stats().is_none());
    Ok(())
}

#[test]
fn gpu_selection_is_explicit_and_lazy_without_a_device() -> Result {
    let selected = composition()
        .configure(|options| options.backend(Renderer::Gpu.into()))
        .frame_renderer(Size::new(8.0, 4.0));
    #[cfg(not(feature = "gpu"))]
    {
        let error = diagnostic(selected.err().unwrap());
        assert_eq!(
            (error.kind, error.field.as_str(), error.frame),
            (ValidationErrorKind::Unsupported, "renderer", None)
        );
    }
    #[cfg(feature = "gpu")]
    {
        let mut selected = selected?;
        assert_eq!(selected.renderer(), Renderer::Gpu);
        assert!(selected.gpu_info().is_none());
        assert!(
            selected
                .render_frame(12, |_| panic!("index precedes callback"))
                .is_err()
        );
        let error = diagnostic(
            selected
                .render_frame(3, |_| tile().opacity(f64::NAN))
                .err()
                .unwrap(),
        );
        assert_eq!(
            (error.kind, error.frame),
            (ValidationErrorKind::InvalidValue, Some(3))
        );
        let error = diagnostic(
            selected
                .render_frame(2, |_| tile().opacity(1e-50))
                .err()
                .unwrap(),
        );
        assert_eq!(
            (error.kind, error.field.as_str(), error.frame),
            (ValidationErrorKind::Unsupported, "precision", Some(2))
        );
        assert!(selected.gpu_info().is_none());
        selected.raster_limits(RasterLimits {
            max_work: 1,
            ..RasterLimits::default()
        })?;
        let error = diagnostic(selected.render_frame(4, |_| tile()).err().unwrap());
        assert_eq!((error.field.as_str(), error.frame), ("max_work", Some(4)));
        assert!(selected.gpu_info().is_none());
        assert!(selected.last_stats().is_none());
    }
    Ok(())
}

#[test]
#[cfg(feature = "gpu")]
#[ignore = "requires separate authorization to initialize native GPU devices"]
fn gpu_device_session_seek_budget_and_recovery_match_cpu() -> Result {
    for (width, height) in [(8, 4), (65, 37), (127, 73)] {
        let composition = Composition::new(
            width,
            height,
            crate::FrameClock::new(motion_core::fps!(24), 12),
        )
        .with_options(
            crate::RenderOptions::default().background(motion_core::Color::rgb(20, 40, 60)),
        )?;
        let mut cpu = composition.frame_renderer(Size::new(8.0, 4.0))?;
        let mut gpu = composition
            .configure(|options| options.backend(Renderer::Gpu.into()))
            .frame_renderer(Size::new(8.0, 4.0))?;
        assert!(gpu.gpu_info().is_none());
        let mut first = None;
        let mut max_delta = 0_u32;
        for index in [5, 0, 11, 5] {
            let build = |context: &SceneContext<'_>| {
                assert_eq!(
                    context.time(),
                    Time::from_frame(i64::from(index), motion_core::fps!(24))
                );
                Node::group([
                    tile().translate(f64::from(index % 3) * 0.25, 0.0),
                    tile().translate(2.0, 0.5).opacity(0.5),
                ])
                .filter(Filter::Blur { sigma: 0.3 })
                .clip_with(
                    Rect::new(0.5, 0.25, 7.5, 3.75),
                    motion_core::ClipMode::Geometric,
                )
                .mask(
                    tile().translate(1.0, 0.0),
                    motion_core::MaskMode::InvertedAlpha,
                )
                .blend_mode(motion_core::BlendMode::Screen)
                .opacity(0.7)
            };
            let expected = cpu.render_frame(index, build)?;
            let actual = gpu.render_frame(index, build)?;
            let mut large = 0;
            for (expected, actual) in expected.packed_rgb().iter().zip(actual.packed_rgb()) {
                let mut delta = 0;
                for shift in [0, 8, 16] {
                    delta =
                        delta.max(((expected >> shift) & 255).abs_diff((actual >> shift) & 255));
                }
                max_delta = max_delta.max(delta);
                assert!(
                    delta <= 2,
                    "{width}x{height} frame {index}: {actual:x} != {expected:x}"
                );
                large += usize::from(delta > 1);
            }
            assert!((large as f64) / (actual.packed_rgb().len() as f64) < 0.001);
            if index == 5 {
                if let Some(first) = &first {
                    assert_eq!(actual.packed_rgb(), *first);
                }
                first = Some(actual.packed_rgb());
            }
        }
        let info = gpu.gpu_info().expect("actual GPU initialization required");
        #[cfg(target_os = "macos")]
        assert_eq!(info.backend, crate::GpuBackend::Metal);
        println!(
            "_gpu_session size={width}x{height} adapter={} backend={:?} seeks=4 max_rgb_difference={max_delta}",
            info.name, info.backend
        );
        let build = |_: &SceneContext<'_>| tile().filter(Filter::Blur { sigma: 0.5 });
        let expected = gpu.render_frame(2, build)?;
        let stats = *gpu.last_stats().unwrap();
        let exact = RasterLimits {
            max_bytes: stats.peak_bytes(),
            max_work: stats.work(),
            ..RasterLimits::default()
        };
        gpu.raster_limits(exact)?;
        assert_eq!(
            gpu.render_frame(2, build)?.packed_rgb(),
            expected.packed_rgb()
        );
        for limits in [
            RasterLimits {
                max_bytes: exact.max_bytes - 1,
                ..exact
            },
            RasterLimits {
                max_work: exact.max_work - 1,
                ..exact
            },
        ] {
            gpu.raster_limits(limits)?;
            let error = diagnostic(gpu.render_frame(2, build).err().unwrap());
            assert_eq!(
                (error.kind, error.frame),
                (ValidationErrorKind::StructuralLimit, Some(2))
            );
            assert!(gpu.last_stats().is_none());
            assert_eq!(gpu.gpu_info().unwrap().name, info.name);
            gpu.raster_limits(exact)?;
            assert_eq!(
                gpu.render_frame(2, build)?.packed_rgb(),
                expected.packed_rgb()
            );
        }
        gpu.raster_limits(RasterLimits::default())?;
        let invalid = diagnostic(
            gpu.render_frame(7, |_| tile().opacity(f64::NAN))
                .err()
                .unwrap(),
        );
        assert_eq!(
            (invalid.kind, invalid.frame),
            (ValidationErrorKind::InvalidValue, Some(7))
        );
        assert!(gpu.last_stats().is_none());
        assert_eq!(
            gpu.render_frame(2, build)?.packed_rgb(),
            expected.packed_rgb()
        );
        println!("_gpu_session size={width}x{height} exact_budget=pass one_short=pass retry=pass");
    }
    Ok(())
}

#[test]
fn cpu_builder_panics_propagate_without_stale_totals() -> Result {
    let mut renderer = session();
    renderer.render_frame(0, |_| tile())?;
    let panic = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        renderer.render_frame(0, |_| panic!("builder failure"))
    }));
    assert!(panic.is_err());
    assert!(renderer.last_stats().is_none());
    assert!(renderer.render_frame(0, |_| tile()).is_ok());
    Ok(())
}
