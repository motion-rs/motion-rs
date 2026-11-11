//! Tile interiors, seams, static admission and explicit CPU path diagnostics.

use super::*;
use motion_core::{Color, FillRule, Node, Path2D, Point, Rect, Size};

fn prepare_cpu(node: Node) -> motion_core::Result<crate::PreparedFrame> {
    let size = PixelSize::new(193, 81);
    let validation = ValidationLimits::default();
    let list = node.compile(Size::new(193.0, 81.0), size, validation)?;
    Ok(list.prepare(validation, RasterLimits::default())?)
}

fn prepare(node: Node) -> motion_core::Result<crate::raster::gpu_prepare::GpuFrame> {
    let size = PixelSize::new(193, 81);
    let validation = ValidationLimits::default();
    let list = node.compile(Size::new(193.0, 81.0), size, validation)?;
    Ok(crate::prepared::prepare_validated_device(
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
        stats.tile_count * (16 + 16 * 9 * 4) + stats.tile_index_capacity.max(1) * 4
    );
    assert_eq!(plan.device.fills[0].columns, 12);
    assert_eq!(plan.device.fills[0].tile_rows, 6);
    assert_eq!(plan.device.fills[0].index_capacity, plan.device.indices);
    // Test the tile stage directly: full planning may shrink optional fill batches.
    let admit = |limits| -> motion_core::Result<(u64, u64)> {
        let mut planner = Planner {
            frame: frame.gpu_view(),
            steps: Vec::new(),
            budget: Budget::new(frame.size, limits)?,
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
#[ignore = "requires native GPU; tile interiors, holes, seams and pooled scratch replay"]
fn tile_interiors_fill_rules_and_reused_scratch_match_cpu() -> motion_core::Result {
    let limits = RasterLimits::default();
    let mut gpu = Gpu::new()?;
    for rule in [FillRule::NonZero, FillRule::EvenOdd, FillRule::NonZero] {
        let frame = prepare(ring(rule)?)?;
        assert_eq!(frame.geometry.device_fills.len(), 1);
        let expected = prepare_cpu(ring(rule)?)?.scene.compose_linear(limits)?;
        let actual = gpu.render(&frame, Plan::new(&frame, limits)?)?;
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
            actual.to_rgba8_frame(false, limits)?.pixels(),
            expected.to_rgba8_frame(false, limits)?.pixels(),
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
            .scene
            .compose_linear(limits)?
            .to_rgba8_frame(false, limits)?;
        let actual = gpu
            .render(&frame, Plan::new(&frame, limits)?)?
            .to_rgba8_frame(false, limits)?;
        super::device_coverage_tests::assert_visually_equivalent(
            actual.pixels(),
            expected.pixels(),
        );
    }
    Ok(())
}
