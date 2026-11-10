//! Native GPU planning, execution and CPU-parity tests.

use super::*;
use motion_core::ValidationErrorKind;
use motion_core::{Color, Node, NodeModifiers, Rect, Shape, Size, ValidationLimits};

#[test]
fn batch_data_topology_distinguishes_long_runs_from_isolated_pairs() -> motion_core::Result {
    let limits = RasterLimits {
        max_bytes: 1 << 30,
        max_work: 20_000_000_000,
        ..RasterLimits::default()
    };
    let palette = [
        Color::from_hex(0x178b79),
        Color::from_hex(0x327fc1),
        Color::from_hex(0xf47762),
    ];
    let wide = Node::group((0..200u32).map(|index| {
        let left = f64::from(index % 20) * 32.0;
        let top = f64::from(index / 20) * 36.0;
        let bounds = Rect::new(left + 1.0, top + 1.0, left + 31.0, top + 35.0);
        Node::group([
            Node::from(motion_core::RoundedRect::new(bounds, 3.0).fill(palette[0])),
            Node::from(motion_core::Circle::new(bounds.center(), 5.0).fill(palette[2])),
        ])
        .opacity(0.4 + f64::from(index % 5) * 0.1)
    }));
    let nodes = |count: u32| {
        let columns = (f64::from(count) * 16.0 / 9.0).sqrt().ceil() as u32;
        let rows = count.div_ceil(columns);
        let width = 640.0 / f64::from(columns);
        let height = 360.0 / f64::from(rows);
        Node::group((0..count).map(|index| {
            let left = f64::from(index % columns) * width;
            let top = f64::from(index / columns) * height;
            Rect::new(left, top, left + width * 0.7, top + height * 0.7)
                .fill(palette[index as usize % palette.len()])
        }))
    };
    for (node, expected_draws, expected_batches) in [
        (nodes(1_000), 1_000, 1),
        (nodes(10_000), 10_000, 1),
        (wide, 2, 200),
    ] {
        let frame = node
            .compile(
                Size::new(640.0, 360.0),
                PixelSize::new(640, 360),
                ValidationLimits::default(),
            )?
            .prepare(ValidationLimits::default(), limits)?;
        let plan = Plan::new(&frame, limits)?;
        let mut histogram = std::collections::BTreeMap::new();
        let mut padding = 0;
        let mut position = 0;
        while position < plan.steps.len() {
            let range = plan.texture_run(position);
            if range.len() > 1 {
                assert!(plan.texture_blend(position));
                *histogram.entry(range.len()).or_insert(0usize) += 1;
                let payload = plan.steps[range.clone()]
                    .iter()
                    .map(|step| step.data.len() as u64 * size_of::<f32>() as u64)
                    .sum::<u64>();
                let bytes = Plan::batch_data_bytes(
                    &plan.steps[range.clone()],
                    u64::MAX,
                    Plan::BATCH_DATA_ALIGNMENT,
                )
                .unwrap();
                padding += bytes - payload;
            }
            position = range.end;
        }
        assert_eq!(
            histogram,
            std::collections::BTreeMap::from([(expected_draws, expected_batches)])
        );
        let admitted = plan
            .texture_data_sizes
            .iter()
            .filter(|bytes| **bytes != 0)
            .count();
        assert_eq!(
            admitted,
            if expected_draws >= Plan::BATCH_DATA_MIN_DRAWS {
                expected_batches
            } else {
                0
            }
        );
        eprintln!(
            "batch_data_topology={histogram:?} steps={} padding_bytes={padding}",
            plan.steps.len()
        );
    }
    Ok(())
}

#[test]
fn image_lifetimes_keep_sources_and_final_output_until_their_last_read() {
    let frame = prepared(tile());
    let mut budget = Budget::new(frame.size, RasterLimits::default()).unwrap();
    let step = |first, second| Step {
        first,
        second,
        parameters: GpuUniforms::new(&frame, GpuOp::Clear, frame.output_bounds),
        data: vec![1.0],
        quads: Vec::new(),
    };
    let chain = vec![step(0, 0), step(1, 0), step(2, 0), step(3, 0)];
    let last = Plan::last_uses(&chain, 4, &mut budget).unwrap();
    let (slots, textures) = Plan::texture_lifetimes(&chain, &last, None, &mut budget).unwrap();
    assert_eq!(last, vec![4, 1, 2, 3, 4]);
    assert_eq!(textures.len(), 3);
    assert_eq!(slots, vec![0, 1, 2, 1, 2]);
    let branches = vec![step(0, 0), step(1, 0), step(1, 0), step(2, 3), step(4, 1)];
    let last = Plan::last_uses(&branches, 5, &mut budget).unwrap();
    let (_, textures) = Plan::texture_lifetimes(&branches, &last, None, &mut budget).unwrap();
    assert_eq!(last, vec![5, 4, 3, 3, 4, 5]);
    assert_eq!(textures.len(), 5);
    let repeated = vec![step(0, 0), step(1, 1), step(2, 2), step(3, 3)];
    let last = Plan::last_uses(&repeated, 4, &mut budget).unwrap();
    let (_, textures) = Plan::texture_lifetimes(&repeated, &last, None, &mut budget).unwrap();
    assert_eq!(textures.len(), 3);
}

#[test]
fn local_images_schedule_capacities_without_live_aliases_and_charge_work() {
    let frame = prepared(tile());
    let limits = RasterLimits::default();
    let mut steps = Vec::new();
    for index in 0..96 {
        let bounds = PixelRect {
            left: 3,
            top: 5,
            right: 3 + (index % 17) as u32,
            bottom: 5 + (index % 11) as u32,
        };
        let mut parameters = GpuUniforms::new(&frame, GpuOp::Clear, bounds);
        parameters.images([bounds; 3]);
        steps.push(Step {
            first: index,
            second: index / 3,
            parameters,
            data: vec![1.0],
            quads: Vec::new(),
        });
    }
    let mut budget = Budget::new(frame.size, limits).unwrap();
    let last = Plan::last_uses(
        &steps,
        steps.len(),
        &mut Budget::new(frame.size, limits).unwrap(),
    )
    .unwrap();
    let (slots, sizes) = Plan::texture_lifetimes(&steps, &last, None, &mut budget).unwrap();
    assert_eq!(sizes[0], PixelSize::new(1, 1));
    for (position, step) in steps.iter().enumerate() {
        let slot = slots[position + 1];
        assert_ne!(slot, slots[step.first]);
        assert_ne!(slot, slots[step.second]);
        let size = image_size(step.parameters.image_bounds(2));
        assert!(sizes[slot].width >= size.width && sizes[slot].height >= size.height);
        for image in 1..=position {
            if last[image] >= position {
                assert_ne!(slots[image], slot);
            }
        }
    }
    assert!(sizes.len() < steps.len());
    assert!(sizes.iter().any(|size| size.width > 1 || size.height > 1));
    let exact = RasterLimits {
        max_work: budget.work_done(),
        ..limits
    };
    assert!(
        Plan::texture_lifetimes(
            &steps,
            &last,
            None,
            &mut Budget::new(frame.size, exact).unwrap()
        )
        .is_ok()
    );
    let short = RasterLimits {
        max_work: exact.max_work - 1,
        ..exact
    };
    assert_eq!(
        Plan::texture_lifetimes(
            &steps,
            &last,
            None,
            &mut Budget::new(frame.size, short).unwrap()
        )
        .unwrap_err()
        .field,
        "max_work"
    );
}

#[test]
#[ignore = "requires native GPU; local origins, changing capacities, filters and masks"]
fn local_images_preserve_filter_neighborhoods_transparency_and_replay() -> motion_core::Result {
    let mut gpu = Gpu::new()?;
    let limits = RasterLimits::default();
    let mut matrix = [0.0; 20];
    for channel in 0..4 {
        matrix[channel * 5 + channel] = 1.0;
    }
    matrix[4] = 0.1;
    matrix[19] = 0.2;
    let mut filters = filter_cases();
    filters.push(motion_core::Filter::Custom(vec![
        motion_core::FilterOperation::ColorMatrix(matrix),
    ]));
    let mut count = 0;
    for width in [65, 128, 257] {
        for (index, filter) in filters.iter().enumerate() {
            let marks = [1.0, 5.0, 17.0, 2.0].map(|extent| {
                Node::from(
                    Rect::new(0.25, 0.5, extent + 0.75, extent + 1.25)
                        .fill(Color::from_rgb_float(1.2, 0.2, 0.4).with_alpha(0.6)),
                )
                .filter(filter.clone())
                .translate(7.25 + extent, 5.75 + extent)
                .mask(
                    Node::from(Rect::new(3.0, 2.0, 18.0, 19.0).fill(Color::WHITE)).opacity(0.4),
                    motion_core::MaskMode::InvertedLuminance,
                )
                .clip_with(
                    Rect::new(0.5, 0.25, 40.5, 39.75),
                    motion_core::ClipMode::Geometric,
                )
                .opacity(0.7)
            });
            let frame = Node::group(marks)
                .compile(
                    Size::new(f64::from(width), 53.0),
                    PixelSize::new(width, 53),
                    ValidationLimits::default(),
                )?
                .prepare(ValidationLimits::default(), limits)?;
            let expected = frame.render_linear(limits)?;
            let plan = Plan::new(&frame, limits)?;
            assert!(plan.steps.iter().any(|step| {
                let bounds = step.parameters.image_bounds(2);
                bounds.left > 0
                    && bounds.top > 0
                    && image_bytes(bounds) < image_bytes(frame.output_bounds)
            }));
            let actual = gpu.render(&frame, plan)?;
            for (result, reference) in actual.pixels().iter().zip(expected.pixels()) {
                for channel in 0..4 {
                    assert!(
                        (result[channel] - reference[channel]).abs() < 3e-6,
                        "width={width} filter={index}: {result:?} != {reference:?}"
                    );
                }
            }
            let actual = actual.to_srgb_rgba8(false, limits)?;
            let expected = expected.to_srgb_rgba8(false, limits)?;
            assert!(
                actual
                    .bytes()
                    .iter()
                    .zip(expected.bytes())
                    .all(|(result, reference)| result.abs_diff(*reference) <= 1)
            );
            assert_eq!(
                actual,
                gpu.render(&frame, Plan::new(&frame, limits)?)?
                    .to_srgb_rgba8(false, limits)?
            );
            count += 1;
        }
    }
    println!("local_image_cases={count} cpu_gpu=pass replay=exact");
    Ok(())
}

#[test]
#[ignore = "requires a native GPU; deliberately destroys only its test device"]
fn gpu_device_loss_callback_diagnostics() -> motion_core::Result {
    let mut gpu = Gpu::new()?;
    let info = gpu.info().clone();
    gpu.device.destroy();
    let _ = wait_for_gpu(&gpu.device, None);
    let frame = prepared(tile());
    let plan = Plan::new(&frame, RasterLimits::default())?;
    let error = gpu.render(&frame, plan).unwrap_err().at_frame(4);
    assert_eq!(error.kind(), motion_core::ErrorKind::DeviceLost);
    assert_eq!(error.frame(), Some(4));
    assert!(std::error::Error::source(&error).is_some());
    assert!(error.to_string().contains("Destroyed"));
    eprintln!(
        "gpu_device_loss adapter={} backend={:?} frame=4 kind=DeviceLost source=retained",
        info.name, info.backend
    );
    Ok(())
}

#[test]
fn gpu_shader_validates_without_a_device() {
    let source = context::textures::shader_source();
    let module = wgpu::naga::front::wgsl::parse_str(&source)
        .unwrap_or_else(|error| panic!("{}", error.emit_to_string(&source)));
    wgpu::naga::valid::Validator::new(
        wgpu::naga::valid::ValidationFlags::all(),
        wgpu::naga::valid::Capabilities::empty(),
    )
    .validate(&module)
    .unwrap();
    for name in ["particle_state", "particle_noise", "particle_hash"] {
        assert!(
            module
                .functions
                .iter()
                .all(|(_, function)| function.name.as_deref() != Some(name)),
            "Trajectory evaluation must remain in content, not WGSL: {name}"
        );
    }
    let uniform = module
        .global_variables
        .iter()
        .find(|(_, variable)| {
            variable
                .binding
                .as_ref()
                .is_some_and(|binding| binding.binding == 0)
        })
        .unwrap()
        .1;
    assert!(matches!(
        module.types[uniform.ty].inner,
        wgpu::naga::TypeInner::Struct { span: 256, .. }
    ));
    assert_eq!(module.entry_points.len(), 10);
    for (name, stage) in [
        ("texture_main", wgpu::naga::ShaderStage::Compute),
        ("texture_check", wgpu::naga::ShaderStage::Compute),
        ("prepare_particle_states", wgpu::naga::ShaderStage::Compute),
        ("count_particle_tiles", wgpu::naga::ShaderStage::Compute),
        ("prefix_particle_tiles", wgpu::naga::ShaderStage::Compute),
        ("fill_particle_tiles", wgpu::naga::ShaderStage::Compute),
        ("texture_vertex", wgpu::naga::ShaderStage::Vertex),
        ("texture_backdrop", wgpu::naga::ShaderStage::Fragment),
        ("texture_source", wgpu::naga::ShaderStage::Fragment),
        ("texture_resolve", wgpu::naga::ShaderStage::Compute),
    ] {
        let entry = module
            .entry_points
            .iter()
            .find(|entry| entry.name == name)
            .expect("shader entry point");
        assert_eq!(entry.stage, stage);
        if stage == wgpu::naga::ShaderStage::Compute {
            assert_eq!(
                entry.workgroup_size,
                if name == "prefix_particle_tiles" {
                    [1, 1, 1]
                } else if name == "prepare_particle_states"
                    || name == "count_particle_tiles"
                    || name == "fill_particle_tiles"
                {
                    [64, 1, 1]
                } else {
                    [8, 8, 1]
                }
            );
        }
    }
}

#[test]
fn gpu_plan_admits_prepared_layers_without_a_device() {
    let node = Node::from(Rect::new(0.0, 0.0, 2.0, 1.0).fill(Color::WHITE)).opacity(0.5);
    let prepared = node
        .compile(
            Size::new(2.0, 1.0),
            PixelSize::new(2, 1),
            ValidationLimits::default(),
        )
        .unwrap()
        .prepare(ValidationLimits::default(), RasterLimits::default())
        .unwrap();
    let plan = Plan::new(&prepared, RasterLimits::default()).unwrap();
    assert!(plan.stats.work > 0);
    assert_eq!(plan.size, PixelSize::new(2, 1));
}

fn prepared(node: Node) -> PreparedFrame {
    node.compile(
        Size::new(8.0, 4.0),
        PixelSize::new(8, 4),
        ValidationLimits::default(),
    )
    .unwrap()
    .prepare(ValidationLimits::default(), RasterLimits::default())
    .unwrap()
}

fn tile() -> Node {
    Rect::new(1.0, 1.0, 4.0, 3.0).fill(Color::WHITE).into()
}

#[test]
fn gpu_plan_exact_budgets_are_retryable_and_device_limits_are_explicit() {
    let frame = prepared(
        tile()
            .filter(motion_core::Filter::Blur { sigma: 0.5 })
            .opacity(0.7),
    );
    let unchanged = frame.clone();
    let plan = Plan::new(&frame, RasterLimits::default()).unwrap();
    let padding = plan
        .texture_data_sizes
        .iter()
        .enumerate()
        .filter(|(_, bytes)| **bytes != 0)
        .map(|(position, bytes)| {
            let payload = plan.steps[plan.texture_run(position)]
                .iter()
                .map(|step| step.data.len() as u64 * size_of::<f32>() as u64)
                .sum::<u64>();
            bytes - payload
        })
        .sum::<u64>();
    let metadata = plan.texture_data_sizes.len() as u64;
    let limits = RasterLimits {
        max_bytes: plan.stats.peak_bytes - metadata * size_of::<u64>() as u64 - padding * 2,
        max_work: plan.stats.work - metadata * 4 - padding / size_of::<f32>() as u64 * 2,
        ..RasterLimits::default()
    };
    assert!(Plan::new(&frame, limits).is_ok());
    let fallback = Plan::new(&frame, limits).unwrap();
    assert!(fallback.texture_data_sizes.is_empty());
    assert_eq!(fallback.stats.peak_bytes, limits.max_bytes);
    assert_eq!(fallback.stats.work, limits.max_work);
    for (limits, field) in [
        (
            RasterLimits {
                max_bytes: limits.max_bytes - 1,
                ..limits
            },
            "max_bytes",
        ),
        (
            RasterLimits {
                max_work: limits.max_work - 1,
                ..limits
            },
            "max_work",
        ),
        (
            RasterLimits {
                max_pixels: 1,
                ..limits
            },
            "max_pixels",
        ),
    ] {
        let error = Plan::new(&frame, limits).err().unwrap();
        assert_eq!(
            (error.kind, error.field.as_str()),
            (ValidationErrorKind::StructuralLimit, field)
        );
    }
    for device in [
        wgpu::Limits {
            max_storage_buffer_binding_size: 4,
            ..Default::default()
        },
        wgpu::Limits {
            max_buffer_size: 4,
            ..Default::default()
        },
        wgpu::Limits {
            max_compute_workgroups_per_dimension: 0,
            ..Default::default()
        },
        wgpu::Limits {
            max_texture_dimension_2d: 0,
            ..Default::default()
        },
    ] {
        assert_eq!(
            plan.validate_device(&device).unwrap_err().kind,
            ValidationErrorKind::Unsupported
        );
    }
    assert!(plan.validate_device(&wgpu::Limits::default()).is_ok());
    assert_eq!(frame, unchanged);
    assert_eq!(Plan::new(&frame, limits).unwrap().stats, fallback.stats);
    assert_eq!(
        Plan::new(&frame, RasterLimits::default()).unwrap().stats,
        plan.stats
    );
}

#[test]
fn gpu_plan_retains_shared_coverage_and_conditional_geometric_clips() {
    let frame = prepared(
        Node::from(Rect::new(0.0, 0.0, 8.0, 4.0).fill(Color::WHITE))
            .clip_with(
                Rect::new(0.5, 0.0, 7.5, 4.0),
                motion_core::ClipMode::Geometric,
            )
            .clip_with(
                Rect::new(0.5, 0.0, 7.5, 4.0),
                motion_core::ClipMode::Geometric,
            ),
    );
    let plan = Plan::new(&frame, RasterLimits::default()).unwrap();
    let fill = plan
        .steps
        .iter()
        .find(|step| step.parameters.operation.code == 1)
        .unwrap();
    assert_eq!(
        fill.data[..4]
            .iter()
            .map(|value| value.to_bits())
            .collect::<Vec<_>>(),
        [1 << 31, 0, 8, 4]
    );
    assert!(fill.data[4..].iter().all(|value| *value == 1.0));
    let layer = plan.steps.last().unwrap();
    assert_eq!(layer.parameters.operation.code, 2);
    assert_eq!(
        layer.data[..6]
            .iter()
            .map(|value| value.to_bits())
            .collect::<Vec<_>>(),
        [CONDITIONAL_COVERAGE, layer.data.len() as u32, 0, 0, 8, 4]
    );
    let planner = Planner {
        frame: &frame,
        steps: Vec::new(),
        budget: Budget::new(frame.size, RasterLimits::default()).unwrap(),
    };
    for row in 0..4 {
        assert_eq!(planner.coverage_at(&layer.data, 0, row), 128.0 / 255.0);
        assert_eq!(planner.coverage_at(&layer.data, 1, row), 1.0);
    }
    let PreparedItem::PushLayer { ref layer } = frame.items[0] else {
        panic!("shape isolation")
    };
    let shape_clip = layer.geometric_clip.expect("cumulative shape domain");
    assert_eq!(frame.clips[shape_clip].bounds.left, 0);
    for (index, step) in plan.steps.iter().enumerate() {
        assert!(step.first <= index && step.second <= index);
    }
}

#[test]
fn compact_coverage_uploads_do_not_expand_solid_or_small_shapes_to_canvas() {
    for width in [128, 1920] {
        let size = PixelSize::new(width, 1080);
        for bounds in [
            Rect::new(0.0, 0.0, f64::from(width), 1080.0),
            Rect::new(13.25, 17.5, 23.75, 29.25),
        ] {
            let frame = Node::from(bounds.fill(Color::WHITE))
                .compile(
                    Size::new(f64::from(width), 1080.0),
                    size,
                    ValidationLimits::default(),
                )
                .unwrap()
                .prepare(ValidationLimits::default(), RasterLimits::default())
                .unwrap();
            let plan = Plan::new(&frame, RasterLimits::default()).unwrap();
            let fill = plan
                .steps
                .iter()
                .find(|step| step.parameters.operation.code == 1)
                .unwrap();
            assert!(fill.data.len() < 128, "{width}: {}", fill.data.len());
            let planner = Planner {
                frame: &frame,
                steps: Vec::new(),
                budget: Budget::new(size, RasterLimits::default()).unwrap(),
            };
            let local = planner.coverage_bounds(&fill.data);
            let mut expected = vec![0u8; (size.width * size.height) as usize];
            for strip in frame.strips() {
                for row in 0..strip.height {
                    for column in 0..strip.width {
                        expected[((strip.y + row) * size.width + strip.x + column) as usize] =
                            frame.alphas()
                                [strip.alpha_offset + (row * strip.width + column) as usize];
                    }
                }
            }
            for span in frame.spans() {
                for row in span.y..span.y + span.height {
                    for column in span.start_x..span.end_x {
                        expected[(row * size.width + column) as usize] = 255;
                    }
                }
            }
            for row in local.top.saturating_sub(1)..(local.bottom + 1).min(size.height) {
                for column in local.left.saturating_sub(1)..(local.right + 1).min(size.width) {
                    assert_eq!(
                        planner.coverage_at(&fill.data, column, row).to_bits(),
                        (f32::from(expected[(row * size.width + column) as usize]) / 255.0)
                            .to_bits()
                    );
                }
            }
        }
    }
    for alpha in 1u32..255 {
        let power = 31 - alpha.leading_zeros();
        let significand = ((alpha << (31 - power)) + 127) / 255;
        let bits = ((power + 119) << 23) | (significand & 0x7fffff);
        assert_eq!(bits, (alpha as f32 / 255.0).to_bits(), "alpha {alpha}");
    }
}

#[test]
#[ignore = "requires native GPU; checks every packed u8 alpha against original CPU conversion"]
fn compact_coverage_all_alpha_bytes_match_cpu_exactly() -> motion_core::Result<()> {
    let frame = motion_core::DisplayList {
        output_size: PixelSize::new(256, 1),
        background: None,
        commands: (0..256)
            .map(|alpha| motion_core::DrawCommand::FillRect {
                bounds: Rect::new(alpha as f64, 0.0, alpha as f64 + alpha as f64 / 255.0, 1.0),
                transform: motion_core::Transform::IDENTITY,
                paint: motion_core::Paint::Solid(Color::WHITE),
            })
            .collect(),
    }
    .prepare(ValidationLimits::default(), RasterLimits::default())?;
    let expected = frame.render_linear(RasterLimits::default())?;
    let mut gpu = Gpu::new()?;
    let actual = gpu.render(&frame, Plan::new(&frame, RasterLimits::default())?)?;
    for (index, (actual, expected)) in actual.pixels().iter().zip(expected.pixels()).enumerate() {
        assert_eq!(
            actual.map(f32::to_bits),
            expected.map(f32::to_bits),
            "alpha {index}"
        );
    }
    let replay = gpu.render(&frame, Plan::new(&frame, RasterLimits::default())?)?;
    assert_eq!(actual.pixels(), replay.pixels());
    Ok(())
}

fn filter_cases() -> Vec<motion_core::Filter> {
    use motion_core::{Filter, FilterOperation as Operation, Vector2};
    let mut matrix = [0.0; 20];
    for channel in 0..4 {
        matrix[channel * 5 + channel] = 1.0;
    }
    matrix[4] = 0.1;
    vec![
        Filter::Blur { sigma: 0.5 },
        Filter::Blur { sigma: 2.0 },
        Filter::Blur { sigma: 5.0 },
        Filter::Blur { sigma: 9.0 },
        Filter::Blur { sigma: 14.0 },
        Filter::DropShadow {
            offset: Vector2::new(0.5, 1.0),
            sigma: 0.5,
            color: Color::BLACK.with_alpha(0.4),
        },
        Filter::Glow {
            threshold: 0.2,
            sigma: 0.5,
            intensity: 0.5,
        },
        Filter::DropShadow {
            offset: Vector2::new(0.5, 1.0),
            sigma: 5.0,
            color: Color::BLACK.with_alpha(0.4),
        },
        Filter::Glow {
            threshold: 0.2,
            sigma: 9.0,
            intensity: 0.5,
        },
        Filter::ColorAdjust {
            exposure: 0.2,
            contrast: 0.8,
            saturation: 0.7,
            hue_degrees: 30.0,
        },
        Filter::Optics(motion_core::Optics::Lens {
            distortion: 0.0,
            chromatic: 0.02,
            auto_fit: false,
            center: None,
        }),
        Filter::Optics(motion_core::Optics::LensVignette {
            angle_of_view: 90.0,
            strength: 0.4,
            roundness: 0.0,
        }),
        Filter::Custom(vec![Operation::Noise {
            seed: u64::MAX - 71,
            amplitude: 0.1,
        }]),
        Filter::Custom(vec![
            Operation::Blur { sigma: 0.5 },
            Operation::Offset { x: -0.5, y: 0.25 },
            Operation::ColorMatrix(matrix),
            Operation::Threshold { value: 0.1 },
            Operation::Noise {
                seed: 42,
                amplitude: 0.1,
            },
            Operation::ChannelOffset {
                red: Vector2::new(0.25, 0.0),
                green: Vector2::new(0.0, 0.5),
                blue: Vector2::new(-0.25, 0.0),
            },
            Operation::CompositeWithSource {
                mode: BlendMode::Difference,
            },
        ]),
    ]
}

#[test]
fn gpu_plan_covers_all_filters_and_preserves_input_snapshot_and_guard_band() {
    let mut operations = std::collections::BTreeSet::new();
    for filter in filter_cases() {
        let frame = prepared(tile().filter(filter));
        let plan = Plan::new(&frame, RasterLimits::default()).unwrap();
        for step in &plan.steps {
            operations.insert(step.parameters.operation.code);
        }
        assert_eq!(frame.size(), PixelSize::new(8, 4));
        assert_eq!(plan.size, frame.raster_size());
        for step in &plan.steps {
            assert_eq!(
                &[step.parameters.frame.left, step.parameters.frame.top],
                &[frame.output_bounds.left, frame.output_bounds.top]
            );
            if matches!(step.parameters.operation.code, 1 | 2) && step.data.len() > 1 {
                let planner = Planner {
                    frame: &frame,
                    steps: Vec::new(),
                    budget: Budget::new(frame.raster_size(), RasterLimits::default()).unwrap(),
                };
                let bounds = planner.coverage_bounds(&step.data);
                assert!(
                    bounds.right <= frame.raster_size().width
                        && bounds.bottom <= frame.raster_size().height
                );
                for row in bounds.top..bounds.bottom {
                    for column in bounds.left..bounds.right {
                        let alpha = planner.coverage_at(&step.data, column, row);
                        assert!(alpha.is_finite() && (0.0..=1.0).contains(&alpha));
                    }
                }
                let words = if step.data[0].to_bits() & (1 << 31) != 0 {
                    5
                } else if step.data[1].to_bits() & (1 << 31) != 0 {
                    4 + bounds.pixels().div_ceil(4) as usize
                } else {
                    4 + bounds.pixels() as usize
                };
                assert!(step.data[words..].iter().all(|value| value.is_finite()));
            } else {
                assert!(step.data.iter().all(|value| value.is_finite()));
            }
        }
        if let Some(step) = plan
            .steps
            .iter()
            .find(|step| step.parameters.operation.code == 2 && step.parameters.operation.kind == 6)
        {
            assert!(step.first < step.second);
            assert_eq!(plan.steps[step.first - 1].parameters.operation.code, 1);
        }
    }
    assert_eq!(
        operations,
        [0, 1, 2, 4, 5, 6, 7, 8, 9, 11, 12, 13, 16]
            .into_iter()
            .collect::<std::collections::BTreeSet<_>>()
    );
    let neutral = |count| {
        let frame = prepared(tile().filter(motion_core::Filter::Custom(vec![
            motion_core::FilterOperation::Blur { sigma: 0.0 };
            count
        ])));
        Plan::new(&frame, RasterLimits::default()).unwrap()
    };
    let empty = neutral(0);
    let repeated = neutral(4);
    assert_eq!(repeated.steps.len(), empty.steps.len());
    assert_eq!(repeated.stats.work, empty.stats.work + 4);
    let frame = prepared(tile().opacity(1e-50));
    assert_eq!(
        Plan::new(&frame, RasterLimits::default())
            .err()
            .unwrap()
            .kind,
        ValidationErrorKind::Unsupported
    );
}

#[test]
fn gpu_plan_records_every_blend_and_mask_without_cpu_fallback() {
    let modes = [
        BlendMode::SourceOver,
        BlendMode::Plus,
        BlendMode::Multiply,
        BlendMode::Screen,
        BlendMode::Darken,
        BlendMode::Lighten,
        BlendMode::Difference,
        BlendMode::Exclusion,
        BlendMode::Overlay,
        BlendMode::HardLight,
        BlendMode::SoftLight,
        BlendMode::ColorDodge,
        BlendMode::ColorBurn,
    ];
    for (index, blend) in modes.into_iter().enumerate() {
        let frame = prepared(tile().blend_mode(blend).opacity(0.5));
        let plan = Plan::new(&frame, RasterLimits::default()).unwrap();
        assert_eq!(
            plan.steps.last().unwrap().parameters.operation.kind,
            index as u32
        );
    }
    for (index, mask) in [
        motion_core::MaskMode::Alpha,
        motion_core::MaskMode::InvertedAlpha,
        motion_core::MaskMode::Luminance,
        motion_core::MaskMode::InvertedLuminance,
    ]
    .into_iter()
    .enumerate()
    {
        let plan = Plan::new(
            &prepared(tile().mask(tile(), mask)),
            RasterLimits::default(),
        )
        .unwrap();
        let step = plan
            .steps
            .iter()
            .find(|step| step.parameters.operation.code == 3)
            .unwrap();
        assert_eq!(step.parameters.operation.detail, index as u32);
    }
}

#[test]
#[ignore = "requires a native GPU; no window or video export"]
fn gpu_additional_separable_modes_match_cpu_and_replay() {
    let mut gpu = Gpu::new().unwrap();
    println!(
        "additional_blend_adapter name={} backend={:?}",
        gpu.info().name,
        gpu.info().backend
    );
    let pairs = [
        (
            Color::from_rgb_float(0.25, 0.75, 0.5).with_alpha(0.5),
            Color::from_rgb_float(0.75, 0.25, 0.5).with_alpha(0.5),
        ),
        (Color::BLACK, Color::WHITE),
        (Color::WHITE, Color::BLACK),
        (
            Color::from_rgb_float(1.2, -0.1, 0.2).with_alpha(0.7),
            Color::from_rgb_float(-0.1, 1.1, 0.8).with_alpha(0.4),
        ),
        (
            Color::from_rgb_float(0.1, 0.4, 0.7),
            Color::from_rgb_float(0.2, 0.8, 0.5),
        ),
    ];
    let mut cases = 0;
    let mut max_linear_difference = 0.0_f32;
    let mut max_rgba_difference = 0_u8;
    for mode in [
        BlendMode::Overlay,
        BlendMode::HardLight,
        BlendMode::SoftLight,
        BlendMode::ColorDodge,
        BlendMode::ColorBurn,
    ] {
        for (backdrop, source) in pairs {
            let rectangle = Rect::new(0.0, 0.0, 8.0, 4.0);
            let foreground = Node::from(rectangle.fill(source))
                .filter(motion_core::Filter::Custom(vec![
                    motion_core::FilterOperation::CompositeWithSource { mode },
                ]))
                .clip_with(
                    Rect::new(0.5, 0.25, 7.5, 3.75),
                    motion_core::ClipMode::Geometric,
                )
                .mask(tile().opacity(0.6), motion_core::MaskMode::Alpha)
                .opacity(0.65)
                .blend_mode(mode);
            let frame = prepared(Node::group([
                Node::from(rectangle.fill(backdrop)),
                foreground,
            ]));
            let unchanged = frame.clone();
            let cpu = frame.render_linear(RasterLimits::default()).unwrap();
            let actual = gpu
                .render(&frame, Plan::new(&frame, RasterLimits::default()).unwrap())
                .unwrap();
            for (actual, expected) in actual.pixels().iter().zip(cpu.pixels()) {
                for channel in 0..4 {
                    let difference = (actual[channel] - expected[channel]).abs();
                    max_linear_difference = max_linear_difference.max(difference);
                    assert!(difference < 3e-6, "{mode:?}: {actual:?} != {expected:?}");
                }
            }
            let output = actual
                .to_srgb_rgba8(false, RasterLimits::default())
                .unwrap();
            let expected = cpu.to_srgb_rgba8(false, RasterLimits::default()).unwrap();
            for (actual, expected) in output.pixels().iter().zip(expected.pixels()) {
                for channel in 0..4 {
                    let difference = actual[channel].abs_diff(expected[channel]);
                    max_rgba_difference = max_rgba_difference.max(difference);
                    assert!(difference <= 1, "{mode:?}: output difference {difference}");
                }
            }
            let replay = gpu
                .render(&frame, Plan::new(&frame, RasterLimits::default()).unwrap())
                .unwrap();
            assert_eq!(replay.pixels(), actual.pixels(), "{mode:?}");
            assert_eq!(frame, unchanged);
            cases += 1;
        }
    }
    println!(
        "additional_blend_parity cases={cases} requests={} pixels={} max_linear_difference={max_linear_difference:e} max_rgba_difference={max_rgba_difference} replay=pass",
        cases * 2,
        cases * 32
    );
}

#[test]
#[ignore = "requires separate authorization to initialize a native GPU device"]
fn gpu_device_matches_cpu_filters_layers_masks_and_shared_output() {
    let mut gpu = Gpu::new().unwrap();
    println!(
        "_gpu_adapter name={} backend={:?}",
        gpu.info().name,
        gpu.info().backend
    );
    let mut cases = vec![
        tile(),
        tile().opacity(0.5),
        tile().clip_with(
            Rect::new(1.5, 1.0, 3.5, 2.5),
            motion_core::ClipMode::Geometric,
        ),
        Node::group(Vec::<Node>::new()),
        tile().translate(-16.0, 0.0),
        Node::from(
            Rect::new(0.25, 0.5, 6.75, 3.5)
                .fill(Color::from_rgb_float(2.0, -0.5, 0.25).with_alpha(0.4)),
        )
        .opacity(0.7),
        Node::group([tile()
            .filter(motion_core::Filter::Blur { sigma: 0.5 })
            .clip_with(
                Rect::new(1.5, 0.5, 5.5, 3.5),
                motion_core::ClipMode::Geometric,
            )])
        .filter(motion_core::Filter::DropShadow {
            offset: motion_core::Vector2::new(-1.25, 0.5),
            sigma: 0.5,
            color: Color::BLACK.with_alpha(0.4),
        })
        .clip_with(
            Rect::new(0.5, 0.25, 7.5, 3.75),
            motion_core::ClipMode::Geometric,
        )
        .mask(
            tile().translate(-0.5, 0.0).opacity(0.6),
            motion_core::MaskMode::InvertedLuminance,
        ),
    ];
    let mut cap = motion_core::Path2D::new();
    cap.try_move_to(motion_core::Point::ORIGIN).unwrap();
    cap.try_quadratic_to(
        motion_core::Point::new(0.5, 2.0),
        motion_core::Point::new(1.0, 0.0),
    )
    .unwrap();
    let mut loop_path = motion_core::Path2D::new();
    loop_path
        .try_move_to(motion_core::Point::new(0.5, 0.0))
        .unwrap();
    loop_path
        .try_cubic_to(
            motion_core::Point::new(1.5, 4.0 / 3.0),
            motion_core::Point::new(-0.5, 4.0 / 3.0),
            motion_core::Point::new(0.5, 0.0),
        )
        .unwrap();
    cases.extend([
        Node::from(cap.clone().fill(Color::WHITE)),
        Node::from(loop_path.fill(Color::WHITE)),
        Node::from(Rect::new(0.0, 0.0, 1.0, 1.0).fill(Color::WHITE))
            .clip_with(cap.clone(), motion_core::ClipMode::Geometric),
        Node::from(cap.fill(Color::WHITE)).filter(motion_core::Filter::Blur { sigma: 0.5 }),
    ]);
    cases.extend(
        filter_cases()
            .into_iter()
            .map(|filter| tile().filter(filter)),
    );
    for blend in [
        BlendMode::SourceOver,
        BlendMode::Plus,
        BlendMode::Multiply,
        BlendMode::Screen,
        BlendMode::Darken,
        BlendMode::Lighten,
        BlendMode::Difference,
        BlendMode::Exclusion,
    ] {
        for mask in [
            motion_core::MaskMode::Alpha,
            motion_core::MaskMode::InvertedAlpha,
            motion_core::MaskMode::Luminance,
            motion_core::MaskMode::InvertedLuminance,
        ] {
            cases.push(Node::group([
                tile(),
                tile()
                    .translate(1.0, 0.0)
                    .blend_mode(blend)
                    .mask(tile().opacity(0.5), mask)
                    .clip_with(
                        Rect::new(0.5, 0.0, 6.5, 4.0),
                        motion_core::ClipMode::Geometric,
                    )
                    .opacity(0.7),
            ]));
        }
    }
    let count = cases.len();
    let mut max_linear_difference = 0.0_f32;
    let mut max_rgba_difference = 0_u8;
    let mut compared_pixels = 0;
    let mut large_difference_pixels = 0;
    for (index, node) in cases.into_iter().enumerate() {
        let frame = prepared(node);
        let cpu = frame.render_linear(RasterLimits::default()).unwrap();
        let plan = Plan::new(&frame, RasterLimits::default()).unwrap();
        let actual = gpu.render(&frame, plan).unwrap();
        assert_eq!(actual.size(), cpu.size());
        for (actual, expected) in actual.pixels().iter().zip(cpu.pixels()) {
            for channel in 0..4 {
                max_linear_difference =
                    max_linear_difference.max((actual[channel] - expected[channel]).abs());
                assert!(
                    (actual[channel] - expected[channel]).abs() < 3e-6,
                    "case {index}: {actual:?} != {expected:?}"
                );
            }
        }
        let actual = actual
            .to_srgb_rgba8(false, RasterLimits::default())
            .unwrap();
        let expected = cpu.to_srgb_rgba8(false, RasterLimits::default()).unwrap();
        let mut large_difference = 0;
        for (actual, expected) in actual.pixels().iter().zip(expected.pixels()) {
            let delta = actual
                .iter()
                .zip(expected)
                .map(|(actual, expected)| actual.abs_diff(*expected))
                .max()
                .unwrap();
            assert!(delta <= 2, "case {index}: output difference {delta}");
            max_rgba_difference = max_rgba_difference.max(delta);
            large_difference += usize::from(delta > 1);
        }
        assert!((large_difference as f64) / (actual.pixels().len() as f64) < 0.001);
        compared_pixels += actual.pixels().len();
        large_difference_pixels += large_difference;
        let replay = gpu
            .render(&frame, Plan::new(&frame, RasterLimits::default()).unwrap())
            .unwrap();
        assert_eq!(
            replay
                .to_srgb_rgba8(false, RasterLimits::default())
                .unwrap(),
            actual
        );
    }
    let overflow = prepared(
        Node::from(Rect::new(1.0, 1.0, 4.0, 3.0).fill(Color::from_rgb_float(2.0, 2.0, 2.0)))
            .filter(motion_core::Filter::Custom(vec![
                motion_core::FilterOperation::ColorMatrix([
                    f64::from(f32::MAX),
                    0.0,
                    0.0,
                    0.0,
                    0.0,
                    0.0,
                    1.0,
                    0.0,
                    0.0,
                    0.0,
                    0.0,
                    0.0,
                    1.0,
                    0.0,
                    0.0,
                    0.0,
                    0.0,
                    0.0,
                    1.0,
                    0.0,
                ]),
            ])),
    );
    let cpu_error = overflow.render_linear(RasterLimits::default()).unwrap_err();
    let gpu_error = gpu
        .render(
            &overflow,
            Plan::new(&overflow, RasterLimits::default()).unwrap(),
        )
        .unwrap_err();
    let gpu_error = gpu_error.downcast_ref::<ValidationError>().unwrap();
    assert_eq!(cpu_error.kind, ValidationErrorKind::InvalidValue);
    assert_eq!(
        (gpu_error.kind, gpu_error.field.as_str()),
        (ValidationErrorKind::InvalidValue, "color")
    );
    let healthy = prepared(tile());
    assert_eq!(
        gpu.render(
            &healthy,
            Plan::new(&healthy, RasterLimits::default()).unwrap()
        )
        .unwrap()
        .pixels(),
        healthy
            .render_linear(RasterLimits::default())
            .unwrap()
            .pixels()
    );
    println!(
        "_gpu_parity cases={count} pixels={compared_pixels} max_linear_difference={max_linear_difference:e} max_rgba_difference={max_rgba_difference} pixels_over_one={large_difference_pixels} replay=pass overflow_recovery=pass"
    );
}
