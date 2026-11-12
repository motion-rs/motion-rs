//! Tile interiors, seams, static admission and explicit CPU path diagnostics.

use super::*;
use crate::gpu::visual::VisualTolerance;
use motion_core::{Color, FillRule, Node, Path2D, Point, Rect, Size};
use wgpu::util::DeviceExt;

fn prepare_cpu(node: Node) -> motion_core::Result<crate::PreparedCpuFrame> {
    let size = PixelSize::new(193, 81);
    let validation = ValidationLimits::default();
    let list = node.compile(Size::new(193.0, 81.0), size, validation)?;
    Ok(list.prepare(validation, RasterLimits::default())?)
}

fn prepare(node: Node) -> motion_core::Result<PreparedGpuFrame> {
    let size = PixelSize::new(193, 81);
    let validation = ValidationLimits::default();
    let list = node.compile(Size::new(193.0, 81.0), size, validation)?;
    Ok(crate::gpu::raster::prepare::prepare_validated_device(
        &list,
        validation,
        RasterLimits::default(),
        1.0 / 256.0,
        None,
    )?)
}

fn contour(path: &mut Path2D, bounds: Rect) -> motion_core::Result {
    path.try_move_to(Point::new(bounds.left, bounds.top))?;
    path.try_line_to(Point::new(bounds.right, bounds.top))?;
    path.try_line_to(Point::new(bounds.right, bounds.bottom))?;
    path.try_line_to(Point::new(bounds.left, bounds.bottom))?;
    path.try_close()?;
    Ok(())
}

fn ring(rule: FillRule) -> motion_core::Result<Node> {
    let mut path = Path2D::new();
    contour(&mut path, Rect::new(-8.0, 0.25, 192.0, 80.75))?;
    contour(&mut path, Rect::new(32.0, 16.0, 160.0, 64.0))?;
    Ok(Node::from(path.fill(Color::WHITE).fill_rule(rule)))
}

#[test]
fn tile_descriptors_admit_scratch_and_report_geometry_uploads() -> motion_core::Result {
    let frame = prepare(ring(FillRule::EvenOdd)?)?;
    let plan = Plan::new(&frame, RasterLimits::default())?;
    let stats = frame.geometry.stats(frame.stats());
    assert_eq!(stats.segment_count, 4);
    assert_eq!(stats.tile_count, 12 * 6);
    assert_eq!(stats.geometry_upload_bytes, 48 + 4 * 32 + 16);
    assert_eq!(u64::from(plan.device.tiles), stats.tile_count);
    assert_eq!(u64::from(plan.device.indices), stats.tile_index_capacity);
    assert_eq!(
        plan.device.buffer_sizes()[2..5].iter().sum::<u64>(),
        stats.tile_count * (16 + 16 * 16 * 4) + stats.tile_index_capacity.max(1) * 4
    );
    assert_eq!(plan.device.fills[0].columns, 12);
    assert_eq!(plan.device.fills[0].tile_rows, 6);
    assert_eq!(plan.device.fills[0].index_capacity, plan.device.indices);
    // Test the tile stage directly: full planning may shrink optional fill batches.
    let admit = |limits| -> motion_core::Result<(u64, u64)> {
        let mut planner = Planner {
            frame: frame.gpu_view(),
            steps: Vec::new(),
            budget: Budget::new(frame.records.size, limits)?,
        };
        planner.device_coverage()?;
        Ok((planner.budget.peak_bytes(), planner.budget.work_done()))
    };
    let (bytes, work) = admit(RasterLimits::default())?;
    let exact = RasterLimits {
        max_bytes: bytes,
        max_work: work,
        ..RasterLimits::default()
    };
    assert_eq!(admit(exact)?, (bytes, work));
    for below in [
        RasterLimits {
            max_bytes: exact.max_bytes - 1,
            ..exact
        },
        RasterLimits {
            max_work: exact.max_work - 1,
            ..exact
        },
    ] {
        assert!(admit(below).is_err());
    }
    let device_limits = wgpu::Limits {
        max_storage_buffer_binding_size: 1,
        ..Default::default()
    };
    assert_eq!(
        plan.validate_device(&device_limits).unwrap_err().field,
        "buffer_size"
    );
    let device_limits = wgpu::Limits {
        max_compute_workgroups_per_dimension: 1,
        ..Default::default()
    };
    assert_eq!(
        plan.validate_device(&device_limits).unwrap_err().field,
        "dispatch"
    );
    Ok(())
}

#[test]
fn unsupported_device_paths_are_validation_errors_not_cpu_coverage() -> motion_core::Result {
    let mut huge = Path2D::new();
    contour(&mut huge, Rect::new(-9000.0, 0.25, 100.0, 40.75))?;
    let error = prepare(Node::from(huge.fill(Color::WHITE))).unwrap_err();
    assert!(error.to_string().contains("f32 device range"), "{error}");
    let mut dense = Path2D::new();
    for _ in 0..2049 {
        contour(&mut dense, Rect::new(0.1, 0.1, 0.9, 0.9))?;
    }
    let error = prepare(Node::from(dense.fill(Color::WHITE))).unwrap_err();
    assert!(error.to_string().contains("tile edge density"), "{error}");
    let frame = prepare(Node::group([
        ring(FillRule::EvenOdd)?,
        Node::from(Rect::new(0.0, 0.0, 1.0, 1.0).fill(Color::WHITE)),
    ]))?;
    assert_eq!(frame.geometry.device_fills.len(), 1);
    Ok(())
}

#[test]
fn shader_coverage_structs_match_host_offsets() {
    let shader =
        wgpu::naga::front::wgsl::parse_str(include_str!("../../shaders/coverage.wgsl")).unwrap();
    for (name, size, offsets) in [
        ("CoverageParams", 16, vec![0, 4, 8, 12]),
        (
            "CoverageFill",
            48,
            vec![0, 8, 12, 16, 20, 24, 28, 32, 36, 40, 44],
        ),
        ("CoverageEdge", 32, vec![0, 16, 20]),
        ("CoverageCurve", 48, vec![0, 16, 32, 36, 40, 44]),
        ("CoverageTile", 16, vec![0, 4, 8, 12]),
    ] {
        let (_, definition) = shader
            .types
            .iter()
            .find(|(_, ty)| ty.name.as_deref() == Some(name))
            .unwrap();
        let wgpu::naga::TypeInner::Struct { members, span } = &definition.inner else {
            panic!("expected {name} struct")
        };
        assert_eq!(*span, size, "{name}");
        assert_eq!(
            members
                .iter()
                .map(|member| member.offset)
                .collect::<Vec<_>>(),
            offsets,
            "{name}"
        );
    }
}

#[test]
#[ignore = "requires native GPU; boundary rows against analytic height reference"]
fn boundary_rows_match_analytic_height_reference() -> motion_core::Result {
    use crate::gpu::abi::{CoverageEdge, CoverageFill, CoverageParams, CoverageTile};
    use crate::gpu::raster::device::COVERAGE_BOUNDARY_STRIDE;

    let context = GpuContext::new()?;
    let mut fills = Vec::new();
    let mut edges = Vec::new();
    let mut tiles = Vec::new();
    let mut indices = Vec::new();
    let mut expected = Vec::new();
    let mut row_count = 0;
    for columns in [1u32, 63, 64, 65, 129] {
        let fill_index = fills.len() as u32;
        let first_tile = tiles.len() as u32;
        let first_row = row_count;
        let tile_rows = 2;
        fills.push(CoverageFill {
            size: [columns * 16, 32],
            first_tile,
            columns,
            first_row,
            tile_rows,
            ..Default::default()
        });
        row_count += tile_rows * 16;
        for band in 0..tile_rows {
            let offset = indices.len() as u32;
            for column in 0..columns {
                let x = if column == 0 {
                    -3.0
                } else {
                    column as f32 * 16.0 + 1.0
                };
                let top = band as f32 * 16.0 + 0.125;
                let bottom = band as f32 * 16.0 + 15.625;
                for direction in [true, false, true] {
                    let points = if direction {
                        [x, top, x, bottom]
                    } else {
                        [x, bottom, x, top]
                    };
                    indices.push(edges.len() as u32);
                    edges.push(CoverageEdge {
                        points,
                        fill: fill_index,
                        ..Default::default()
                    });
                }
                for lane in 0..16 {
                    let y = (band * 16 + lane as u32) as f32;
                    let height = (bottom.min(y + 1.0) - top.max(y)).max(0.0);
                    expected.extend([height; 16]);
                }
                tiles.push(CoverageTile {
                    count: indices.len() as u32 - offset,
                    offset,
                    ..Default::default()
                });
            }
        }
    }
    let params = CoverageParams {
        fills: fills.len() as u32,
        edges: edges.len() as u32,
        tiles: tiles.len() as u32,
        rows: row_count,
    };
    let upload = |contents: &[u8], usage| {
        context
            .device
            .create_buffer_init(&wgpu::util::BufferInitDescriptor {
                label: None,
                contents,
                usage,
            })
    };
    let uniforms = upload(bytemuck::bytes_of(&params), wgpu::BufferUsages::UNIFORM);
    let fills = upload(bytemuck::cast_slice(&fills), wgpu::BufferUsages::STORAGE);
    let edges = upload(bytemuck::cast_slice(&edges), wgpu::BufferUsages::STORAGE);
    let tiles = upload(bytemuck::cast_slice(&tiles), wgpu::BufferUsages::STORAGE);
    let indices = upload(bytemuck::cast_slice(&indices), wgpu::BufferUsages::STORAGE);
    let bytes = u64::from(params.tiles) * 16 * COVERAGE_BOUNDARY_STRIDE * 4;
    let carries = context.device.create_buffer(&wgpu::BufferDescriptor {
        label: None,
        size: bytes,
        usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_SRC,
        mapped_at_creation: false,
    });
    let coverage = upload(&[0; 48], wgpu::BufferUsages::STORAGE);
    let curves = upload(&[0; 48], wgpu::BufferUsages::STORAGE);
    let buffers = [
        &uniforms, &fills, &edges, &tiles, &indices, &carries, &coverage, &curves,
    ];
    let pipelines = context.coverage_pipelines();
    let entries: Vec<_> = buffers
        .iter()
        .enumerate()
        .map(|(binding, buffer)| wgpu::BindGroupEntry {
            binding: binding as u32,
            resource: buffer.as_entire_binding(),
        })
        .collect();
    let bindings = context
        .device
        .create_bind_group(&wgpu::BindGroupDescriptor {
            label: None,
            layout: &pipelines.layout,
            entries: &entries,
        });
    let readback = context.device.create_buffer(&wgpu::BufferDescriptor {
        label: None,
        size: bytes,
        usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
        mapped_at_creation: false,
    });
    let mut encoder = context.device.create_command_encoder(&Default::default());
    for (index, groups) in [(6, params.tiles * 16)] {
        let mut pass = encoder.begin_compute_pass(&Default::default());
        pass.set_pipeline(&pipelines.passes[index]);
        pass.set_bind_group(0, &bindings, &[]);
        let width = groups.min(7);
        pass.dispatch_workgroups(width, groups.div_ceil(width), 1);
    }
    encoder.copy_buffer_to_buffer(&carries, 0, &readback, 0, bytes);
    context.queue.submit([encoder.finish()]);
    let (sender, receiver) = std::sync::mpsc::channel();
    readback
        .slice(..)
        .map_async(wgpu::MapMode::Read, move |result| {
            sender.send(result).unwrap();
        });
    wait_for_gpu(&context.device, None)?;
    receiver.recv().unwrap().unwrap();
    let mapped = readback.slice(..).get_mapped_range().unwrap();
    let actual = bytemuck::cast_slice::<u8, f32>(&mapped);
    for (index, expected) in expected.into_iter().enumerate() {
        let actual = actual[index];
        assert!(
            (actual - expected).abs() <= 1e-4,
            "tile row {index}: {actual} != {expected}"
        );
    }
    drop(mapped);
    readback.unmap();
    context.check_device()?;
    Ok(())
}

#[test]
#[ignore = "requires native GPU; tile interiors, holes, seams and pooled scratch replay"]
fn tile_interiors_fill_rules_and_reused_scratch_match_cpu() -> motion_core::Result {
    let limits = RasterLimits::default();
    let mut gpu = Gpu::new()?;
    for rule in [FillRule::NonZero, FillRule::EvenOdd, FillRule::NonZero] {
        let frame = prepare(ring(rule)?)?;
        assert_eq!(frame.geometry.device_fills.len(), 1);
        let expected = prepare_cpu(ring(rule)?)?.0.compose_linear(limits)?;
        let actual = gpu.render(&frame.records, Plan::new(&frame, limits)?)?;
        // Empty tile interiors need carries; global RMS alone would hide a missing hole.
        for (x, y) in [
            (16, 8),
            (31, 16),
            (32, 16),
            (64, 32),
            (159, 63),
            (160, 64),
            (191, 79),
        ] {
            let a = actual.pixel(x, y).unwrap();
            let b = expected.pixel(x, y).unwrap();
            for channel in 0..4 {
                assert!(
                    (a[channel] - b[channel]).abs() <= 1.0 / 255.0,
                    "{rule:?} ({x}, {y}): {a:?} != {b:?}"
                );
            }
        }
        super::device_coverage_tests::assert_visually_equivalent(
            &actual.to_rgba8_frame(false, limits)?,
            &expected.to_rgba8_frame(false, limits)?,
            VisualTolerance::COVERAGE,
        );
    }
    Ok(())
}

#[test]
#[ignore = "requires native GPU; sloping edge bands, subpixel contours and clipped origins"]
fn sloping_tile_edges_match_cpu_with_explicit_tolerance() -> motion_core::Result {
    let limits = RasterLimits::default();
    let mut gpu = Gpu::new()?;
    for phase in [0.0, 0.25, 0.5, 0.75] {
        let path = Path2D::builder()
            .move_to(Point::new(-30.0, -20.0 + phase))
            .line_to(Point::new(192.0, 32.0 + phase))
            .line_to(Point::new(160.0, 80.0 + phase))
            .line_to(Point::new(16.0, 16.0 + phase))
            .close()
            .build()?;
        let node = Node::from(path.fill(Color::WHITE));
        let frame = prepare(node.clone())?;
        assert_eq!(frame.geometry.device_fills.len(), 1);
        let expected = prepare_cpu(node)?
            .0
            .compose_linear(limits)?
            .to_rgba8_frame(false, limits)?;
        let actual = gpu
            .render(&frame.records, Plan::new(&frame, limits)?)?
            .to_rgba8_frame(false, limits)?;
        super::device_coverage_tests::assert_visually_equivalent(
            &actual,
            &expected,
            VisualTolerance::COVERAGE,
        );
    }
    Ok(())
}
