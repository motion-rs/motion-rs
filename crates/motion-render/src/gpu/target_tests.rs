//! GPU uniform layout, shader constant and texture-target tests.

use super::*;
use motion_core::NodeModifiers;

#[test]
fn typed_parameters_preserve_layout_and_reject_oversized_values() {
    let mut parameters = GpuUniforms::default();
    assert!(parameters.values(&[1.0; 33]).is_err());
    assert_eq!(bytemuck::bytes_of(&parameters), &[0; 256]);
    parameters.values(&[1.0; 32]).unwrap();
    assert_eq!(parameters.values[31].to_bits(), 1.0_f32.to_bits());
    parameters.mask(motion_core::MaskMode::InvertedLuminance);
    assert_eq!(parameters.operation.detail, 3);
    parameters.noise(0x12345678_90abcdef);
    assert_eq!(parameters.operation.detail, 0x90abcdef);
    assert_eq!(parameters.operation.extent, 0x12345678);
    let mut expected = [0u32; 64];
    expected[14] = 0x90abcdef;
    expected[15] = 0x12345678;
    expected[16..48].fill(1.0_f32.to_bits());
    assert_eq!(
        bytemuck::bytes_of(&parameters),
        bytemuck::cast_slice(&expected)
    );
    let shader = shader_constants()
        + include_str!("shaders/noise.wgsl")
        + include_str!("shaders/compose.wgsl")
        + include_str!("shaders/lut.wgsl");
    let module = wgpu::naga::front::wgsl::parse_str(&shader).unwrap();
    let evaluate = module
        .functions
        .iter()
        .find(|(_, function)| function.name.as_deref() == Some("evaluate"))
        .unwrap()
        .1;
    let cases = evaluate
        .body
        .iter()
        .find_map(|statement| {
            if let wgpu::naga::Statement::Switch { cases, .. } = statement {
                Some(cases)
            } else {
                None
            }
        })
        .unwrap();
    for operation in [
        GpuOp::Clear,
        GpuOp::Fill,
        GpuOp::Composite,
        GpuOp::Mask,
        GpuOp::Blur,
        GpuOp::Offset,
        GpuOp::ColorMatrix,
        GpuOp::Threshold,
        GpuOp::Noise,
        GpuOp::ChannelOffset,
        GpuOp::ColorAdjust,
        GpuOp::Colorize,
        GpuOp::Scale,
        GpuOp::Particles,
    ] {
        assert!(
            cases
                .iter()
                .any(|case| case.value == wgpu::naga::SwitchValue::U32(operation as u32))
        );
    }
}

#[test]
#[ignore = "requires real native GPU; no window, fonts or encoder"]
fn external_texture_formats_guard_bands_and_rejections_match_cpu() -> motion_core::Result<()> {
    let context = GpuContext::new()?;
    let limits = RasterLimits::default();
    let frame = motion_core::Node::from(
        motion_core::Rect::new(0.25, 0.5, 12.75, 7.25).fill(motion_core::Color::rgb(20, 190, 230)),
    )
    .opacity(0.5)
    .filter(motion_core::Filter::Blur { sigma: 0.5 })
    .compile(
        motion_core::Size::new(16.0, 8.0),
        PixelSize::new(16, 8),
        motion_core::ValidationLimits::default(),
    )?
    .prepare(motion_core::ValidationLimits::default(), limits)?;
    for (texture_format, format, bpp) in [
        (
            wgpu::TextureFormat::Rgba8Unorm,
            crate::PixelFormat::SrgbRgba8 {
                premultiplied: false,
            },
            4u32,
        ),
        (
            wgpu::TextureFormat::Rgba8Unorm,
            crate::PixelFormat::SrgbRgba8 {
                premultiplied: true,
            },
            4,
        ),
        (
            wgpu::TextureFormat::Bgra8Unorm,
            crate::PixelFormat::SrgbRgba8 {
                premultiplied: false,
            },
            4u32,
        ),
        (
            wgpu::TextureFormat::Bgra8Unorm,
            crate::PixelFormat::SrgbRgba8 {
                premultiplied: true,
            },
            4,
        ),
        (
            wgpu::TextureFormat::Rgba16Float,
            crate::PixelFormat::LinearRgba16Float,
            8,
        ),
    ] {
        for cropped in [false, true] {
            let mut frame = frame.clone();
            if cropped {
                frame.output_bounds = PixelRect {
                    left: 3,
                    top: 2,
                    right: 13,
                    bottom: 7,
                };
            }
            let linear = frame.render_linear(limits)?;
            let size = frame.size();
            let texture = context.device().create_texture(&wgpu::TextureDescriptor {
                label: Some("External format/guard-band acceptance"),
                size: wgpu::Extent3d {
                    width: size.width,
                    height: size.height,
                    depth_or_array_layers: 1,
                },
                mip_level_count: 1,
                sample_count: 1,
                dimension: wgpu::TextureDimension::D2,
                format: texture_format,
                usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::COPY_SRC,
                view_formats: &[],
            });
            context.render(
                &frame,
                crate::RenderTarget::Texture(&texture),
                format,
                limits,
            )?;
            let pitch = (size.width * bpp).div_ceil(256) * 256;
            let buffer = context.device().create_buffer(&wgpu::BufferDescriptor {
                label: Some("External format acceptance readback"),
                size: u64::from(pitch) * u64::from(size.height),
                usage: wgpu::BufferUsages::MAP_READ | wgpu::BufferUsages::COPY_DST,
                mapped_at_creation: false,
            });
            let mut encoder = context.device().create_command_encoder(&Default::default());
            encoder.copy_texture_to_buffer(
                wgpu::TexelCopyTextureInfo {
                    texture: &texture,
                    mip_level: 0,
                    origin: wgpu::Origin3d::ZERO,
                    aspect: wgpu::TextureAspect::All,
                },
                wgpu::TexelCopyBufferInfo {
                    buffer: &buffer,
                    layout: wgpu::TexelCopyBufferLayout {
                        offset: 0,
                        bytes_per_row: Some(pitch),
                        rows_per_image: None,
                    },
                },
                texture.size(),
            );
            context.queue().submit(Some(encoder.finish()));
            let (sender, receiver) = std::sync::mpsc::channel();
            buffer
                .slice(..)
                .map_async(wgpu::MapMode::Read, move |result| {
                    sender.send(result).unwrap();
                });
            wait_for_gpu(context.device(), None)?;
            receiver
                .recv()
                .unwrap()
                .map_err(|error| motion_core::Error::Gpu(Box::new(error)))?;
            let mapped = buffer.slice(..).get_mapped_range().unwrap();
            let mut actual = mapped
                .chunks_exact(pitch as usize)
                .flat_map(|row| row[..size.width as usize * bpp as usize].iter().copied())
                .collect::<Vec<_>>();
            drop(mapped);
            buffer.unmap();
            let mut expected = crate::FrameBuffer::new(frame.size(), limits)?;
            crate::FrameBuffer::write_linear(
                &linear,
                crate::RenderTarget::Memory(&mut expected),
                format,
                limits,
            )?;
            let expected = expected.planes()[0].bytes();
            if bpp == 4 {
                if texture_format == wgpu::TextureFormat::Bgra8Unorm {
                    for pixel in actual.as_chunks_mut::<4>().0 {
                        pixel.swap(0, 2);
                    }
                }
                assert!(
                    actual
                        .iter()
                        .zip(expected)
                        .all(|(actual, expected)| actual.abs_diff(*expected) <= 1)
                );
            } else {
                for (actual, expected) in actual
                    .as_chunks::<2>()
                    .0
                    .iter()
                    .zip(expected.as_chunks::<2>().0)
                {
                    let actual = half::f16::from_le_bytes(*actual).to_f32();
                    let expected = half::f16::from_le_bytes(*expected).to_f32();
                    assert!((actual - expected).abs() <= 0.001);
                }
            }
            assert!(
                context
                    .render(
                        &frame,
                        crate::RenderTarget::Texture(&texture),
                        crate::PixelFormat::Yuv420 { bit_depth: 8 },
                        limits
                    )
                    .is_err()
            );
            context.render(
                &frame,
                crate::RenderTarget::Texture(&texture),
                format,
                limits,
            )?;
        }
    }
    Ok(())
}

#[test]
#[ignore = "requires native GPU; checks completed staging remapping, identity and shared cache limits"]
fn completed_staging_is_reused_without_aliasing_inflight_uploads() -> motion_core::Result<()> {
    let context = GpuContext::new()?;
    let first = context.staging_buffer(256);
    let second = context.staging_buffer(256);
    assert_ne!(first, second);
    assert_eq!(context.pooled_working_bytes(), 0);
    first
        .slice(..)
        .get_mapped_range_mut()
        .unwrap()
        .slice(..)
        .fill(17);
    second
        .slice(..)
        .get_mapped_range_mut()
        .unwrap()
        .slice(..)
        .fill(34);
    first.unmap();
    second.unmap();
    let readback = context.device.create_buffer(&wgpu::BufferDescriptor {
        label: Some("staging reuse verification"),
        size: 512,
        usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
        mapped_at_creation: false,
    });
    let mut encoder = context.device.create_command_encoder(&Default::default());
    encoder.copy_buffer_to_buffer(&first, 0, &readback, 0, 256);
    encoder.copy_buffer_to_buffer(&second, 0, &readback, 256, 256);
    let submission = context.queue.submit(Some(encoder.finish()));
    wait_for_gpu(&context.device, Some(submission))?;
    context.return_working(
        WorkingLease {
            buffers: vec![first.clone(), second.clone()],
            ..Default::default()
        },
        512,
    );
    let (sender, receiver) = std::sync::mpsc::channel();
    readback
        .slice(..)
        .map_async(wgpu::MapMode::Read, move |result| {
            let _ = sender.send(result);
        });
    wait_for_gpu(&context.device, None)?;
    receiver.recv().unwrap().unwrap();
    assert_eq!(context.pooled_working_bytes(), 512);
    let reused_first = context.staging_buffer(256);
    let reused_second = context.staging_buffer(256);
    assert!(reused_first == first || reused_first == second);
    assert!(reused_second == first || reused_second == second);
    assert_ne!(reused_first, reused_second);
    assert_eq!(context.pooled_working_bytes(), 0);
    reused_first
        .slice(..)
        .get_mapped_range_mut()
        .unwrap()
        .slice(..)
        .fill(51);
    reused_second
        .slice(..)
        .get_mapped_range_mut()
        .unwrap()
        .slice(..)
        .fill(68);
    let mapped = readback.slice(..).get_mapped_range().unwrap();
    assert!(mapped[..256].iter().all(|byte| *byte == 17));
    assert!(mapped[256..].iter().all(|byte| *byte == 34));
    drop(mapped);
    readback.unmap();
    reused_first.unmap();
    reused_second.unmap();
    context.return_working(
        WorkingLease {
            buffers: vec![reused_first, reused_second],
            ..Default::default()
        },
        512,
    );
    context.working_pool.lock().unwrap().trim(0);
    wait_for_gpu(&context.device, None)?;
    assert_eq!(context.pooled_working_bytes(), 0);
    let pending = context.staging_buffer(256);
    pending.unmap();
    let completion = std::sync::Arc::new(std::sync::OnceLock::new());
    {
        let mut pool = context.working_pool.lock().unwrap();
        pool.staging.push(StagingBuffer {
            buffer: pending.clone(),
            mapped: completion.clone(),
        });
        pool.bytes += 256;
    }
    let fallback = context.staging_buffer(256);
    assert_ne!(fallback, pending, "Unready staging must not be reused");
    assert_eq!(context.pooled_working_bytes(), 256);
    completion.set(false).unwrap();
    let smaller = context.staging_buffer(128);
    assert_eq!(smaller.size(), 128);
    assert_eq!(context.pooled_working_bytes(), 0);
    fallback.unmap();
    smaller.unmap();
    context.return_working(
        WorkingLease {
            buffers: vec![fallback, smaller],
            ..Default::default()
        },
        128,
    );
    assert_eq!(context.pooled_working_bytes(), 128);
    context.working_pool.lock().unwrap().trim(0);
    wait_for_gpu(&context.device, None)?;
    assert_eq!(context.pooled_working_bytes(), 0);
    Ok(())
}

#[test]
#[ignore = "requires native GPU; validates independent asynchronous texture leases and pool recovery"]
fn async_textures_keep_inflight_uploads_distinct_and_recover_after_drop() -> motion_core::Result<()>
{
    let context = GpuContext::new()?;
    let limits = RasterLimits::default();
    let frame = |color| {
        motion_core::Node::from(motion_core::Rect::new(0.0, 0.0, 4.0, 4.0).fill(color))
            .compile(
                motion_core::Size::new(4.0, 4.0),
                PixelSize::new(4, 4),
                motion_core::ValidationLimits::default(),
            )?
            .prepare(motion_core::ValidationLimits::default(), limits)
            .map_err(motion_core::Error::from)
    };
    let red = frame(motion_core::Color::rgb(255, 0, 0))?;
    let green = frame(motion_core::Color::rgb(0, 255, 0))?;
    let texture = || {
        context.device.create_texture(&wgpu::TextureDescriptor {
            label: Some("async texture test"),
            size: wgpu::Extent3d {
                width: 4,
                height: 4,
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: wgpu::TextureFormat::Rgba8Unorm,
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::COPY_SRC,
            view_formats: &[],
        })
    };
    let first = texture();
    let second = texture();
    let format = crate::PixelFormat::SrgbRgba8 {
        premultiplied: true,
    };
    let pending_first = context.submit_texture(&red, &first, format, limits)?;
    let pending_second = context.submit_texture(&green, &second, format, limits)?;
    assert_eq!(context.pooled_working_bytes(), 0);
    pending_second.finish()?;
    assert!(context.pooled_working_bytes() > 0);
    pending_first.finish()?;
    let completed: Vec<_> = context
        .working_pool
        .lock()
        .unwrap()
        .staging
        .iter()
        .map(|entry| entry.buffer.clone())
        .collect();
    let abandoned = context.submit_texture(&red, &first, format, limits)?;
    if context.textures.blends.is_some() {
        assert!(
            abandoned
                .working
                .iter()
                .any(|buffer| completed.contains(buffer)),
            "A later frame must reuse completed batch staging"
        );
    }
    drop(abandoned);
    context
        .submit_texture(&green, &second, format, limits)?
        .finish()?;
    for (texture, expected) in [(&first, [255, 0, 0, 255]), (&second, [0, 255, 0, 255])] {
        let buffer = context.device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("async texture verification"),
            size: 1024,
            usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
            mapped_at_creation: false,
        });
        let mut encoder = context.device.create_command_encoder(&Default::default());
        encoder.copy_texture_to_buffer(
            wgpu::TexelCopyTextureInfo {
                texture,
                mip_level: 0,
                origin: wgpu::Origin3d::ZERO,
                aspect: wgpu::TextureAspect::All,
            },
            wgpu::TexelCopyBufferInfo {
                buffer: &buffer,
                layout: wgpu::TexelCopyBufferLayout {
                    offset: 0,
                    bytes_per_row: Some(256),
                    rows_per_image: None,
                },
            },
            texture.size(),
        );
        let submission = context.queue.submit(Some(encoder.finish()));
        let (sender, receiver) = std::sync::mpsc::channel();
        buffer
            .slice(..)
            .map_async(wgpu::MapMode::Read, move |result| {
                let _ = sender.send(result);
            });
        wait_for_gpu(&context.device, Some(submission))?;
        receiver
            .recv()
            .map_err(|error| motion_core::Error::Gpu(Box::new(error)))?
            .map_err(|error| motion_core::Error::Gpu(Box::new(error)))?;
        let mapped = buffer.slice(..).get_mapped_range().unwrap();
        for row in 0..4 {
            for column in 0..4 {
                assert_eq!(
                    &mapped[row * 256 + column * 4..row * 256 + column * 4 + 4],
                    &expected
                );
            }
        }
        drop(mapped);
        buffer.unmap();
    }
    assert!(context.pooled_working_bytes() <= (limits.max_bytes / 4).min(64 * 1024 * 1024));
    Ok(())
}

#[test]
#[ignore = "requires real native GPU; no window, fonts or encoder"]
fn shared_context_texture_target_self_check_and_device_loss_are_real() -> motion_core::Result<()> {
    let context = GpuContext::new()?;
    context.validate_policy(false)?;
    assert!(context.packing.get().is_some());
    assert!(context.alpha_thresholds.get().is_some());
    assert!(context.present_check.get().is_some());
    assert!(
        context
            .present
            .iter()
            .all(|pipeline| pipeline.get().is_some())
    );
    assert!(context.self_check_result.get().is_none());
    context.self_check()?;
    assert!(context.same_device(&context.clone()));
    let node = motion_core::Node::from(
        motion_core::Rect::new(0.25, 0.5, 12.75, 7.25).fill(motion_core::Color::rgb(20, 190, 230)),
    )
    .opacity(0.5);
    let limits = RasterLimits::default();
    let frame = node
        .compile(
            motion_core::Size::new(16.0, 8.0),
            PixelSize::new(16, 8),
            motion_core::ValidationLimits::default(),
        )?
        .prepare(motion_core::ValidationLimits::default(), limits)?;
    let texture = context.device().create_texture(&wgpu::TextureDescriptor {
        label: Some("External output acceptance"),
        size: wgpu::Extent3d {
            width: 16,
            height: 8,
            depth_or_array_layers: 1,
        },
        mip_level_count: 1,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format: wgpu::TextureFormat::Rgba8Unorm,
        usage: wgpu::TextureUsages::RENDER_ATTACHMENT
            | wgpu::TextureUsages::COPY_SRC
            | wgpu::TextureUsages::TEXTURE_BINDING,
        view_formats: &[],
    });
    context.render(
        &frame,
        crate::RenderTarget::Texture(&texture),
        crate::PixelFormat::SrgbRgba8 {
            premultiplied: true,
        },
        limits,
    )?;
    let readback = context.device().create_buffer(&wgpu::BufferDescriptor {
        label: Some("External acceptance only readback"),
        size: 256 * 8,
        usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
        mapped_at_creation: false,
    });
    let mut encoder = context.device().create_command_encoder(&Default::default());
    encoder.copy_texture_to_buffer(
        wgpu::TexelCopyTextureInfo {
            texture: &texture,
            mip_level: 0,
            origin: wgpu::Origin3d::ZERO,
            aspect: wgpu::TextureAspect::All,
        },
        wgpu::TexelCopyBufferInfo {
            buffer: &readback,
            layout: wgpu::TexelCopyBufferLayout {
                offset: 0,
                bytes_per_row: Some(256),
                rows_per_image: None,
            },
        },
        texture.size(),
    );
    context.queue().submit(Some(encoder.finish()));
    let (sender, receiver) = std::sync::mpsc::channel();
    readback
        .slice(..)
        .map_async(wgpu::MapMode::Read, move |result| {
            sender.send(result).unwrap();
        });
    wait_for_gpu(context.device(), None)?;
    receiver
        .recv()
        .unwrap()
        .map_err(|error| motion_core::Error::Gpu(Box::new(error)))?;
    let mapped = readback.slice(..).get_mapped_range().unwrap();
    let actual = mapped
        .as_chunks::<256>()
        .0
        .iter()
        .flat_map(|row| row[..64].iter().copied())
        .collect::<Vec<_>>();
    drop(mapped);
    readback.unmap();
    let expected = frame.render_linear(limits)?.to_srgb_rgba8(true, limits)?;
    assert!(
        actual
            .iter()
            .zip(expected.bytes())
            .all(|(actual, expected)| actual.abs_diff(*expected) <= 1)
    );
    assert!(
        actual
            .as_chunks::<4>()
            .0
            .iter()
            .any(|pixel| pixel[3] > 0 && pixel[3] < 255)
    );
    let composition = crate::Composition::new(16, 8)
        .gpu_context(context.clone())
        .backend_preference(crate::BackendPreference::Automatic, false);
    let mut first = composition.frame_renderer(motion_core::Size::new(16.0, 8.0))?;
    let mut second = composition.frame_renderer(motion_core::Size::new(16.0, 8.0))?;
    assert_eq!(
        first.render_frame(0, |_| node.clone())?.pixels(),
        second.render_frame(0, |_| node.clone())?.pixels()
    );
    assert_eq!(
        first.backend_decision().unwrap().renderer,
        crate::Renderer::Gpu
    );
    context.device().destroy();
    let _ = wait_for_gpu(context.device(), None);
    let error = first
        .render_frame(1, |_| node.clone())
        .err()
        .expect("lost device fails");
    assert_eq!(error.kind(), motion_core::ErrorKind::DeviceLost);
    Ok(())
}
