use super::*;
use motion_core::{NodeModifiers, Shape, ValidationLimits};

mod noise_tests;

fn assert_packed_output_matches(actual: &crate::FrameBuffer, expected: &crate::FrameBuffer) {
    assert_eq!(actual.size(), expected.size());
    assert_eq!(actual.format(), expected.format());
    assert_eq!(actual.planes().len(), expected.planes().len());
    for (index, (actual_plane, expected_plane)) in
        actual.planes().iter().zip(expected.planes()).enumerate()
    {
        assert_eq!(actual_plane.width(), expected_plane.width());
        assert_eq!(actual_plane.height(), expected_plane.height());
        assert_eq!(actual_plane.stride(), expected_plane.stride());
        assert_eq!(actual_plane.bytes().len(), expected_plane.bytes().len());
        let samples = |plane: &crate::PixelPlane| -> Vec<u16> {
            if actual.format() == (crate::PixelFormat::Yuv420 { bit_depth: 10 }) {
                assert!(plane.bytes().as_chunks::<2>().1.is_empty());
                plane
                    .bytes()
                    .as_chunks::<2>()
                    .0
                    .iter()
                    .map(|sample| u16::from_le_bytes(*sample))
                    .collect()
            } else {
                plane
                    .bytes()
                    .iter()
                    .map(|sample| u16::from(*sample))
                    .collect()
            }
        };
        for (offset, (actual, expected)) in samples(actual_plane)
            .into_iter()
            .zip(samples(expected_plane))
            .enumerate()
        {
            assert!(
                actual.abs_diff(expected) <= 1,
                "plane {index}, sample {offset}: {actual} != {expected}"
            );
        }
    }
}

#[test]
#[ignore = "requires native GPU; Bloom pyramid output parity, no performance measurements"]
fn bloom_pyramid_cpu_gpu_parity() -> motion_core::Result {
    use motion_core::{Color, Filter, Node, Rect, Size};
    let mut gpu = Gpu::new()?;
    let limits = RasterLimits::default();
    // Bright highlights at odd offsets exercise outward rounding between pyramid levels.
    let source = || {
        Node::group([
            Rect::new(0.0, 0.0, 48.0, 36.0)
                .fill(Color::rgb(10, 20, 40))
                .into(),
            Node::from(Rect::new(13.0, 9.0, 18.0, 12.0).fill(Color::WHITE)).filter(
                Filter::ColorAdjust {
                    exposure: 4.0,
                    contrast: 1.0,
                    saturation: 1.0,
                    hue_degrees: 0.0,
                },
            ),
            Node::from(Rect::new(31.0, 21.0, 33.0, 27.0).fill(Color::rgb(255, 160, 60))).filter(
                Filter::ColorAdjust {
                    exposure: 2.5,
                    contrast: 1.0,
                    saturation: 1.0,
                    hue_degrees: 0.0,
                },
            ),
        ])
    };
    for (levels, radius, intensity, tint) in [
        (1, 2.0, 0.6, Color::WHITE),
        (2, 1.5, 1.4, Color::rgb(140, 200, 255)),
        (5, 1.0, 0.8, Color::WHITE),
        (5, 3.0, 1.2, Color::rgb(255, 220, 180)),
    ] {
        let filter = Filter::Bloom {
            threshold: 0.8,
            knee: 0.3,
            radius,
            levels,
            falloff: 0.6,
            intensity,
            tint,
        };
        let prepared = source()
            .filter(filter.clone())
            .compile(
                Size::new(48.0, 36.0),
                PixelSize::new(48, 36),
                ValidationLimits::default(),
            )?
            .prepare(ValidationLimits::default(), limits)?;
        let cpu = prepared.compose_linear(limits)?;
        let actual = gpu.render(&prepared, Plan::new(&prepared, limits)?)?;
        let mut glow = 0.0;
        for (cpu, actual) in cpu.pixels().iter().zip(actual.pixels()) {
            glow += cpu[0];
            for channel in 0..4 {
                assert!(
                    (cpu[channel] - actual[channel]).abs() <= 1e-4 * cpu[channel].abs().max(1.0),
                    "{filter:?}: {cpu:?} != {actual:?}"
                );
            }
        }
        assert!(glow.is_finite() && glow > 0.0);
    }
    Ok(())
}

#[test]
#[ignore = "requires native GPU; cinematic output parity, no performance measurements"]
fn cinematic_display_and_optics_cpu_gpu_parity() -> motion_core::Result {
    use motion_core::{
        ApertureShape, Cdl, Color, ColorWheels, Filter, FilterOperation, HeightFrom, LutInput,
        Node, Optics, Point, Rect, Size, Transform,
    };
    let mut gpu = Gpu::new()?;
    let limits = RasterLimits::default();
    let source = || {
        Node::group([
            Rect::new(0.0, 0.0, 16.0, 16.0)
                .fill(Color::rgb(20, 80, 100))
                .into(),
            Rect::new(6.0, 5.0, 10.0, 11.0)
                .fill(Color::WHITE)
                .filter(Filter::ColorAdjust {
                    exposure: 3.0,
                    contrast: 1.0,
                    saturation: 1.0,
                    hue_degrees: 0.0,
                }),
        ])
    };
    let lut = std::sync::Arc::new(ColorWheels::default().bake(3, 16.0, Cdl::default())?);
    let shaped = std::sync::Arc::new((*lut).clone().with_shaper(motion_core::Lut1d::new(
        [0.0; 3],
        [16.0; 3],
        vec![[0.0; 3], [0.7, 0.4, 0.9], [1.0; 3]],
    )?));
    let filters = [
        Filter::Bloom {
            threshold: 0.9,
            knee: 0.2,
            radius: 1.0,
            levels: 3,
            falloff: 0.5,
            intensity: 0.2,
            tint: Color::WHITE,
        },
        Filter::Optics(Optics::Lens {
            distortion: 0.05,
            chromatic: 0.01,
            auto_fit: true,
            center: None,
        }),
        Filter::Optics(Optics::LensVignette {
            angle_of_view: 50.0,
            strength: 0.5,
            roundness: 0.5,
        }),
        Filter::Optics(Optics::Bokeh {
            radius: 2.0,
            shape: ApertureShape::Polygon {
                sides: 6,
                rotation: 0.2,
            },
            highlight_boost: 0.3,
        }),
        Filter::Optics(Optics::RadialBlur {
            center: Point::new(8.0, 8.0),
            strength: 0.4,
            samples: 8,
            decay: 0.8,
        }),
        Filter::Optics(Optics::Anamorphic {
            threshold: 0.5,
            length: 3.0,
            stretch: 6.0,
            tint: Color::rgb(90, 170, 255),
            intensity: 0.7,
        }),
        Filter::Optics(Optics::Relight {
            height_from: HeightFrom::Alpha,
            light_dir: [0.3, 0.2, 1.0],
            color: Color::WHITE,
            ambient: 0.2,
            specular: 0.1,
        }),
        Filter::Optics(Optics::LensFlare {
            center: Point::new(8.0, 8.0),
            ghosts: 3,
            threshold: 0.9,
            intensity: 0.2,
            tint: Color::WHITE,
        }),
        Filter::Custom(vec![
            FilterOperation::Transform(Transform::translate(0.25, 0.5)),
            FilterOperation::BlurXY {
                sigma_x: 2.0,
                sigma_y: 0.3,
            },
        ]),
        Filter::Lut {
            lut: lut.clone(),
            input: LutInput::Linear,
        },
        Filter::Lut {
            lut: shaped.clone(),
            input: LutInput::Linear,
        },
        Filter::Cdl(Cdl {
            slope: [1.1, 0.9, 1.0],
            ..Cdl::default()
        }),
    ];
    for filter in filters {
        let prepared = source()
            .filter(filter.clone())
            .compile(
                Size::new(16.0, 16.0),
                PixelSize::new(16, 16),
                ValidationLimits::default(),
            )?
            .prepare(ValidationLimits::default(), limits)?;
        let cpu = prepared.compose_linear(limits)?;
        let actual = gpu.render(&prepared, Plan::new(&prepared, limits)?)?;
        for (cpu, actual) in cpu.pixels().iter().zip(actual.pixels()) {
            for channel in 0..4 {
                assert!(
                    (cpu[channel] - actual[channel]).abs() <= 1e-4 * cpu[channel].abs().max(1.0),
                    "{filter:?}: {cpu:?} != {actual:?}"
                );
            }
        }
    }
    let prepared = source()
        .compile(
            Size::new(16.0, 16.0),
            PixelSize::new(16, 16),
            ValidationLimits::default(),
        )?
        .prepare(ValidationLimits::default(), limits)?;
    let linear = prepared.compose_linear(limits)?;
    for tone_map in [
        crate::ToneMap::PbrNeutral,
        crate::ToneMap::Aces,
        crate::ToneMap::AgX,
        crate::ToneMap::Hable { white: 11.2 },
    ] {
        let mut display = crate::DisplayTransform {
            tone_map,
            grain: Some(crate::FilmGrain {
                seed: 42,
                amount: 0.02,
                size: 1.5,
                luma_response: 1.0,
                chroma: 0.2,
            }),
            look_lut: Some(shaped.clone()),
            display_lut: Some(std::sync::Arc::new(
                ColorWheels::default()
                    .bake(3, 1.0, Cdl::default())?
                    .with_shaper(motion_core::Lut1d::new(
                        [0.0; 3],
                        [1.0; 3],
                        vec![[0.0; 3], [0.6, 0.3, 0.4], [1.0; 3]],
                    )?),
            )),
            ..crate::DisplayTransform::default()
        };
        for transfer in [crate::OutputTransfer::Srgb, crate::OutputTransfer::Gamma24] {
            display.transfer = transfer;
            display.reference_white = Some(1.0);
            for format in [
                crate::PixelFormat::SrgbRgba8 {
                    premultiplied: false,
                },
                crate::PixelFormat::SrgbRgba8 {
                    premultiplied: true,
                },
                crate::PixelFormat::Yuv420 { bit_depth: 8 },
                crate::PixelFormat::Yuv420 { bit_depth: 10 },
            ] {
                let cpu =
                    crate::FrameBuffer::from_linear_display(&linear, format, limits, &display, 7)?;
                let actual = gpu.render_output(
                    &prepared,
                    Plan::new(&prepared, limits)?.with_display(&display, 7)?,
                    format,
                )?;
                assert_packed_output_matches(&actual, &cpu);
            }
        }
    }
    Ok(())
}

#[test]
#[ignore = "requires native GPU; shaper encodings and tiled lookup, no timing"]
fn shaper_input_encodings_cpu_gpu_parity() -> motion_core::Result {
    use motion_core::{Cdl, Color, ColorWheels, Filter, Lut1d, LutInput, Node, Rect, Size};
    let mut gpu = Gpu::new()?;
    let limits = RasterLimits::default();
    let lut = std::sync::Arc::new(
        ColorWheels::default()
            .bake(3, 1.0, Cdl::default())?
            .with_shaper(Lut1d::new(
                [0.0; 3],
                [1.0; 3],
                vec![[0.0; 3], [0.8, 0.2, 0.4], [1.0; 3]],
            )?),
    );
    for input in [
        LutInput::Linear,
        LutInput::SrgbEncoded,
        LutInput::Log {
            minimum: -8.0,
            maximum: 4.0,
        },
    ] {
        let node = Node::group((0..65).map(|index| {
            Rect::new(f64::from(index), 0.0, f64::from(index + 1), 17.0)
                .fill(Color::rgb((index * 4).min(255) as u8, 90, 180))
                .filter(Filter::Lut {
                    lut: lut.clone(),
                    input,
                })
        }))
        .translate(0.25, 0.125);
        let prepared = node
            .compile(
                Size::new(65.0, 17.0),
                PixelSize::new(65, 17),
                ValidationLimits::default(),
            )?
            .prepare(ValidationLimits::default(), limits)?;
        let cpu = prepared.compose_linear(limits)?;
        let actual = gpu.render(&prepared, Plan::new(&prepared, limits)?)?;
        for (cpu, actual) in cpu.pixels().iter().zip(actual.pixels()) {
            for channel in 0..4 {
                assert!(
                    (cpu[channel] - actual[channel]).abs() <= 2e-4 * cpu[channel].abs().max(1.0),
                    "{input:?}: {cpu:?} != {actual:?}"
                );
            }
        }
        let replay = gpu.render(&prepared, Plan::new(&prepared, limits)?)?;
        assert_eq!(actual.pixels(), replay.pixels());
    }
    Ok(())
}

#[test]
fn timestamp_capabilities_need_only_pass_level_queries() {
    assert!(super::super::context::timestamp_features(wgpu::Features::empty()).is_empty());
    assert!(
        super::super::context::timestamp_features(wgpu::Features::TIMESTAMP_QUERY_INSIDE_ENCODERS)
            .is_empty()
    );
    for available in [
        wgpu::Features::TIMESTAMP_QUERY,
        wgpu::Features::TIMESTAMP_QUERY | wgpu::Features::TIMESTAMP_QUERY_INSIDE_ENCODERS,
        wgpu::Features::TIMESTAMP_QUERY | wgpu::Features::IMMEDIATES,
    ] {
        assert_eq!(
            super::super::context::timestamp_features(available),
            wgpu::Features::TIMESTAMP_QUERY
        );
    }
}

#[test]
fn shared_noise_shader_validates_without_a_device() {
    let source = include_str!("../shaders/noise.wgsl");
    let module = wgpu::naga::front::wgsl::parse_str(source)
        .unwrap_or_else(|error| panic!("{}", error.emit_to_string(source)));
    wgpu::naga::valid::Validator::new(
        wgpu::naga::valid::ValidationFlags::all(),
        wgpu::naga::valid::Capabilities::empty(),
    )
    .validate(&module)
    .unwrap();
}

#[test]
fn present_shader_validates_without_a_device() {
    use wgpu::naga;
    let module = naga::front::wgsl::parse_str(&crate::gpu::output_shader(include_str!(
        "../shaders/present.wgsl"
    )))
    .expect("present shader must parse");
    naga::valid::Validator::new(
        naga::valid::ValidationFlags::all(),
        naga::valid::Capabilities::all(),
    )
    .validate(&module)
    .expect("present shader must validate");
}

#[test]
#[ignore = "requires native GPU; checks timestamp isolation and output without latency assertions"]
fn native_frame_timestamps_preserve_bytes_and_inflight_results() -> motion_core::Result {
    let mut gpu = Gpu::new()?;
    let limits = RasterLimits::default();
    let format = crate::PixelFormat::SrgbRgba8 {
        premultiplied: false,
    };
    let prepare = |background| -> motion_core::Result<PreparedFrame> {
        Ok(motion_core::Node::empty()
            .compile(
                motion_core::Size::new(8.0, 8.0),
                PixelSize::new(8, 8),
                ValidationLimits::default(),
            )?
            .with_background(Some(background))
            .prepare(ValidationLimits::default(), limits)?)
    };
    let first = prepare(motion_core::Color::BLACK)?;
    let second = prepare(motion_core::Color::WHITE)?;
    let expected_linear = gpu.render(&first, Plan::new(&first, limits)?)?;
    let expected_output = gpu.render_output(&second, Plan::new(&second, limits)?, format)?;
    assert!(gpu.timing_handle().is_none());
    gpu.enable_timings(true);
    let supported = gpu.context().supports_timestamps();
    let actual_linear = gpu.render(&first, Plan::new(&first, limits)?)?;
    assert_eq!(actual_linear.pixels(), expected_linear.pixels());
    assert_eq!(actual_linear.stats(), expected_linear.stats());
    assert_eq!(gpu.timings().is_some(), supported);

    let pending_linear = gpu.submit_linear(&first, Plan::new(&first, limits)?)?;
    let linear_handle = pending_linear.timing_handle();
    let pending_output = gpu.submit_output(&second, Plan::new(&second, limits)?, format)?;
    let output_handle = pending_output.timing_handle();
    assert!(
        linear_handle
            .as_ref()
            .and_then(GpuTimingHandle::get)
            .is_none()
    );
    assert!(
        output_handle
            .as_ref()
            .and_then(GpuTimingHandle::get)
            .is_none()
    );
    let output = pending_output.finish()?;
    assert_eq!(
        output.planes()[0].bytes(),
        expected_output.planes()[0].bytes()
    );
    assert_eq!(
        output_handle
            .as_ref()
            .and_then(GpuTimingHandle::get)
            .is_some(),
        supported
    );
    assert!(
        linear_handle
            .as_ref()
            .and_then(GpuTimingHandle::get)
            .is_none()
    );
    let linear = pending_linear.finish()?;
    assert_eq!(linear.pixels(), expected_linear.pixels());
    assert_eq!(
        linear_handle
            .as_ref()
            .and_then(GpuTimingHandle::get)
            .is_some(),
        supported
    );

    let dropped = gpu.submit_output(&second, Plan::new(&second, limits)?, format)?;
    let dropped_handle = dropped.timing_handle();
    drop(dropped);
    let texture = gpu
        .context()
        .device()
        .create_texture(&wgpu::TextureDescriptor {
            label: Some("Timestamp test target"),
            size: wgpu::Extent3d {
                width: 8,
                height: 8,
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: wgpu::TextureFormat::Rgba8Unorm,
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT,
            view_formats: &[],
        });
    let pending_texture =
        gpu.submit_texture(&first, Plan::new(&first, limits)?, &texture, format)?;
    let texture_handle = pending_texture.timing_handle();
    pending_texture.finish()?;
    assert_eq!(
        texture_handle
            .as_ref()
            .and_then(GpuTimingHandle::get)
            .is_some(),
        supported
    );
    assert!(
        dropped_handle
            .as_ref()
            .and_then(GpuTimingHandle::get)
            .is_none()
    );
    let mut invalid = Plan::new(&first, limits)?;
    invalid.steps[0].parameters.values[0] = f32::INFINITY;
    let failed = gpu.submit_output(&first, invalid, format)?;
    let failed_handle = failed.timing_handle();
    assert!(failed.finish().is_err());
    assert!(
        failed_handle
            .as_ref()
            .and_then(GpuTimingHandle::get)
            .is_none()
    );
    gpu.enable_timings(false);
    assert!(gpu.timing_handle().is_none());
    let replay = gpu.render_output(&second, Plan::new(&second, limits)?, format)?;
    assert_eq!(
        replay.planes()[0].bytes(),
        expected_output.planes()[0].bytes()
    );
    Ok(())
}

#[test]
#[ignore = "requires native GPU; verifies single planning and retained-target admission"]
fn packed_memory_target_plans_instance_upload_once() -> motion_core::Result {
    use motion_core::instances::{
        InstanceBatch, InstanceComposite, InstanceRecord, InstanceUpload,
    };
    use std::sync::{
        Arc,
        atomic::{AtomicUsize, Ordering},
    };

    #[derive(Debug)]
    struct CountedBatch(Arc<AtomicUsize>);

    impl InstanceBatch for CountedBatch {
        fn len(&self) -> usize {
            1
        }

        fn bounds(&self) -> motion_core::Rect {
            motion_core::Rect::new(1.0, 1.0, 7.0, 7.0)
        }

        fn bytes(&self) -> u64 {
            size_of::<Self>() as u64
        }

        fn work(&self) -> u64 {
            1
        }

        fn validate(&self) -> Result<(), ValidationError> {
            Ok(())
        }

        fn sample(
            &self,
            _: motion_core::Point,
            _: f64,
            _: [f32; 4],
            _: motion_core::BlendSpace,
            _: InstanceComposite,
        ) -> Result<[f32; 4], ValidationError> {
            unreachable!("GPU-only fixture")
        }

        fn upload_bytes(&self) -> u64 {
            64
        }

        fn upload(&self) -> Result<Option<InstanceUpload>, ValidationError> {
            self.0.fetch_add(1, Ordering::Relaxed);
            InstanceUpload::new(
                [InstanceRecord {
                    position: [4.0, 4.0],
                    start: [4.0, 4.0],
                    color: [1.0, 0.0, 0.0, 1.0],
                    radius: 2.0,
                    rotation: 0.0,
                    width: 0.0,
                    kind: motion_core::instances::InstanceKind::Disc,
                }]
                .into_iter(),
                [0, 0],
                [],
            )
            .map(Some)
        }

        fn as_any(&self) -> &dyn std::any::Any {
            self
        }

        fn content_eq(&self, other: &dyn InstanceBatch) -> bool {
            other
                .as_any()
                .downcast_ref::<Self>()
                .is_some_and(|other| Arc::ptr_eq(&self.0, &other.0))
        }
    }

    let context = GpuContext::new()?;
    let limits = RasterLimits::default();
    let uploads = Arc::new(AtomicUsize::new(0));
    let mut frame = motion_core::Node::empty()
        .compile(
            motion_core::Size::new(8.0, 8.0),
            PixelSize::new(8, 8),
            motion_core::ValidationLimits::default(),
        )?
        .prepare(motion_core::ValidationLimits::default(), limits)?;
    frame.items.push(PreparedItem::ParticleInstances {
        batch: Arc::new(CountedBatch(uploads.clone())),
        transform: motion_core::Transform::IDENTITY,
    });
    let format = crate::PixelFormat::SrgbRgba8 {
        premultiplied: false,
    };
    let mut target = crate::FrameBuffer::new(frame.size(), limits)?;
    for _ in 0..2 {
        uploads.store(0, Ordering::Relaxed);
        context.render(
            &frame,
            crate::RenderTarget::Memory(&mut target),
            format,
            limits,
        )?;
        assert_eq!(uploads.load(Ordering::Relaxed), 1);
        assert_eq!(
            &target.planes()[0].bytes()[(4 * 8 + 4) * 4..][..4],
            &[255, 0, 0, 255]
        );
    }
    let previous = target.clone();
    let tight = RasterLimits {
        max_bytes: target.retained_bytes(),
        ..limits
    };
    uploads.store(0, Ordering::Relaxed);
    let error = context
        .render(
            &frame,
            crate::RenderTarget::Memory(&mut target),
            format,
            tight,
        )
        .unwrap_err();
    assert_eq!(
        error.downcast_ref::<ValidationError>().unwrap().field,
        "max_bytes"
    );
    assert_eq!(uploads.load(Ordering::Relaxed), 0);
    assert_eq!(target, previous);
    Ok(())
}

#[test]
#[ignore = "requires native GPU; validates shared coverage leases without performance assertions"]
fn direct_coverage_payloads_remain_independent_inflight() -> motion_core::Result {
    let limits = RasterLimits::default();
    let mut first = motion_core::Node::from(
        motion_core::Rect::new(0.25, 0.5, 31.5, 63.5).fill(motion_core::Color::WHITE),
    )
    .compile(
        motion_core::Size::new(32.0, 64.0),
        PixelSize::new(32, 64),
        ValidationLimits::default(),
    )?
    .prepare(ValidationLimits::default(), limits)?;
    let mut second = first.clone();
    first.alphas.fill(183);
    second.alphas.fill(37);
    let format = crate::PixelFormat::SrgbRgba8 {
        premultiplied: false,
    };
    let expected_first =
        crate::FrameBuffer::from_linear(&first.compose_linear(limits)?, format, limits)?;
    let expected_second =
        crate::FrameBuffer::from_linear(&second.compose_linear(limits)?, format, limits)?;
    assert_ne!(
        expected_first.planes()[0].bytes(),
        expected_second.planes()[0].bytes()
    );
    let mut gpu = Gpu::new()?;
    let first_plan = Plan::new(&first, limits)?;
    let coverage_bytes = first_plan.coverage.len() as u64 * 4;
    assert!(coverage_bytes > 0);
    let pending_first = gpu.submit_output(&first, first_plan, format)?;
    let pending_second = gpu.submit_output(&second, Plan::new(&second, limits)?, format)?;
    assert_eq!(pending_first.working[2].size(), coverage_bytes);
    assert_eq!(pending_second.working[2].size(), coverage_bytes);
    assert_ne!(pending_first.working[2], pending_second.working[2]);
    let actual_second = pending_second.finish()?;
    let actual_first = pending_first.finish()?;
    assert_eq!(
        actual_first.planes()[0].bytes(),
        expected_first.planes()[0].bytes()
    );
    assert_eq!(
        actual_second.planes()[0].bytes(),
        expected_second.planes()[0].bytes()
    );
    let replay = gpu.render_output(&first, Plan::new(&first, limits)?, format)?;
    assert_eq!(replay.planes()[0].bytes(), actual_first.planes()[0].bytes());
    Ok(())
}

#[test]
#[ignore = "requires native GPU; verifies final texture leases, cropping and dropped submissions"]
fn native_present_retains_textures_without_linear_output_buffer() -> motion_core::Result {
    let context = GpuContext::new()?;
    let mut gpu = Gpu::from_context(context.clone());
    let limits = RasterLimits::default();
    let frame = motion_core::Node::from(
        motion_core::Rect::new(0.25, 0.5, 15.75, 7.5).fill(motion_core::Color::WHITE),
    )
    .compile(
        motion_core::Size::new(16.0, 8.0),
        PixelSize::new(16, 8),
        ValidationLimits::default(),
    )?
    .prepare(ValidationLimits::default(), limits)?;
    for output_bounds in [
        frame.output_bounds,
        PixelRect {
            left: 3,
            top: 2,
            right: 13,
            bottom: 7,
        },
    ] {
        let mut frame = frame.clone();
        frame.output_bounds = output_bounds;
        let size = frame.size();
        let target = context.device.create_texture(&wgpu::TextureDescriptor {
            label: Some("Native present lease fixture"),
            size: wgpu::Extent3d {
                width: size.width,
                height: size.height,
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: wgpu::TextureFormat::Rgba8Unorm,
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT,
            view_formats: &[],
        });
        let format = crate::PixelFormat::SrgbRgba8 {
            premultiplied: true,
        };
        let pending = gpu.submit_texture(&frame, Plan::new(&frame, limits)?, &target, format)?;
        assert!(pending.working.buffers.iter().all(|buffer| {
            buffer.usage() != wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_SRC
        }));
        assert!(context.present_check.get().is_some());
        pending.finish()?;
        drop(gpu.submit_texture(&frame, Plan::new(&frame, limits)?, &target, format)?);
        gpu.submit_texture(&frame, Plan::new(&frame, limits)?, &target, format)?
            .finish()?;
    }
    let cloned = context.clone();
    assert!(std::ptr::eq(
        context.present_check_pipeline(),
        cloned.present_check_pipeline()
    ));
    context.check_device()?;
    Ok(())
}

#[test]
#[ignore = "requires native GPU; validates present finite checks and deferred alpha with explicit texture data"]
fn native_present_checks_nonfinite_inside_and_outside_crop() -> motion_core::Result {
    use wgpu::util::DeviceExt;
    let context = GpuContext::new()?;
    for (texture_format, linear) in [
        (wgpu::TextureFormat::Rgba8Unorm, false),
        (wgpu::TextureFormat::Bgra8Unorm, false),
        (wgpu::TextureFormat::Rgba16Float, true),
    ] {
        for (channel, value) in [
            (0, 0.25),
            (0, f32::NAN),
            (1, f32::INFINITY),
            (2, f32::NEG_INFINITY),
            (3, f32::INFINITY),
            (0, 65520.0),
        ] {
            for index in [1, 12] {
                for scan in [false, true] {
                    let mut pixels = vec![[0.25_f32, 0.5, 0.75, 2.0]; 32];
                    pixels[index][channel] = value;
                    let source = context.device.create_texture_with_data(
                        &context.queue,
                        &wgpu::TextureDescriptor {
                            label: Some("Present finite fixture source"),
                            size: wgpu::Extent3d {
                                width: 16,
                                height: 2,
                                depth_or_array_layers: 1,
                            },
                            mip_level_count: 1,
                            sample_count: 1,
                            dimension: wgpu::TextureDimension::D2,
                            format: wgpu::TextureFormat::Rgba32Float,
                            usage: wgpu::TextureUsages::TEXTURE_BINDING,
                            view_formats: &[],
                        },
                        wgpu::util::TextureDataOrder::LayerMajor,
                        bytemuck::cast_slice(&pixels),
                    );
                    let target = context.device.create_texture(&wgpu::TextureDescriptor {
                        label: Some("Present finite fixture target"),
                        size: wgpu::Extent3d {
                            width: 8,
                            height: 2,
                            depth_or_array_layers: 1,
                        },
                        mip_level_count: 1,
                        sample_count: 1,
                        dimension: wgpu::TextureDimension::D2,
                        format: texture_format,
                        usage: wgpu::TextureUsages::RENDER_ATTACHMENT,
                        view_formats: &[],
                    });
                    let mut words = vec![3_u32, 5, 8, 2, 3, 5, 19, 7, u32::from(linear), 1, 1, 0];
                    words.extend([0; 32]);
                    let settings =
                        context
                            .device
                            .create_buffer_init(&wgpu::util::BufferInitDescriptor {
                                label: None,
                                contents: bytemuck::cast_slice(&words),
                                usage: wgpu::BufferUsages::UNIFORM,
                            });
                    let status =
                        context
                            .device
                            .create_buffer_init(&wgpu::util::BufferInitDescriptor {
                                label: None,
                                contents: &[0; 4],
                                usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_SRC,
                            });
                    let readback = context.device.create_buffer(&wgpu::BufferDescriptor {
                        label: None,
                        size: 4,
                        mapped_at_creation: false,
                        usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
                    });
                    let source_view = source.create_view(&Default::default());
                    let lut =
                        context
                            .device
                            .create_buffer_init(&wgpu::util::BufferInitDescriptor {
                                label: None,
                                contents: bytemuck::cast_slice(&[0.0f32; 20]),
                                usage: wgpu::BufferUsages::STORAGE,
                            });
                    let entries = [
                        wgpu::BindGroupEntry {
                            binding: 0,
                            resource: wgpu::BindingResource::TextureView(&source_view),
                        },
                        wgpu::BindGroupEntry {
                            binding: 1,
                            resource: settings.as_entire_binding(),
                        },
                        wgpu::BindGroupEntry {
                            binding: 2,
                            resource: status.as_entire_binding(),
                        },
                        wgpu::BindGroupEntry {
                            binding: 5,
                            resource: lut.as_entire_binding(),
                        },
                    ];
                    let mut encoder = context.device.create_command_encoder(&Default::default());
                    if scan {
                        let pipeline = context.present_check_pipeline();
                        let bindings =
                            context
                                .device
                                .create_bind_group(&wgpu::BindGroupDescriptor {
                                    label: None,
                                    layout: &pipeline.get_bind_group_layout(0),
                                    entries: &entries[..3],
                                });
                        let mut pass = encoder.begin_compute_pass(&Default::default());
                        pass.set_pipeline(pipeline);
                        pass.set_bind_group(0, &bindings, &[]);
                        pass.dispatch_workgroups(2, 1, 1);
                    }
                    let pipeline = context.present_pipeline(texture_format);
                    let bindings = context
                        .device
                        .create_bind_group(&wgpu::BindGroupDescriptor {
                            label: None,
                            layout: &pipeline.get_bind_group_layout(0),
                            entries: &entries,
                        });
                    let target_view = target.create_view(&Default::default());
                    {
                        let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                            color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                                view: &target_view,
                                depth_slice: None,
                                resolve_target: None,
                                ops: wgpu::Operations {
                                    load: wgpu::LoadOp::Clear(wgpu::Color::TRANSPARENT),
                                    store: wgpu::StoreOp::Store,
                                },
                            })],
                            ..Default::default()
                        });
                        pass.set_pipeline(pipeline);
                        pass.set_bind_group(0, &bindings, &[]);
                        pass.draw(0..3, 0..1);
                    }
                    encoder.copy_buffer_to_buffer(&status, 0, &readback, 0, 4);
                    context.queue.submit(Some(encoder.finish()));
                    let (sender, receiver) = std::sync::mpsc::channel();
                    readback
                        .slice(..)
                        .map_async(wgpu::MapMode::Read, move |result| {
                            let _ = sender.send(result);
                        });
                    wait_for_gpu(&context.device, None)?;
                    receiver.recv().unwrap().unwrap();
                    let mapped = readback.slice(..).get_mapped_range().unwrap();
                    let failure = u32::from_le_bytes(mapped[..4].try_into().unwrap());
                    let expected = (!value.is_finite() && (index == 1 || scan))
                        || (linear && index == 1 && value.abs() > 65504.0);
                    assert_eq!(
                        failure,
                        u32::from(expected),
                        "{texture_format:?}, channel={channel}, value={value}, index={index}, scan={scan}"
                    );
                    drop(mapped);
                    readback.unmap();
                }
            }
        }
    }
    context.check_device()?;
    Ok(())
}

#[test]
fn linear_readback_preserves_crops_unaligned_fallback_and_error_paths() -> motion_core::Result {
    let output_bounds = PixelRect {
        left: 1,
        top: 1,
        right: 3,
        bottom: 3,
    };
    let mut source: Vec<[f32; 4]> = (0..12)
        .map(|index| [index as f32, -(index as f32), 0.25, 0.5])
        .collect();
    source[0][0] = f32::NAN;
    let expected = [source[5], source[6], source[9], source[10]];
    let words: Vec<[u32; 4]> = source
        .iter()
        .map(|pixel| pixel.map(|value| value.to_bits().to_le()))
        .collect();
    for (path, message) in [
        ("gpu.output", "Native GPU readback must remain finite"),
        ("gpu.pending", "Nonfinite GPU readback"),
    ] {
        let aligned = bytemuck::cast_slice(&words);
        assert_eq!(
            super::pending::linear_pixels(aligned, 4, output_bounds, path, "allocation", message)?,
            expected
        );
        let mut storage = vec![0_u32; source.len() * 4 + 1];
        let bytes = bytemuck::cast_slice_mut::<u32, u8>(&mut storage);
        bytes[1..1 + aligned.len()].copy_from_slice(aligned);
        let unaligned = &mut bytes[1..1 + aligned.len()];
        assert!(bytemuck::try_cast_slice::<u8, [f32; 4]>(unaligned).is_err());
        assert_eq!(
            super::pending::linear_pixels(
                unaligned,
                4,
                output_bounds,
                path,
                "allocation",
                message
            )?,
            expected
        );
        for value in [f32::NAN, f32::INFINITY, f32::NEG_INFINITY] {
            let offset = 5 * 16;
            unaligned[offset..offset + 4].copy_from_slice(&value.to_le_bytes());
            let error = super::pending::linear_pixels(
                unaligned,
                4,
                output_bounds,
                path,
                "allocation",
                message,
            )
            .unwrap_err();
            assert_eq!(error.path, path);
            assert_eq!(error.field, "color");
            assert_eq!(error.message, message);
            let mut invalid_source = words.clone();
            invalid_source[5][0] = value.to_bits().to_le();
            assert!(
                super::pending::linear_pixels(
                    bytemuck::cast_slice(&invalid_source),
                    4,
                    output_bounds,
                    path,
                    "allocation",
                    message
                )
                .is_err()
            );
        }
    }
    Ok(())
}

#[test]
#[ignore = "requires native GPU; verifies complete texture working images and saturated hardware Plus"]
fn texture_working_images_and_hardware_plus_preserve_frame_contracts() -> motion_core::Result {
    use motion_core::NodeModifiers;
    use std::sync::atomic::Ordering;
    let context = GpuContext::new()?;
    assert!(context.textures.blends.is_some());
    let mut gpu = Gpu::from_context(context.clone());
    let limits = RasterLimits::default();
    let rectangle = motion_core::Rect::new(0.25, 0.5, 31.5, 63.5);
    let mut nodes = Vec::new();
    for index in 0..12 {
        let node = motion_core::Node::from(
            rectangle.fill(motion_core::Color::rgb(25, 65, 120).with_alpha(0.2)),
        );
        nodes.push(if index % 3 == 0 {
            node.blend_mode(motion_core::BlendMode::Plus)
        } else {
            node
        });
    }
    nodes.push(
        motion_core::Node::from(
            rectangle.fill(motion_core::Color::rgb(190, 40, 100).with_alpha(0.3)),
        )
        .filter(motion_core::Filter::Blur { sigma: 0.5 })
        .mask(
            rectangle.fill(motion_core::Color::WHITE.with_alpha(0.8)),
            motion_core::MaskMode::Alpha,
        ),
    );
    let frame = motion_core::Node::group(nodes)
        .compile(
            motion_core::Size::new(32.0, 64.0),
            PixelSize::new(32, 64),
            motion_core::ValidationLimits::default(),
        )?
        .prepare(motion_core::ValidationLimits::default(), limits)?;
    let expected = frame
        .compose_linear(limits)?
        .to_rgba8_frame(false, limits)?;
    let actual = gpu.render(&frame, Plan::new(&frame, limits)?)?;
    let output = actual.to_rgba8_frame(false, limits)?;
    let mut maximum = 0;
    let mut over_one = 0;
    for (actual, expected) in output.pixels().iter().zip(expected.pixels()) {
        let difference = actual
            .iter()
            .zip(expected)
            .map(|(actual, expected)| actual.abs_diff(*expected))
            .max()
            .unwrap();
        maximum = maximum.max(difference);
        over_one += usize::from(difference > 1);
    }
    assert!(maximum <= 2 && over_one * 1000 < output.pixels().len());
    assert_eq!(
        context.color_image_buffers_encoded.load(Ordering::Relaxed),
        0,
        "Working color images must remain textures, including filter and mask boundaries"
    );
    assert!(context.hardware_plus_encoded.load(Ordering::Relaxed) >= 4);
    assert_eq!(
        gpu.render(&frame, Plan::new(&frame, limits)?)?.pixels(),
        actual.pixels()
    );
    for premultiplied in [false, true] {
        let format = crate::PixelFormat::SrgbRgba8 { premultiplied };
        let expected = crate::FrameBuffer::from_linear(&actual, format, limits)?;
        let pending = gpu.submit_output(&frame, Plan::new(&frame, limits)?, format)?;
        assert!(pending.working.buffers.iter().all(|buffer| {
            buffer.usage() != wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_SRC
        }));
        let output = pending.finish()?;
        assert_eq!(
            output,
            gpu.render_output(&frame, Plan::new(&frame, limits)?, format)?
        );
        assert!(
            output.planes()[0]
                .bytes()
                .iter()
                .zip(expected.planes()[0].bytes())
                .all(|(actual, expected)| actual.abs_diff(*expected) <= 1)
        );
    }
    let plus_before = context.hardware_plus_encoded.load(Ordering::Relaxed);
    let mut compute = context.clone();
    std::sync::Arc::make_mut(&mut compute.textures).blends = None;
    let mut fallback = Gpu::from_context(compute);
    let fallback_frame = fallback.render(&frame, Plan::new(&frame, limits)?)?;
    let fallback_rgba = fallback_frame.to_rgba8_frame(false, limits)?;
    let differences = fallback_rgba
        .pixels()
        .iter()
        .zip(expected.pixels())
        .map(|(actual, expected)| {
            actual
                .iter()
                .zip(expected)
                .map(|(actual, expected)| actual.abs_diff(*expected))
                .max()
                .unwrap()
        })
        .collect::<Vec<_>>();
    assert!(differences.iter().all(|difference| *difference <= 2));
    assert!(
        differences
            .iter()
            .filter(|difference| **difference > 1)
            .count()
            * 1000
            < differences.len()
    );
    assert_eq!(
        fallback
            .render(&frame, Plan::new(&frame, limits)?)?
            .pixels(),
        fallback_frame.pixels()
    );
    assert_eq!(
        context.hardware_plus_encoded.load(Ordering::Relaxed),
        plus_before
    );
    assert!(
        context
            .working_pool
            .lock()
            .unwrap()
            .buffers
            .iter()
            .all(|buffer| buffer.usage().contains(wgpu::BufferUsages::COPY_DST)),
        "Only upload-compatible buffers belong in the working cache"
    );
    Ok(())
}

#[test]
#[ignore = "requires native GPU; verifies batch textures feed layers without round trips"]
fn resident_batch_textures_skip_resolve_and_layer_recapture() -> motion_core::Result {
    use motion_core::NodeModifiers;
    use std::sync::atomic::Ordering;
    let context = GpuContext::new()?;
    assert!(context.textures.blends.is_some());
    let mut gpu = Gpu::from_context(context.clone());
    let limits = RasterLimits::default();
    let group = |color| {
        motion_core::Node::group((0..32).map(|index| {
            motion_core::Node::from(
                motion_core::Rect::new(0.25 + f64::from(index % 3) * 0.25, 0.5, 31.0, 63.0)
                    .fill(color),
            )
        }))
    };
    let node = motion_core::Node::group([
        group(motion_core::Color::rgb(230, 40, 90).with_alpha(0.125)).opacity(0.6),
        group(motion_core::Color::rgb(20, 200, 140).with_alpha(0.25))
            .opacity(0.4)
            .blend_mode(motion_core::BlendMode::Plus),
    ]);
    let frame = node
        .compile(
            motion_core::Size::new(32.0, 64.0),
            PixelSize::new(32, 64),
            motion_core::ValidationLimits::default(),
        )?
        .prepare(motion_core::ValidationLimits::default(), limits)?;
    let reference = frame
        .compose_linear(limits)?
        .to_rgba8_frame(false, limits)?;
    let actual = gpu.render(&frame, Plan::new(&frame, limits)?)?;
    let rgba = actual.to_rgba8_frame(false, limits)?;
    let mut maximum = 0_u8;
    let mut over_one = 0usize;
    for (actual, reference) in rgba.pixels().iter().zip(reference.pixels()) {
        let difference = actual
            .iter()
            .zip(reference)
            .map(|(actual, reference)| actual.abs_diff(*reference))
            .max()
            .unwrap();
        maximum = maximum.max(difference);
        over_one += usize::from(difference > 1);
    }
    assert!(maximum <= 2 && over_one * 1000 < rgba.pixels().len());
    let batches = context.fill_batches_encoded.load(Ordering::Relaxed);
    let resolves = context.batch_resolves_encoded.load(Ordering::Relaxed);
    let captures = context.layer_captures_encoded.load(Ordering::Relaxed);
    eprintln!(
        "resident_layers batches={batches} resolves={resolves} captures={captures} max_rgba={maximum}"
    );
    assert_eq!(batches, 2);
    assert_eq!(resolves, 0, "Layer sources should remain resident textures");
    assert_eq!(captures, 0, "Resident batch sources must not be recaptured");
    let created = context.working_textures_created.load(Ordering::Relaxed);
    let replay = gpu.render(&frame, Plan::new(&frame, limits)?)?;
    assert_eq!(replay.pixels(), actual.pixels());
    assert_eq!(
        context.working_textures_created.load(Ordering::Relaxed),
        created,
        "A replay must reuse its completed attachments"
    );
    let mut cases = Vec::new();
    for mode in [
        motion_core::MaskMode::Alpha,
        motion_core::MaskMode::InvertedAlpha,
        motion_core::MaskMode::Luminance,
        motion_core::MaskMode::InvertedLuminance,
    ] {
        cases.push(
            motion_core::Node::from(
                motion_core::Rect::new(8.25, 7.5, 26.5, 57.0)
                    .fill(motion_core::Color::rgb(30, 100, 235)),
            )
            .mask(
                group(motion_core::Color::from_rgb_float(1.5, -0.2, 0.7).with_alpha(0.2))
                    .translate(4.5, 3.25),
                mode,
            ),
        );
    }
    cases.push(
        group(motion_core::Color::from_rgb_float(-0.2, 1.7, 0.5).with_alpha(0.2))
            .opacity(0.7)
            .blend_mode(motion_core::BlendMode::Hue),
    );
    cases.push(
        group(motion_core::Color::rgb(30, 200, 170).with_alpha(0.2))
            .filter(motion_core::Filter::Blur { sigma: 0.5 }),
    );
    cases.push(
        group(motion_core::Color::rgb(30, 200, 170).with_alpha(0.2)).filter(
            motion_core::Filter::Custom(vec![motion_core::FilterOperation::CompositeWithSource {
                mode: motion_core::BlendMode::Difference,
            }]),
        ),
    );
    cases.push(
        group(motion_core::Color::from_rgb_float(1e8, -1e5, 0.125).with_alpha(0.2)).opacity(0.5),
    );
    for (index, node) in cases.into_iter().enumerate() {
        let frame = node
            .compile(
                motion_core::Size::new(32.0, 64.0),
                PixelSize::new(32, 64),
                motion_core::ValidationLimits::default(),
            )?
            .prepare(motion_core::ValidationLimits::default(), limits)?;
        let plan = Plan::new(&frame, limits)?;
        let batches_before = context.fill_batches_encoded.load(Ordering::Relaxed);
        let resolves_before = context.batch_resolves_encoded.load(Ordering::Relaxed);
        let captures_before = context.layer_captures_encoded.load(Ordering::Relaxed);
        let expected = frame.compose_linear(limits)?;
        let actual = gpu.render(&frame, plan)?;
        let output = actual.to_rgba8_frame(false, limits)?;
        let expected_rgba = expected.to_rgba8_frame(false, limits)?;
        let mut maximum = 0;
        let mut over_one = 0;
        for (pixel, reference) in output.pixels().iter().zip(expected_rgba.pixels()) {
            let difference = pixel
                .iter()
                .zip(reference)
                .map(|(pixel, reference)| pixel.abs_diff(*reference))
                .max()
                .unwrap();
            maximum = maximum.max(difference);
            over_one += usize::from(difference > 1);
        }
        assert!(
            maximum <= 2 && over_one * 1000 < output.pixels().len(),
            "resident case {index}: max={maximum}, over_one={over_one}"
        );
        let batches = context.fill_batches_encoded.load(Ordering::Relaxed) - batches_before;
        let resolves = context.batch_resolves_encoded.load(Ordering::Relaxed) - resolves_before;
        let captures = context.layer_captures_encoded.load(Ordering::Relaxed) - captures_before;
        assert_eq!(batches, usize::from(index != 7), "resident case {index}");
        assert_eq!(
            resolves, 0,
            "Layer and filter sources must remain textures: {index}"
        );
        assert_eq!(
            captures, 0,
            "Layer and filter sources must not be recaptured: {index}"
        );
        eprintln!(
            "resident_case={index} batches={batches} resolves={resolves} captures={captures} max_rgba={maximum}"
        );
        let created = context.working_textures_created.load(Ordering::Relaxed);
        let replay = gpu.render(&frame, Plan::new(&frame, limits)?)?;
        assert_eq!(replay.pixels(), actual.pixels());
        assert_eq!(
            context.working_textures_created.load(Ordering::Relaxed),
            created,
            "Captures and batch attachments must be reused on replay, case {index}"
        );
    }
    Ok(())
}

#[test]
#[ignore = "requires native GPU; checks real fill batching, per-frame RGBA tolerance and replay"]
fn hardware_fill_batches_preserve_rgba_tolerance_boundaries_and_pooling() -> motion_core::Result {
    use motion_core::NodeModifiers;
    use std::sync::atomic::Ordering;
    let context = GpuContext::new()?;
    assert!(
        context.textures.blends.is_some(),
        "This activation check requires RGBA32F hardware blending"
    );
    let mut gpu = Gpu::from_context(context.clone());
    let mut unbatched = context.clone();
    std::sync::Arc::make_mut(&mut unbatched.textures).blends = None;
    let mut unbatched = Gpu::from_context(unbatched);
    let limits = RasterLimits::default();
    let mut frames = Vec::new();
    for phase in 0..5 {
        let mut children = Vec::new();
        for index in 0..64 {
            let offset = ((index * 7 + phase * 3) % 16) as f64 * 0.25;
            let rect = motion_core::Rect::new(offset, offset * 0.5, 31.0 - offset, 62.0 - offset);
            let alpha = [1.0 / 65536.0, 0.0125, 0.37, 0.875][index % 4];
            let color = motion_core::Color::from_rgb_float(1.4, -0.15, (index % 7) as f64 / 6.0)
                .with_alpha(alpha);
            children.push(if phase == 4 {
                let mut path = motion_core::Path2D::new();
                for left in [0.25, 28.25] {
                    path.try_move_to(motion_core::Point::new(left, 0.5))?;
                    path.try_line_to(motion_core::Point::new(left + 1.0, 0.5))?;
                    path.try_line_to(motion_core::Point::new(left + 1.0, 63.5))?;
                    path.try_line_to(motion_core::Point::new(left, 63.5))?;
                    path.try_close()?;
                }
                if index % 3 == 0 {
                    motion_core::Node::from(
                        path.fill(
                            motion_core::LinearGradient::new(
                                motion_core::Point::ORIGIN,
                                motion_core::Point::new(1.0, 1.0),
                            )
                            .stop(0.0, color)
                            .stop(1.0, motion_core::Color::rgb(20, 210, 145).with_alpha(alpha)),
                        ),
                    )
                } else {
                    motion_core::Node::from(path.fill(color))
                }
            } else if index % 3 == 0 {
                motion_core::Node::from(
                    rect.fill(
                        motion_core::LinearGradient::new(
                            motion_core::Point::ORIGIN,
                            motion_core::Point::new(1.0, 1.0),
                        )
                        .stop(0.0, color)
                        .stop(1.0, motion_core::Color::rgb(20, 210, 145).with_alpha(alpha)),
                    ),
                )
            } else {
                motion_core::Node::from(rect.fill(color))
            });
        }
        let group = motion_core::Node::group(children);
        let node = match phase {
            0 => group,
            1 => group.clip_with(
                motion_core::Rect::new(0.5, 1.5, 30.0, 61.0),
                motion_core::ClipMode::Geometric,
            ),
            2 => group.filter(motion_core::Filter::Blur { sigma: 0.5 }).mask(
                motion_core::Rect::new(0.25, 0.5, 30.5, 62.0).fill(motion_core::Color::WHITE),
                motion_core::MaskMode::Alpha,
            ),
            3 => motion_core::Node::group([
                motion_core::Node::from(
                    motion_core::Rect::new(0.0, 0.0, 32.0, 64.0)
                        .fill(motion_core::Color::rgb(20, 90, 200)),
                ),
                group.opacity(0.8).blend_mode(motion_core::BlendMode::Plus),
                motion_core::Node::from(
                    motion_core::Rect::new(0.0, 0.0, 32.0, 64.0)
                        .fill(motion_core::Color::WHITE.with_alpha(0.05)),
                ),
            ]),
            _ => group,
        };
        frames.push(
            node.compile(
                motion_core::Size::new(32.0, 64.0),
                PixelSize::new(32, 64),
                motion_core::ValidationLimits::default(),
            )?
            .prepare(motion_core::ValidationLimits::default(), limits)?,
        );
    }
    let mut outputs = Vec::new();
    for (index, frame) in frames.iter().enumerate() {
        let before = context.fill_batches_encoded.load(Ordering::Relaxed);
        let plan = Plan::new(frame, limits)?;
        if index == 4 {
            let direct = plan
                .steps
                .iter()
                .filter(|step| step.data[0].to_bits() == DIRECT_COVERAGE)
                .count();
            assert!(
                direct >= 64,
                "Direct strip ranges did not activate: {direct}"
            );
            assert_eq!(plan.coverage[2] as usize, frame.alphas.len());
            let mut position = 0;
            let mut accumulated = 0;
            while position < plan.steps.len() {
                let range = plan.texture_run(position);
                accumulated += usize::from(
                    plan.texture_slots[plan.steps[position].first] == plan.texture_slots[range.end],
                );
                position = range.end;
            }
            assert!(
                accumulated > 0,
                "Hardware layer accumulation did not activate"
            );
        }
        let expected = frame.compose_linear(limits)?;
        let actual = gpu.render(frame, plan)?;
        assert!(
            context.fill_batches_encoded.load(Ordering::Relaxed) > before,
            "frame {index} did not activate batching"
        );
        let fallback = unbatched.render(frame, Plan::new(frame, limits)?)?;
        for (pixel, reference) in actual.pixels().iter().zip(fallback.pixels()) {
            for (value, reference) in pixel.iter().zip(reference) {
                assert!(
                    value.is_finite() && (value - reference).abs() < 3e-5,
                    "linear/HDR mismatch {pixel:?}"
                );
            }
        }
        let actual_rgba = actual.to_rgba8_frame(false, limits)?;
        let expected_rgba = expected.to_rgba8_frame(false, limits)?;
        let mut maximum = 0;
        let mut over_one = 0;
        assert_eq!(actual_rgba.pixels().len(), expected_rgba.pixels().len());
        for (pixel, reference) in actual_rgba.pixels().iter().zip(expected_rgba.pixels()) {
            let difference = pixel
                .iter()
                .zip(reference)
                .map(|(value, reference)| value.abs_diff(*reference))
                .max()
                .unwrap();
            maximum = maximum.max(difference);
            over_one += usize::from(difference > 1);
        }
        assert!(
            maximum <= 2 && over_one * 1000 < actual_rgba.pixels().len(),
            "frame {index}: max={maximum}, pixels_over_one={over_one}"
        );
        eprintln!(
            "fill_batch_frame={index} max_rgba_difference={maximum} pixels_over_one={over_one}/{}",
            actual_rgba.pixels().len()
        );
        outputs.push(actual);
    }
    for index in [3, 0, 2, 1, 0] {
        let replay = gpu.render(&frames[index], Plan::new(&frames[index], limits)?)?;
        assert_eq!(replay.pixels(), outputs[index].pixels());
    }
    for (index, frame) in frames.iter().enumerate() {
        for premultiplied in [false, true] {
            let format = crate::PixelFormat::SrgbRgba8 { premultiplied };
            let expected = crate::FrameBuffer::from_linear(&outputs[index], format, limits)?;
            let plan = Plan::new(frame, limits)?;
            let mut position = 0;
            let mut batches = 0;
            let mut uniform_buffers = 0;
            let mut unshared_vertex_buffers = 0;
            let mut unshared_uniform_buffers = 0;
            let mut vertex_bytes = 0;
            let mut uniform_bytes = 0;
            let mut staging_bytes = 0;
            let mut data_buffers = 0u64;
            let mut data_copies = 0u64;
            let mut data_bytes = 0u64;
            let mut data_payload_bytes = 0u64;
            let mut bind_groups = 0u64;
            while position < plan.steps.len() {
                let range = plan.texture_run(position);
                if plan.texture_blend(position) {
                    assert!(
                        Plan::batch_vertex_bytes(
                            &plan.steps[range.clone()],
                            context.device.limits().max_buffer_size,
                        )
                        .is_some()
                    );
                    assert!(
                        Plan::batch_uniform_bytes(
                            &plan.steps[range.clone()],
                            context.device.limits().max_buffer_size,
                            context.device.limits().min_uniform_buffer_offset_alignment,
                        )
                        .is_some()
                    );
                    batches += 1;
                    uniform_buffers += 2;
                    unshared_vertex_buffers += range.len() + 1;
                    unshared_uniform_buffers += range.len() + 1;
                    vertex_bytes += size_of::<[f32; 4]>() as u64
                        * (1 + plan.steps[range.clone()]
                            .iter()
                            .map(|step| step.quads.len().max(1) as u64)
                            .sum::<u64>());
                    uniform_bytes += (range.len() as u64 + 1) * size_of::<GpuUniforms>() as u64;
                    let payload = plan.steps[range.clone()]
                        .iter()
                        .map(|step| step.data.len() as u64 * size_of::<f32>() as u64)
                        .sum::<u64>();
                    let shared = plan.texture_data_bytes(
                        position,
                        context.device.limits().max_buffer_size,
                        context.device.limits().min_storage_buffer_offset_alignment,
                    );
                    let bytes = shared.unwrap_or(payload);
                    let count = if shared.is_some() {
                        1
                    } else {
                        range.len() as u64
                    };
                    let staging =
                        Plan::batch_upload_bytes(&plan.steps[range.clone()]) + bytes - payload;
                    data_buffers += count;
                    data_copies += if staging <= context.device.limits().max_buffer_size {
                        count
                    } else {
                        0
                    };
                    data_bytes += bytes;
                    data_payload_bytes += payload;
                    bind_groups += range.len() as u64;
                    staging_bytes += staging;
                } else {
                    uniform_buffers += 1;
                    unshared_uniform_buffers += 1;
                    uniform_bytes += size_of::<GpuUniforms>() as u64;
                }
                position = range.end;
            }
            let counters = || {
                [
                    context.batch_data_buffers_encoded.load(Ordering::Relaxed),
                    context.batch_data_copies_encoded.load(Ordering::Relaxed),
                    context.batch_data_bytes_encoded.load(Ordering::Relaxed),
                    context.batch_bind_groups_encoded.load(Ordering::Relaxed),
                ]
            };
            let before = counters();
            let pending = gpu.submit_output(frame, plan, format)?;
            assert_eq!(
                std::array::from_fn::<_, 4, _>(|index| counters()[index] - before[index]),
                [data_buffers, data_copies, data_bytes, bind_groups]
            );
            assert!(data_buffers < bind_groups);
            assert_eq!(
                pending
                    .working
                    .buffers
                    .iter()
                    .filter(|buffer| buffer.usage().contains(wgpu::BufferUsages::VERTEX))
                    .count(),
                batches * 2,
                "frame {index}: expected one backdrop and one shared instance buffer per batch",
            );
            assert_eq!(
                pending
                    .working
                    .buffers
                    .iter()
                    .filter(|buffer| buffer.usage().contains(wgpu::BufferUsages::UNIFORM))
                    .count(),
                uniform_buffers,
                "frame {index}: expected one backdrop and one shared uniform buffer per batch",
            );
            assert_eq!(
                pending
                    .working
                    .buffers
                    .iter()
                    .filter(|buffer| buffer.usage().contains(wgpu::BufferUsages::VERTEX))
                    .map(wgpu::Buffer::size)
                    .sum::<u64>(),
                vertex_bytes,
                "frame {index}: sharing must preserve the original vertex payload bytes",
            );
            assert_eq!(
                pending
                    .working
                    .buffers
                    .iter()
                    .filter(|buffer| buffer.usage().contains(wgpu::BufferUsages::UNIFORM))
                    .map(wgpu::Buffer::size)
                    .sum::<u64>(),
                uniform_bytes,
                "frame {index}: sharing must preserve uniform payload bytes without padding",
            );
            assert_eq!(
                pending
                    .working
                    .buffers
                    .iter()
                    .filter(|buffer| buffer.usage().contains(wgpu::BufferUsages::MAP_WRITE))
                    .map(wgpu::Buffer::size)
                    .sum::<u64>(),
                staging_bytes,
                "frame {index}: shared uploads must keep the original staging allocation",
            );
            assert!(unshared_vertex_buffers > batches * 2);
            assert!(unshared_uniform_buffers > uniform_buffers);
            if !premultiplied {
                eprintln!(
                    "batch_data_frame={index} buffers={bind_groups}->{data_buffers} copies={bind_groups}->{data_copies} payload_bytes={data_payload_bytes} allocated_bytes={data_bytes} padding_bytes={} bind_groups={bind_groups}",
                    data_bytes - data_payload_bytes
                );
                eprintln!(
                    "batch_resources_frame={index} vertex_buffers={unshared_vertex_buffers}->{} uniform_buffers={unshared_uniform_buffers}->{uniform_buffers} vertex_bytes={vertex_bytes} uniform_bytes={uniform_bytes} staging_bytes={staging_bytes} lease_capacity={}",
                    batches * 2,
                    pending.working.buffers.capacity(),
                );
            }
            let actual = pending.finish()?;
            let replay = gpu
                .submit_output(frame, Plan::new(frame, limits)?, format)?
                .finish()?;
            assert_eq!(actual, replay);
            if index == 0 && !premultiplied {
                let mut fallback = Plan::new(frame, limits)?;
                let start = fallback
                    .texture_data_sizes
                    .iter()
                    .position(|bytes| *bytes != 0)
                    .unwrap();
                let bytes = fallback.texture_data_sizes[start];
                assert_eq!(fallback.texture_data_bytes(start, bytes, 256), Some(bytes));
                assert!(fallback.texture_data_bytes(start, bytes - 1, 256).is_none());
                assert!(fallback.texture_data_bytes(start, u64::MAX, 0).is_none());
                assert!(fallback.texture_data_bytes(start, u64::MAX, 512).is_none());
                fallback.texture_data_sizes.clear();
                let before = counters();
                let separate = gpu.submit_output(frame, fallback, format)?.finish()?;
                assert_eq!(
                    std::array::from_fn::<_, 4, _>(|index| counters()[index] - before[index]),
                    [bind_groups, bind_groups, data_payload_bytes, bind_groups]
                );
                assert_eq!(separate, actual);
            }
            assert!(
                actual.planes()[0]
                    .bytes()
                    .iter()
                    .zip(expected.planes()[0].bytes())
                    .all(|(actual, expected)| actual.abs_diff(*expected) <= 1)
            );
        }
    }
    let format = crate::PixelFormat::Yuv420 { bit_depth: 8 };
    let expected = crate::FrameBuffer::from_linear(&outputs[3], format, limits)?;
    let actual = gpu
        .submit_output(&frames[3], Plan::new(&frames[3], limits)?, format)?
        .finish()?;
    for (actual, expected) in actual.planes().iter().zip(expected.planes()) {
        assert!(
            actual
                .bytes()
                .iter()
                .zip(expected.bytes())
                .all(|(actual, expected)| actual.abs_diff(*expected) <= 1)
        );
    }
    let maximum = frames
        .iter()
        .map(|frame| Plan::new(frame, limits).unwrap().stats.peak_bytes)
        .max()
        .unwrap();
    let mut queue = FrameQueue::new(context.clone(), maximum * 3)?;
    for index in [4, 0, 2] {
        queue.submit(&frames[index], limits)?;
    }
    for index in [4, 0, 2] {
        assert_eq!(
            queue
                .finish_next()?
                .unwrap()
                .linear_pixels()
                .collect::<Vec<_>>(),
            outputs[index].pixels()
        );
    }
    assert!(context.pooled_working_bytes() > 0);
    assert!(context.pooled_working_bytes() <= 64 * 1024 * 1024);
    Ok(())
}

#[test]
fn packing_shader_validates_without_a_device() {
    let shader = wgpu::naga::front::wgsl::parse_str(&crate::gpu::output_shader(include_str!(
        "../shaders/pack.wgsl"
    )))
    .unwrap();
    wgpu::naga::valid::Validator::new(
        wgpu::naga::valid::ValidationFlags::all(),
        wgpu::naga::valid::Capabilities::all(),
    )
    .validate(&shader)
    .unwrap();
}

#[test]
fn packed_output_yuv_depth_alignment_and_budget() -> motion_core::Result {
    let limits = RasterLimits::default();
    let size = PixelSize::new(16, 8);
    let frame = motion_core::Node::empty()
        .compile(
            motion_core::Size::new(16.0, 8.0),
            size,
            ValidationLimits::default(),
        )?
        .prepare(ValidationLimits::default(), limits)?;
    let plan = Plan::new(&frame, limits)?;
    let eight = plan.packed_stats(size, crate::PixelFormat::Yuv420 { bit_depth: 8 })?;
    let ten = plan.packed_stats(size, crate::PixelFormat::Yuv420 { bit_depth: 10 })?;
    assert_eq!(ten.peak_bytes - eight.peak_bytes, 16 * 8 * 3 / 2 * 3);
    assert_eq!(eight.work, ten.work);
    let mut exact = Plan::new(&frame, limits)?;
    exact.limits.max_bytes = ten.peak_bytes;
    assert_eq!(
        exact
            .packed_stats(size, crate::PixelFormat::Yuv420 { bit_depth: 10 })?
            .peak_bytes,
        ten.peak_bytes
    );
    exact.limits.max_bytes -= 1;
    assert!(
        exact
            .packed_stats(size, crate::PixelFormat::Yuv420 { bit_depth: 10 })
            .is_err()
    );
    for bit_depth in [8, 10] {
        let format = crate::PixelFormat::Yuv420 { bit_depth };
        for width in [8, 16, 1920] {
            assert!(Gpu::supports_packed_output(
                PixelSize::new(width, 2),
                format
            ));
        }
        for invalid_size in [
            PixelSize::new(2, 2),
            PixelSize::new(10, 2),
            PixelSize::new(16, 3),
        ] {
            assert!(!Gpu::supports_packed_output(invalid_size, format));
            assert!(plan.packed_stats(invalid_size, format).is_err());
        }
    }
    for bit_depth in [0, 9, 12, 16] {
        assert!(!Gpu::supports_packed_output(
            size,
            crate::PixelFormat::Yuv420 { bit_depth }
        ));
    }
    Ok(())
}

#[test]
fn dedicated_packed_plans_remove_only_unused_linear_readback_charges() -> motion_core::Result {
    let limits = RasterLimits::default();
    let size = PixelSize::new(960, 540);
    let frame = motion_core::Node::empty()
        .compile(
            motion_core::Size::new(960.0, 540.0),
            size,
            ValidationLimits::default(),
        )?
        .prepare(ValidationLimits::default(), limits)?;
    let conservative = Plan::new(&frame, limits)?;
    for format in [
        crate::PixelFormat::SrgbRgba8 {
            premultiplied: false,
        },
        crate::PixelFormat::SrgbRgba8 {
            premultiplied: true,
        },
        crate::PixelFormat::Yuv420 { bit_depth: 8 },
        crate::PixelFormat::Yuv420 { bit_depth: 10 },
    ] {
        let packed = Plan::new_packed(&frame, limits, format)?;
        let previous = conservative.packed_stats(size, format)?;
        assert_eq!(
            previous.peak_bytes - packed.stats.peak_bytes,
            u64::from(size.width) * u64::from(size.height) * 32 + size_of::<LinearFrame>() as u64
        );
        assert_eq!(
            previous.work - packed.stats.work,
            u64::from(size.width) * u64::from(size.height) * 2
        );
        assert_eq!(packed.packed_stats(size, format)?, packed.stats);
        let exact = RasterLimits {
            max_bytes: packed.stats.peak_bytes,
            max_work: packed.stats.work,
            ..limits
        };
        assert_eq!(Plan::new_packed(&frame, exact, format)?.stats, packed.stats);
        let required = RasterLimits {
            max_bytes: exact.max_bytes
                - packed.texture_data_sizes.len() as u64 * size_of::<u64>() as u64,
            max_work: exact.max_work - packed.steps.len() as u64 * 4,
            ..exact
        };
        let fallback = Plan::new_packed(&frame, required, format)?;
        assert!(fallback.texture_data_sizes.is_empty());
        assert_eq!(fallback.stats.peak_bytes, required.max_bytes);
        assert_eq!(fallback.stats.work, required.max_work);
        assert!(
            Plan::new_packed(
                &frame,
                RasterLimits {
                    max_bytes: required.max_bytes - 1,
                    ..required
                },
                format
            )
            .is_err()
        );
        assert!(
            Plan::new_packed(
                &frame,
                RasterLimits {
                    max_work: required.max_work - 1,
                    ..required
                },
                format
            )
            .is_err()
        );
        assert!(conservative.packed_stats(size, format)?.peak_bytes > exact.max_bytes);
        assert!(
            packed
                .packed_stats(PixelSize::new(952, 540), format)
                .is_err()
        );
        let different = match format {
            crate::PixelFormat::SrgbRgba8 { premultiplied } => crate::PixelFormat::SrgbRgba8 {
                premultiplied: !premultiplied,
            },
            _ => crate::PixelFormat::SrgbRgba8 {
                premultiplied: false,
            },
        };
        assert!(packed.packed_stats(size, different).is_err());
    }
    let eight = Plan::new_packed(&frame, limits, crate::PixelFormat::Yuv420 { bit_depth: 8 })?;
    let ten = Plan::new_packed(&frame, limits, crate::PixelFormat::Yuv420 { bit_depth: 10 })?;
    assert_eq!(
        ten.stats.peak_bytes - eight.stats.peak_bytes,
        u64::from(size.width) * u64::from(size.height) * 3 / 2 * 3
    );
    assert!(Plan::new_packed(&frame, limits, crate::PixelFormat::Yuv420 { bit_depth: 9 }).is_err());
    Ok(())
}

#[test]
#[ignore = "requires native GPU; checks all nonfinite channels outside packed texture crop"]
fn packed_output_checks_nonfinite_texture_outside_crop() -> motion_core::Result {
    use wgpu::util::DeviceExt;

    let gpu = Gpu::new()?;
    for (channel, value) in [
        (0, 0.25),
        (0, f32::NAN),
        (1, f32::INFINITY),
        (2, f32::NEG_INFINITY),
        (3, f32::INFINITY),
    ] {
        for mode in 0..4u32 {
            for scan_outside in [false, true] {
                let mut pixels = [[0.25f32, 0.5, 0.75, 2.0]; 32];
                pixels[12][channel] = value;
                let texture = gpu.device.create_texture_with_data(
                    &gpu.queue,
                    &wgpu::TextureDescriptor {
                        label: Some("packed finite-check fixture"),
                        size: wgpu::Extent3d {
                            width: 16,
                            height: 2,
                            depth_or_array_layers: 1,
                        },
                        mip_level_count: 1,
                        sample_count: 1,
                        dimension: wgpu::TextureDimension::D2,
                        format: wgpu::TextureFormat::Rgba32Float,
                        usage: wgpu::TextureUsages::TEXTURE_BINDING,
                        view_formats: &[],
                    },
                    wgpu::util::TextureDataOrder::LayerMajor,
                    bytemuck::cast_slice(&pixels),
                );
                let view = texture.create_view(&Default::default());
                let mut words = vec![
                    3u32,
                    5,
                    8,
                    2,
                    3,
                    5,
                    19,
                    7,
                    mode,
                    1,
                    1,
                    u32::from(scan_outside),
                ];
                words.extend([0; 32]);
                let settings = gpu
                    .device
                    .create_buffer_init(&wgpu::util::BufferInitDescriptor {
                        label: Some("packed finite-check settings"),
                        contents: bytemuck::cast_slice(&words),
                        usage: wgpu::BufferUsages::UNIFORM,
                    });
                let output = gpu.device.create_buffer(&wgpu::BufferDescriptor {
                    label: Some("packed finite-check output"),
                    size: 64,
                    usage: wgpu::BufferUsages::STORAGE,
                    mapped_at_creation: false,
                });
                let status = gpu
                    .device
                    .create_buffer_init(&wgpu::util::BufferInitDescriptor {
                        label: Some("packed finite-check status"),
                        contents: &[0; 4],
                        usage: wgpu::BufferUsages::STORAGE
                            | wgpu::BufferUsages::COPY_SRC
                            | wgpu::BufferUsages::COPY_DST,
                    });
                let readback = gpu.device.create_buffer(&wgpu::BufferDescriptor {
                    label: Some("packed finite-check readback"),
                    size: 4,
                    usage: wgpu::BufferUsages::MAP_READ | wgpu::BufferUsages::COPY_DST,
                    mapped_at_creation: false,
                });
                let pipeline = gpu.packing_pipeline();
                let lut = gpu
                    .device
                    .create_buffer_init(&wgpu::util::BufferInitDescriptor {
                        label: None,
                        contents: bytemuck::cast_slice(&[0.0f32; 20]),
                        usage: wgpu::BufferUsages::STORAGE,
                    });
                let bindings = gpu.device.create_bind_group(&wgpu::BindGroupDescriptor {
                    label: Some("packed finite-check bindings"),
                    layout: &pipeline.get_bind_group_layout(0),
                    entries: &[
                        wgpu::BindGroupEntry {
                            binding: 0,
                            resource: wgpu::BindingResource::TextureView(&view),
                        },
                        wgpu::BindGroupEntry {
                            binding: 1,
                            resource: settings.as_entire_binding(),
                        },
                        wgpu::BindGroupEntry {
                            binding: 2,
                            resource: output.as_entire_binding(),
                        },
                        wgpu::BindGroupEntry {
                            binding: 3,
                            resource: status.as_entire_binding(),
                        },
                        wgpu::BindGroupEntry {
                            binding: 4,
                            resource: gpu.alpha_thresholds().as_entire_binding(),
                        },
                        wgpu::BindGroupEntry {
                            binding: 5,
                            resource: lut.as_entire_binding(),
                        },
                    ],
                });
                let mut encoder = gpu.device.create_command_encoder(&Default::default());
                {
                    let mut pass = encoder.begin_compute_pass(&Default::default());
                    pass.set_pipeline(pipeline);
                    pass.set_bind_group(0, &bindings, &[]);
                    pass.dispatch_workgroups(1, 1, 1);
                }
                encoder.copy_buffer_to_buffer(&status, 0, &readback, 0, 4);
                gpu.queue.submit(Some(encoder.finish()));
                let (sender, receiver) = std::sync::mpsc::channel();
                readback
                    .slice(..)
                    .map_async(wgpu::MapMode::Read, move |result| {
                        sender.send(result).unwrap();
                    });
                wait_for_gpu(&gpu.device, None)?;
                receiver.recv().unwrap().unwrap();
                let mapped = readback.slice(..).get_mapped_range().unwrap();
                let flags = u32::from_le_bytes(mapped[..4].try_into().unwrap());
                assert_eq!(flags, u32::from(scan_outside && !value.is_finite()));
                drop(mapped);
                readback.unmap();
                if mode == 0 && scan_outside {
                    gpu.queue.write_buffer(&status, 0, &[0; 4]);
                    let bounds = PixelRect {
                        left: 3,
                        top: 5,
                        right: 19,
                        bottom: 7,
                    };
                    let mut uniforms = GpuUniforms::default();
                    uniforms.images([bounds; 3]);
                    let parameters =
                        gpu.device
                            .create_buffer_init(&wgpu::util::BufferInitDescriptor {
                                label: Some("accumulation finite-check settings"),
                                contents: bytemuck::bytes_of(&uniforms),
                                usage: wgpu::BufferUsages::UNIFORM,
                            });
                    let pipeline = &gpu.textures.check;
                    let bindings = gpu.device.create_bind_group(&wgpu::BindGroupDescriptor {
                        label: Some("accumulation finite-check bindings"),
                        layout: &pipeline.get_bind_group_layout(0),
                        entries: &[
                            wgpu::BindGroupEntry {
                                binding: 0,
                                resource: parameters.as_entire_binding(),
                            },
                            wgpu::BindGroupEntry {
                                binding: 5,
                                resource: status.as_entire_binding(),
                            },
                            wgpu::BindGroupEntry {
                                binding: 7,
                                resource: wgpu::BindingResource::TextureView(&view),
                            },
                        ],
                    });
                    let mut encoder = gpu.device.create_command_encoder(&Default::default());
                    {
                        let mut pass = encoder.begin_compute_pass(&Default::default());
                        pass.set_pipeline(pipeline);
                        pass.set_bind_group(0, &bindings, &[]);
                        pass.dispatch_workgroups(2, 1, 1);
                    }
                    encoder.copy_buffer_to_buffer(&status, 0, &readback, 0, 4);
                    gpu.queue.submit(Some(encoder.finish()));
                    let (sender, receiver) = std::sync::mpsc::channel();
                    readback
                        .slice(..)
                        .map_async(wgpu::MapMode::Read, move |result| {
                            sender.send(result).unwrap();
                        });
                    wait_for_gpu(&gpu.device, None)?;
                    receiver.recv().unwrap().unwrap();
                    let mapped = readback.slice(..).get_mapped_range().unwrap();
                    let flags = u32::from_le_bytes(mapped[..4].try_into().unwrap());
                    assert_eq!(flags, u32::from(!value.is_finite()));
                    drop(mapped);
                    readback.unmap();
                }
            }
        }
    }
    Ok(())
}

#[test]
#[ignore = "requires native GPU; compares dedicated and conservative packed plans and tight budgets"]
fn dedicated_packed_plans_preserve_bytes_crop_replay_and_tight_admission() -> motion_core::Result {
    let mut gpu = Gpu::new()?;
    let limits = RasterLimits::default();
    let mut frame = motion_core::Node::from(
        motion_core::Rect::new(3.25, 4.5, 87.75, 58.25).fill(motion_core::Color::rgb(190, 40, 125)),
    )
    .compile(
        motion_core::Size::new(96.0, 64.0),
        PixelSize::new(96, 64),
        ValidationLimits::default(),
    )?
    .with_background(Some(motion_core::Color::BLACK))
    .prepare(ValidationLimits::default(), limits)?;
    for cropped in [false, true] {
        if cropped {
            frame.output_bounds = PixelRect {
                left: 3,
                top: 4,
                right: 83,
                bottom: 52,
            };
        }
        for format in [
            crate::PixelFormat::SrgbRgba8 {
                premultiplied: false,
            },
            crate::PixelFormat::SrgbRgba8 {
                premultiplied: true,
            },
            crate::PixelFormat::Yuv420 { bit_depth: 8 },
            crate::PixelFormat::Yuv420 { bit_depth: 10 },
        ] {
            let legacy = Plan::new(&frame, limits)?;
            let expected = gpu.render_output(&frame, legacy, format)?;
            let dedicated = Plan::new_packed(&frame, limits, format)?;
            let exact = RasterLimits {
                max_bytes: dedicated.stats.peak_bytes,
                ..limits
            };
            let pending =
                gpu.submit_output(&frame, Plan::new_packed(&frame, exact, format)?, format)?;
            let transferred = pending.readback_bytes();
            let actual = pending.finish()?;
            assert_eq!(actual, expected);
            assert_eq!(
                transferred,
                actual
                    .planes()
                    .iter()
                    .map(|plane| plane.bytes().len() as u64)
                    .sum::<u64>()
            );
            assert_eq!(
                gpu.render_output(&frame, Plan::new_packed(&frame, exact, format)?, format)?,
                expected
            );
            let legacy = Plan::new(&frame, limits)?;
            assert!(legacy.packed_stats(frame.size(), format)?.peak_bytes > exact.max_bytes);
            assert!(
                gpu.submit_linear(&frame, Plan::new_packed(&frame, limits, format)?)
                    .is_err()
            );
        }
    }
    Ok(())
}

#[test]
#[ignore = "requires native GPU; checks ten-bit blocks, crop origins, precision and replay without timings"]
fn packed_yuv10_gpu_output_preserves_layout_and_replay() -> motion_core::Result {
    use motion_core::{Color, LinearGradient, Node, Point, Rect, Size};
    let mut gpu = Gpu::new()?;
    let limits = RasterLimits::default();
    let format = crate::PixelFormat::Yuv420 { bit_depth: 10 };
    for size in [PixelSize::new(8, 2), PixelSize::new(24, 10)] {
        let design = Size::new(f64::from(size.width), f64::from(size.height));
        let mut prepared = Node::group([
            Node::from(
                Rect::new(0.25, 0.5, design.width - 0.75, design.height - 1.0).fill(
                    LinearGradient::new(Point::ORIGIN, Point::new(1.0, 0.0))
                        .stop(0.0, Color::rgb(255, 0, 0))
                        .stop(0.5, Color::rgb(0, 255, 0))
                        .stop(1.0, Color::rgb(0, 0, 255)),
                ),
            ),
            Node::from(Rect::new(3.0, 1.0, 5.0, 2.0).fill(Color::from_rgb_float(2.0, -0.5, 3.0))),
        ])
        .compile(design, size, ValidationLimits::default())?
        .prepare(ValidationLimits::default(), limits)?;
        assert!(
            gpu.submit_output(&prepared, Plan::new(&prepared, limits)?, format)?
                .finish()
                .is_err()
        );
        prepared.background = Some(PaintRecord {
            linear_premultiplied: [0.0, 0.0, 0.0, 1.0],
        });
        for cropped in [false, true] {
            if cropped && size.width == 8 {
                continue;
            }
            let mut frame = prepared.clone();
            if cropped {
                frame.output_bounds = PixelRect {
                    left: 3,
                    top: 2,
                    right: 19,
                    bottom: 8,
                };
            }
            for display in [
                crate::DisplayTransform::default(),
                crate::DisplayTransform {
                    tone_map: crate::ToneMap::AgX,
                    reference_white: Some(1.0),
                    transfer: crate::OutputTransfer::Gamma24,
                    grain: Some(crate::FilmGrain {
                        seed: 0x123456789abcdef0,
                        amount: 0.03,
                        size: 1.5,
                        luma_response: 1.0,
                        chroma: 0.2,
                    }),
                    ..crate::DisplayTransform::default()
                },
            ] {
                let (expected, _, _) = crate::FrameBuffer::from_prepared_display(
                    &frame,
                    format,
                    limits,
                    &mut Default::default(),
                    &crate::output::OutputPool::default(),
                    crate::display::DisplaySettings {
                        display: (!display.is_identity()).then_some(&display),
                        frame: 7,
                        origin: [frame.output_bounds.left, frame.output_bounds.top],
                    },
                )?;
                let plan = Plan::new_packed(&frame, limits, format)?.with_display(&display, 7)?;
                let pending = gpu.submit_output(&frame, plan, format)?;
                let bytes = u64::from(frame.size().width) * u64::from(frame.size().height) * 3;
                assert_eq!(pending.readback_bytes(), bytes);
                let actual = pending.finish()?;
                assert_packed_output_matches(&actual, &expected);
                for (index, plane) in actual.planes().iter().enumerate() {
                    for sample in plane.bytes().as_chunks::<2>().0 {
                        let sample = u16::from_le_bytes(*sample);
                        assert!((64..=if index == 0 { 940 } else { 960 }).contains(&sample));
                    }
                }
                for _ in 0..2 {
                    assert_eq!(
                        actual,
                        gpu.render_output(
                            &frame,
                            Plan::new(&frame, limits)?.with_display(&display, 7)?,
                            format
                        )?
                    );
                    assert_eq!(gpu.readback.as_ref().unwrap().0, bytes);
                    assert_eq!(gpu.packed_output.as_ref().unwrap().0, bytes);
                }
            }
        }
    }
    Ok(())
}

#[test]
#[ignore = "requires native GPU; validates packed output against original linear conversion"]
fn packed_gpu_output_preserves_tolerance_alpha_and_replay() -> motion_core::Result<()> {
    let context = GpuContext::new()?;
    let mut gpu = Gpu::from_context(context);
    let limits = RasterLimits::default();
    for color in [
        motion_core::Color::BLACK,
        motion_core::Color::WHITE,
        motion_core::Color::rgb(190, 40, 125),
    ] {
        let frame =
            motion_core::Node::from(motion_core::Rect::new(0.25, 0.5, 14.75, 6.5).fill(color))
                .compile(
                    motion_core::Size::new(16.0, 8.0),
                    PixelSize::new(16, 8),
                    motion_core::ValidationLimits::default(),
                )?
                .prepare(motion_core::ValidationLimits::default(), limits)?;
        let full = gpu.render(&frame, Plan::new(&frame, limits)?)?;
        let mut cropped = frame.clone();
        cropped.output_bounds.left = 3;
        cropped.output_bounds.right = 13;
        cropped.output_bounds.top = 2;
        cropped.output_bounds.bottom = 6;
        let actual = gpu.render(&cropped, Plan::new(&cropped, limits)?)?;
        let expected = full
            .pixels()
            .as_chunks::<16>()
            .0
            .iter()
            .skip(2)
            .take(4)
            .flat_map(|row| row[3..13].iter().copied())
            .collect::<Vec<_>>();
        assert_eq!(actual.size(), PixelSize::new(10, 4));
        assert_eq!(actual.pixels(), expected);
        let pending = gpu
            .submit_linear(&cropped, Plan::new(&cropped, limits)?)?
            .finish()?;
        assert_eq!(pending.size(), actual.size());
        assert_eq!(pending.pixels(), expected);
        for format in [
            crate::PixelFormat::SrgbRgba8 {
                premultiplied: false,
            },
            crate::PixelFormat::SrgbRgba8 {
                premultiplied: true,
            },
        ] {
            let expected = crate::FrameBuffer::from_linear(
                &gpu.render(&frame, Plan::new(&frame, limits)?)?,
                format,
                limits,
            )?;
            let pending = gpu.submit_output(&frame, Plan::new(&frame, limits)?, format)?;
            assert_eq!(pending.readback_bytes(), 16 * 8 * 4);
            let actual = pending.finish()?;
            let replay = gpu.render_output(&frame, Plan::new(&frame, limits)?, format)?;
            assert_eq!(actual, replay);
            let replay = gpu.render_output(&frame, Plan::new(&frame, limits)?, format)?;
            assert_eq!(actual, replay);
            assert_eq!(gpu.readback.as_ref().unwrap().0, 16 * 8 * 4);
            assert_eq!(gpu.packed_output.as_ref().unwrap().0, 16 * 8 * 4);
            assert_eq!(
                expected.planes()[0].bytes(),
                actual.planes()[0].bytes(),
                "packed RGBA8 differs from linear readback conversion: {color:?}, {format:?}"
            );
            let linear = gpu.render(&cropped, Plan::new(&cropped, limits)?)?;
            let expected = crate::FrameBuffer::from_linear(&linear, format, limits)?;
            let pending = gpu.submit_output(&cropped, Plan::new(&cropped, limits)?, format)?;
            assert_eq!(pending.readback_bytes(), 10 * 4 * 4);
            assert!(pending.working.buffers.iter().all(|buffer| {
                buffer.usage() != wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_SRC
            }));
            let output = pending.finish()?;
            assert_eq!(output.planes()[0].bytes(), expected.planes()[0].bytes());
            assert_eq!(
                output,
                gpu.render_output(&cropped, Plan::new(&cropped, limits)?, format)?
            );
        }
        assert!(
            gpu.submit_output(
                &frame,
                Plan::new(&frame, limits)?,
                crate::PixelFormat::Yuv420 { bit_depth: 8 }
            )?
            .finish()
            .is_err()
        );
        let mut opaque = frame.clone();
        opaque.background = Some(PaintRecord {
            linear_premultiplied: [0.0, 0.0, 0.0, 1.0],
        });
        let format = crate::PixelFormat::Yuv420 { bit_depth: 8 };
        let expected = crate::FrameBuffer::from_linear(
            &gpu.render(&opaque, Plan::new(&opaque, limits)?)?,
            format,
            limits,
        )?;
        let pending = gpu.submit_output(&opaque, Plan::new(&opaque, limits)?, format)?;
        assert_eq!(pending.readback_bytes(), 16 * 8 * 3 / 2);
        let actual = pending.finish()?;
        let replay = gpu
            .submit_output(&opaque, Plan::new(&opaque, limits)?, format)?
            .finish()?;
        assert_eq!(actual, replay);
        for (a, b) in expected.planes().iter().zip(actual.planes()) {
            assert!(
                a.bytes()
                    .iter()
                    .zip(b.bytes())
                    .all(|(a, b)| a.abs_diff(*b) <= 1)
            );
        }
        opaque.output_bounds.left = 3;
        opaque.output_bounds.right = 11;
        opaque.output_bounds.top = 2;
        opaque.output_bounds.bottom = 6;
        let linear = gpu.render(&opaque, Plan::new(&opaque, limits)?)?;
        let expected = crate::FrameBuffer::from_linear(&linear, format, limits)?;
        let pending = gpu.submit_output(&opaque, Plan::new(&opaque, limits)?, format)?;
        assert_eq!(pending.readback_bytes(), 8 * 4 * 3 / 2);
        let actual = pending.finish()?;
        for (actual, expected) in actual.planes().iter().zip(expected.planes()) {
            assert!(
                actual
                    .bytes()
                    .iter()
                    .zip(expected.bytes())
                    .all(|(a, b)| a.abs_diff(*b) <= 1)
            );
        }
    }
    for byte in 0..255 {
        let middle = ((byte as f64 + 0.5) / 255.0) as f32;
        for alpha in [middle.next_down(), middle, middle.next_up()] {
            let mut frame = motion_core::Node::empty()
                .compile(
                    motion_core::Size::new(8.0, 2.0),
                    PixelSize::new(8, 2),
                    motion_core::ValidationLimits::default(),
                )?
                .prepare(motion_core::ValidationLimits::default(), limits)?;
            frame.background = Some(PaintRecord {
                linear_premultiplied: [alpha * 0.5, 0.0, 0.0, alpha],
            });
            let format = crate::PixelFormat::SrgbRgba8 {
                premultiplied: false,
            };
            let expected = crate::FrameBuffer::from_linear(
                &gpu.render(&frame, Plan::new(&frame, limits)?)?,
                format,
                limits,
            )?;
            let actual = gpu
                .submit_output(&frame, Plan::new(&frame, limits)?, format)?
                .finish()?;
            for (a, b) in expected.planes()[0]
                .bytes()
                .as_chunks::<4>()
                .0
                .iter()
                .zip(actual.planes()[0].bytes().as_chunks::<4>().0)
            {
                assert_eq!(a[3], b[3]);
                assert!(a.iter().zip(b).all(|(a, b)| a.abs_diff(*b) <= 1));
            }
        }
    }
    let size = PixelSize::new(1920, 1080);
    let frame = motion_core::Node::empty()
        .compile(
            motion_core::Size::new(1920.0, 1080.0),
            size,
            motion_core::ValidationLimits::default(),
        )?
        .prepare(motion_core::ValidationLimits::default(), limits)?;
    let pending = gpu.submit_output(
        &frame,
        Plan::new(&frame, limits)?,
        crate::PixelFormat::SrgbRgba8 {
            premultiplied: false,
        },
    )?;
    assert_eq!(pending.readback_bytes(), 8_294_400);
    let output = pending.finish()?;
    assert_eq!(output.size(), size);
    assert!(output.planes()[0].bytes().iter().all(|byte| *byte == 0));
    Ok(())
}

#[test]
#[ignore = "requires native GPU; verifies real three-frame submissions and ordered completion"]
fn three_in_flight_frames_preserve_order_and_reject_before_submission() -> motion_core::Result<()> {
    let context = GpuContext::new()?;
    let limits = RasterLimits::default();
    let frames = [
        motion_core::Color::rgb(190, 40, 125),
        motion_core::Color::rgb(20, 180, 90),
        motion_core::Color::WHITE,
    ]
    .map(|color| {
        motion_core::Node::from(motion_core::Rect::new(0.25, 0.5, 6.75, 3.5).fill(color))
            .compile(
                motion_core::Size::new(8.0, 4.0),
                PixelSize::new(8, 4),
                motion_core::ValidationLimits::default(),
            )
            .unwrap()
            .prepare(motion_core::ValidationLimits::default(), limits)
            .unwrap()
    });
    let charge = Plan::new(&frames[0], limits)?.stats.peak_bytes;
    let mut queue = FrameQueue::new(context.clone(), charge * 3)?;
    let mut expected = Vec::new();
    let mut serial = Gpu::from_context(context);
    for frame in &frames {
        expected.push(serial.render(frame, Plan::new(frame, limits)?)?);
        queue.submit(frame, limits)?;
    }
    assert_eq!(queue.len(), 3);
    assert_eq!(queue.in_flight_bytes(), charge * 3);
    assert!(queue.submit(&frames[0], limits).is_err());
    for expected in expected {
        assert_eq!(
            queue.finish_next()?.unwrap(),
            crate::FrameBuffer::from_linear(
                &expected,
                crate::PixelFormat::LinearRgba32Float,
                limits
            )?
        );
    }
    assert!(queue.is_empty());
    assert_eq!(queue.in_flight_bytes(), 0);
    assert!(queue.finish_next()?.is_none());
    assert!(queue.pooled_readback_bytes() > 0);
    for index in 0..6 {
        if queue.len() == 3 {
            let expected = &frames[(index - 3) % 3];
            assert_eq!(
                queue
                    .finish_next()?
                    .unwrap()
                    .linear_pixels()
                    .collect::<Vec<_>>(),
                serial
                    .render(expected, Plan::new(expected, limits)?)?
                    .pixels()
                    .to_vec()
            );
        }
        let frame = &frames[index % 3];
        queue.submit(frame, limits)?;
    }
    for frame in &frames {
        assert_eq!(
            queue
                .finish_next()?
                .unwrap()
                .linear_pixels()
                .collect::<Vec<_>>(),
            serial
                .render(frame, Plan::new(frame, limits)?)?
                .pixels()
                .to_vec()
        );
    }
    let mut tiny = FrameQueue::new(serial.context().clone(), charge - 1)?;
    assert!(tiny.submit(&frames[0], limits).is_err());
    assert!(tiny.is_empty());
    queue.submit(&frames[0], limits)?;
    drop(queue);
    assert_eq!(
        serial
            .render(&frames[0], Plan::new(&frames[0], limits)?)?
            .pixels(),
        frames[0].compose_linear(limits)?.pixels()
    );
    Ok(())
}

#[test]
#[ignore = "requires native GPU; validates persistent output and bounded readback reuse"]
fn persistent_present_and_readback_reuse_preserve_exact_replay() -> motion_core::Result<()> {
    let context = GpuContext::new()?;
    let mut gpu = Gpu::from_context(context.clone());
    let limits = RasterLimits::default();
    let frame = motion_core::Node::from(
        motion_core::Rect::new(0.25, 0.5, 6.75, 3.5).fill(motion_core::Color::WHITE),
    )
    .compile(
        motion_core::Size::new(8.0, 4.0),
        PixelSize::new(8, 4),
        motion_core::ValidationLimits::default(),
    )?
    .prepare(motion_core::ValidationLimits::default(), limits)?;
    assert_eq!(gpu.pooled_readback_bytes(), 0);
    let first = gpu.render(&frame, Plan::new(&frame, limits)?)?;
    assert_eq!(gpu.pooled_readback_bytes(), 8 * 4 * 16 + 4);
    let second = gpu.render(&frame, Plan::new(&frame, limits)?)?;
    assert_eq!(first, second);
    assert_eq!(gpu.pooled_readback_bytes(), 8 * 4 * 16 + 4);
    for format in [
        wgpu::TextureFormat::Rgba16Float,
        wgpu::TextureFormat::Rgba8Unorm,
        wgpu::TextureFormat::Bgra8Unorm,
    ] {
        let first = context.present_pipeline(format) as *const _;
        let cloned = context.clone();
        assert_eq!(first, cloned.present_pipeline(format) as *const _);
    }
    context.check_device()?;
    Ok(())
}

#[test]
#[ignore = "requires native GPU; checks aligned rectangle copies and unaligned fallback"]
fn strip_copy_rectangles_preserve_origins_gaps_and_unaligned_rows() -> motion_core::Result {
    use motion_core::{Color, Node, Rect, Size};
    let limits = RasterLimits::default();
    let mut gpu = Gpu::new()?;
    for width in [16, 17, 31, 32] {
        let size = PixelSize::new(width, 12);
        let frame = Node::group([
            Node::from(Rect::new(0.0, 0.0, f64::from(width), 12.0).fill(Color::rgb(31, 79, 113))),
            Node::from(Rect::new(2.0, 1.0, 5.0, 8.0).fill(Color::rgb(220, 40, 90))),
            Node::from(Rect::new(8.25, 3.5, 11.75, 10.25).fill(Color::rgb(20, 200, 120))),
            Node::from(
                Rect::new(f64::from(width) - 2.0, 4.0, f64::from(width), 12.0).fill(Color::WHITE),
            ),
        ])
        .compile(
            Size::new(f64::from(width), 12.0),
            size,
            ValidationLimits::default(),
        )?
        .prepare(ValidationLimits::default(), limits)?;
        let plan = Plan::new(&frame, limits)?;
        assert!(plan.steps.iter().any(|step| {
            step.quads
                .iter()
                .any(|quad| quad[0] > 0.0 && quad[1] > 0.0 && quad[3] - quad[1] > 1.0)
        }));
        let expected = frame.compose_linear(limits)?;
        let actual = gpu.render(&frame, plan)?;
        assert_eq!(
            actual.pixels(),
            gpu.render(&frame, Plan::new(&frame, limits)?)?.pixels()
        );
        let expected = expected.to_rgba8_frame(false, limits)?;
        let actual = actual.to_rgba8_frame(false, limits)?;
        for (reference, result) in expected.bytes().iter().zip(actual.bytes()) {
            assert!(reference.abs_diff(*result) <= 1, "width={width}");
        }
        assert_eq!(&actual.bytes()[..4], &expected.bytes()[..4]);
    }
    Ok(())
}

#[test]
#[ignore = "requires native GPU; hundreds of layers must be submitted incrementally and complete"]
fn hundreds_of_layers_complete_and_match_cpu() -> motion_core::Result {
    use std::sync::atomic::Ordering;

    let limits = RasterLimits::default();
    let layers = (0..400u32).map(|index| {
        let left = f64::from(index % 20) * 16.0 + 1.0;
        let top = f64::from(index / 20) * 8.0 + 1.0;
        let bounds = motion_core::Rect::new(left, top, left + 14.0, top + 6.0);
        motion_core::Node::group([
            motion_core::Node::from(
                motion_core::RoundedRect::new(bounds, 2.0)
                    .unwrap()
                    .fill(motion_core::Color::rgb(30, 150, 160)),
            ),
            motion_core::Node::from(
                motion_core::Circle::new(bounds.center(), 2.0)
                    .unwrap()
                    .fill(motion_core::Color::rgb(240, 100, 80)),
            ),
        ])
        .opacity(0.4 + f64::from(index % 5) * 0.1)
    });
    let frame = motion_core::Node::group(layers)
        .compile(
            motion_core::Size::new(320.0, 160.0),
            PixelSize::new(320, 160),
            ValidationLimits::default(),
        )?
        .prepare(ValidationLimits::default(), limits)?;
    let expected = frame
        .compose_linear(limits)?
        .to_rgba8_frame(false, limits)?;
    let mut gpu = Gpu::new()?;
    let plan = Plan::new(&frame, limits)?;
    assert!(plan.texture_data_sizes.iter().all(|bytes| *bytes == 0));
    let mut position = 0;
    let mut draws = 0;
    let mut pairs = 0;
    while position < plan.steps.len() {
        let range = plan.texture_run(position);
        assert!(range.len() < Plan::BATCH_DATA_MIN_DRAWS);
        pairs += usize::from(range.len() == 2);
        if plan.texture_blend(position) {
            draws += range.len() as u64;
        }
        position = range.end;
    }
    assert_eq!(pairs, 400);
    let counters = |gpu: &Gpu| {
        [
            gpu.context
                .batch_data_buffers_encoded
                .load(Ordering::Relaxed),
            gpu.context
                .batch_data_copies_encoded
                .load(Ordering::Relaxed),
            gpu.context
                .batch_bind_groups_encoded
                .load(Ordering::Relaxed),
        ]
    };
    let before = counters(&gpu);
    let actual = gpu.render(&frame, plan)?.to_rgba8_frame(false, limits)?;
    assert_eq!(
        std::array::from_fn::<_, 3, _>(|index| counters(&gpu)[index] - before[index]),
        [draws; 3]
    );
    let maximum = actual
        .pixels()
        .iter()
        .zip(expected.pixels())
        .flat_map(|(actual, expected)| actual.iter().zip(expected).map(|(a, e)| a.abs_diff(*e)))
        .max()
        .unwrap();
    assert!(maximum <= 2, "maximum difference {maximum}");
    let replay = gpu
        .render(&frame, Plan::new(&frame, limits)?)?
        .to_rgba8_frame(false, limits)?;
    assert_eq!(actual, replay);
    eprintln!(
        "short_batch_pairs={pairs} data_buffers={draws} data_copies={draws} bind_groups={draws} max_rgba_difference={maximum}"
    );
    Ok(())
}

#[test]
#[ignore = "requires native GPU; depth-of-field capture CPU/GPU parity, no performance measurements"]
fn depth_of_field_capture_cpu_gpu_parity() -> motion_core::Result {
    use motion_core::{
        Camera, Color, DepthOfField, DepthPlane, Rect, SceneCache, SceneContext, Size, Time,
    };
    let mut gpu = Gpu::new()?;
    let limits = RasterLimits::default();
    let lens = DepthOfField {
        focus_distance: 1000.0,
        f_stop: 2.0,
        focal_length: 50.0,
        max_radius: 6.0,
        perspective: true,
    };
    let cache = SceneCache::new(0);
    let context = SceneContext::new(
        Time::ZERO,
        motion_core::fps!(30),
        Size::new(32.0, 32.0),
        &cache,
    );
    let plane = |left: f64, depth: f64, color: Color| DepthPlane {
        node: Rect::new(left, -4.0, left + 8.0, 4.0).fill(color).into(),
        depth,
    };
    let node = lens.capture(
        &context,
        Camera::default(),
        [
            plane(-12.0, 3000.0, Color::rgb(40, 90, 200)),
            plane(-4.0, 1000.0, Color::WHITE),
            plane(4.0, 300.0, Color::rgb(255, 160, 40)),
        ],
    )?;
    let prepared = node
        .compile(
            Size::new(32.0, 32.0),
            PixelSize::new(32, 32),
            ValidationLimits::default(),
        )?
        .prepare(ValidationLimits::default(), limits)?;
    let cpu = prepared.compose_linear(limits)?;
    let actual = gpu.render(&prepared, Plan::new(&prepared, limits)?)?;
    for (cpu, actual) in cpu.pixels().iter().zip(actual.pixels()) {
        for channel in 0..4 {
            assert!(
                (cpu[channel] - actual[channel]).abs() <= 1e-4 * cpu[channel].abs().max(1.0),
                "{cpu:?} != {actual:?}"
            );
        }
    }
    Ok(())
}
