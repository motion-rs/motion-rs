//! GPU-computed fill coverage against the CPU reference.

use super::*;
use crate::gpu::visual::VisualTolerance;
use motion_core::{Color, FillRule, Node, Path2D, Point, Shape};
use wgpu::util::DeviceExt;

fn star(center: Point, radius: f64, rule: FillRule, color: Color) -> motion_core::Result<Node> {
    let mut path = Path2D::new();
    for index in 0..5 {
        let angle = f64::from(index) * 4.0 * std::f64::consts::PI / 5.0 - 1.5;
        let point = Point::new(
            center.x + radius * angle.cos(),
            center.y + radius * angle.sin(),
        );
        if index == 0 {
            path.try_move_to(point)?;
        } else {
            path.try_line_to(point)?;
        }
    }
    path.try_close()?;
    Ok(Node::from(path.fill(color).fill_rule(rule)))
}

fn blob(color: Color) -> motion_core::Result<Node> {
    let mut path = Path2D::new();
    path.try_move_to(Point::new(120.0, 20.3))?;
    path.try_cubic_to(
        Point::new(190.0, 10.0),
        Point::new(200.0, 80.0),
        Point::new(150.5, 90.7),
    )?;
    path.try_cubic_to(
        Point::new(110.0, 120.0),
        Point::new(90.0, 60.0),
        Point::new(120.0, 20.3),
    )?;
    path.try_close()?;
    Ok(Node::from(path.fill(color)))
}

fn sliver(color: Color) -> motion_core::Result<Node> {
    let mut path = Path2D::new();
    path.try_move_to(Point::new(10.2, 140.1))?;
    path.try_line_to(Point::new(210.7, 150.4))?;
    path.try_line_to(Point::new(210.7, 150.9))?;
    path.try_line_to(Point::new(10.2, 140.6))?;
    path.try_close()?;
    Ok(Node::from(path.fill(color)))
}

fn scene(crossing: bool) -> motion_core::Result<Node> {
    let stars = if crossing {
        vec![
            star(
                Point::new(50.0, 55.0),
                40.0,
                FillRule::NonZero,
                Color::rgb(230, 90, 60),
            )?,
            star(
                Point::new(50.0, 55.0),
                30.0,
                FillRule::EvenOdd,
                Color::rgb(30, 80, 200).with_alpha(0.7),
            )?,
        ]
    } else {
        Vec::new()
    };
    Ok(Node::group(
        stars.into_iter().chain([
            blob(Color::rgb(60, 200, 120))?,
            sliver(Color::rgb(250, 240, 40))?,
            Node::from(
                motion_core::Circle::new(Point::new(200.0, 40.0), 24.0)?
                    .fill(Color::rgb(180, 60, 220).with_alpha(0.6)),
            ),
        ]),
    ))
}

fn prepare(crossing: bool) -> motion_core::Result<PreparedGpuFrame> {
    prepare_node(scene(crossing)?, PixelSize::new(240, 160))
}

/// CPU reference preparation of the same scene.
fn prepare_cpu(crossing: bool) -> motion_core::Result<crate::PreparedCpuFrame> {
    let size = PixelSize::new(240, 160);
    let validation = ValidationLimits::default();
    let list = scene(crossing)?.compile(motion_core::Size::new(240.0, 160.0), size, validation)?;
    Ok(list.prepare(validation, RasterLimits::default())?)
}

fn prepare_node(node: Node, size: PixelSize) -> motion_core::Result<PreparedGpuFrame> {
    let validation = ValidationLimits::default();
    let list = node.compile(
        motion_core::Size::new(f64::from(size.width), f64::from(size.height)),
        size,
        validation,
    )?;
    Ok(crate::gpu::raster::prepare::prepare_validated_device(
        &list,
        validation,
        RasterLimits::default(),
        1.0 / 256.0,
        None,
    )?)
}

#[test]
fn coverage_shader_constants_match_the_host_planner() {
    use crate::gpu::raster::device::{COVERAGE_CARRY_STRIDE, COVERAGE_TILE_SIZE};
    let source = include_str!("../../shaders/coverage.wgsl");
    let constant = |name: &str| -> u64 {
        let prefix = format!("const {name} = ");
        let line = source
            .lines()
            .find_map(|line| line.strip_prefix(prefix.as_str()))
            .unwrap_or_else(|| panic!("{name} missing"));
        line.trim_end_matches("u;").parse().unwrap()
    };
    assert_eq!(constant("TILE_SIZE"), u64::from(COVERAGE_TILE_SIZE));
    assert_eq!(constant("CARRY_STRIDE"), COVERAGE_CARRY_STRIDE);
}

#[test]
fn coverage_shader_validates_without_a_device() {
    let shader =
        wgpu::naga::front::wgsl::parse_str(include_str!("../../shaders/coverage.wgsl")).unwrap();
    wgpu::naga::valid::Validator::new(
        wgpu::naga::valid::ValidationFlags::all(),
        wgpu::naga::valid::Capabilities::all(),
    )
    .validate(&shader)
    .unwrap();
}

#[test]
#[ignore = "requires native GPU; exact segmented tile prefix readback"]
fn segmented_tile_prefix_matches_serial_reference() -> motion_core::Result {
    use crate::gpu::abi::{CoverageFill, CoverageParams, CoverageTile};

    let context = GpuContext::new()?;
    let mut fills = Vec::new();
    let mut tiles = Vec::new();
    let mut expected = Vec::new();
    for (fill_index, length) in [1u32, 63, 64, 65, 4095, 4096, 4097, 1_048_576]
        .into_iter()
        .enumerate()
    {
        let first_tile = tiles.len() as u32;
        let total: u32 = (0..length).map(|tile| tile * 7 % 17).sum();
        let first_index = if fill_index == 7 {
            u32::MAX - total
        } else {
            (fill_index as u32 + 1) * 100_000
        };
        fills.push(CoverageFill {
            first_tile,
            columns: length,
            tile_rows: 1,
            first_index,
            index_capacity: total,
            ..Default::default()
        });
        let mut offset = first_index;
        for tile in 0..length {
            let count = tile * 7 % 17;
            tiles.push(CoverageTile {
                count,
                ..Default::default()
            });
            expected.push(offset);
            offset += count;
        }
    }
    let params = CoverageParams {
        fills: fills.len() as u32,
        tiles: tiles.len() as u32,
        ..Default::default()
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
    let tiles = upload(
        bytemuck::cast_slice(&tiles),
        wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_SRC,
    );
    let unused: Vec<_> = (0..5)
        .map(|_| upload(&[0; 48], wgpu::BufferUsages::STORAGE))
        .collect();
    let buffers = [
        &uniforms, &fills, &unused[0], &tiles, &unused[1], &unused[2], &unused[3], &unused[4],
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
    let bytes = u64::from(params.tiles) * size_of::<CoverageTile>() as u64;
    let readback = context.device.create_buffer(&wgpu::BufferDescriptor {
        label: None,
        size: bytes,
        usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
        mapped_at_creation: false,
    });
    let mut encoder = context.device.create_command_encoder(&Default::default());
    for (index, groups) in [
        (2, params.tiles.div_ceil(64)),
        (3, params.fills),
        (4, params.tiles.div_ceil(64)),
    ] {
        let mut pass = encoder.begin_compute_pass(&Default::default());
        pass.set_pipeline(&pipelines.passes[index]);
        pass.set_bind_group(0, &bindings, &[]);
        let width = groups.min(7);
        pass.dispatch_workgroups(width, groups.div_ceil(width), 1);
    }
    encoder.copy_buffer_to_buffer(&tiles, 0, &readback, 0, bytes);
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
    let actual = bytemuck::cast_slice::<u8, CoverageTile>(&mapped);
    for (index, (tile, expected)) in actual.iter().zip(expected).enumerate() {
        assert_eq!(tile.offset, expected, "tile {index}");
    }
    drop(mapped);
    readback.unmap();
    context.check_device()?;
    Ok(())
}

#[test]
fn empty_device_coverage_has_no_prefix_dispatch() -> motion_core::Result {
    let frame = prepare_node(Node::group(Vec::<Node>::new()), PixelSize::new(1, 1))?;
    let plan = Plan::new(&frame, RasterLimits::default())?;
    assert!(plan.device.fills.is_empty());
    assert_eq!(plan.device.tiles, 0);
    assert_eq!(plan.device.samples, 0);
    Ok(())
}

#[test]
fn device_fills_replace_cpu_coverage() -> motion_core::Result {
    let device = prepare(true)?;
    assert_eq!(device.geometry.device_fills.len(), 5);
    assert!(device.records.alphas.is_empty());
    assert!(device.has_device_coverage());
    Ok(())
}

#[test]
#[ignore = "requires native GPU; device fill coverage CPU/GPU parity"]
fn device_fill_coverage_matches_cpu_and_replays() -> motion_core::Result {
    assert_matches_cpu(false)?;
    assert_matches_cpu(true)
}

fn assert_matches_cpu(crossing: bool) -> motion_core::Result {
    let limits = RasterLimits::default();
    let device = prepare(crossing)?;
    let expected = prepare_cpu(crossing)?
        .0
        .compose_linear(limits)?
        .to_rgba8_frame(false, limits)?;
    let mut gpu = Gpu::new()?;
    let actual = gpu
        .render(&device.records, Plan::new(&device, limits)?)?
        .to_rgba8_frame(false, limits)?;
    assert_visually_equivalent(&actual, &expected, VisualTolerance::COVERAGE);
    let replay = gpu
        .render(&device.records, Plan::new(&device, limits)?)?
        .to_rgba8_frame(false, limits)?;
    assert_visually_equivalent(&replay, &actual, VisualTolerance::REPLAY);
    Ok(())
}

#[test]
#[ignore = "requires native GPU; duplicate winding coverage against CPU"]
fn duplicate_contour_matches_cpu_visual_overlap_tier() -> motion_core::Result {
    let node = |contours| -> motion_core::Result<Node> {
        let mut path = Path2D::new();
        for _ in 0..contours {
            path.try_move_to(Point::new(4.25, 4.25))?;
            path.try_line_to(Point::new(20.25, 4.25))?;
            path.try_line_to(Point::new(20.25, 20.25))?;
            path.try_line_to(Point::new(4.25, 20.25))?;
            path.try_close()?;
        }
        Ok(Node::from(path.fill(Color::WHITE)))
    };
    let size = PixelSize::new(32, 32);
    let limits = RasterLimits::default();
    let device = prepare_node(node(2)?, size)?;
    assert!(device.has_device_coverage());
    let prepare_cpu = |contours| -> motion_core::Result<_> {
        Ok(node(contours)?
            .compile(
                motion_core::Size::new(32.0, 32.0),
                size,
                ValidationLimits::default(),
            )?
            .prepare(ValidationLimits::default(), limits)?
            .0
            .compose_linear(limits)?
            .to_rgba8_frame(false, limits)?)
    };
    let cpu = prepare_cpu(2)?;
    assert_eq!(prepare_cpu(1)?.pixels(), cpu.pixels());
    let gpu = Gpu::new()?
        .render(&device.records, Plan::new(&device, limits)?)?
        .to_rgba8_frame(false, limits)?;
    eprintln!(
        "duplicate contour corner CPU={:?} GPU={:?}",
        cpu.pixels()[4 * 32 + 4],
        gpu.pixels()[4 * 32 + 4]
    );
    assert_visually_equivalent(&gpu, &cpu, VisualTolerance::OVERLAP);
    Ok(())
}

pub(super) fn assert_visually_equivalent(
    actual: &crate::cpu::output::Rgba8Frame,
    expected: &crate::cpu::output::Rgba8Frame,
    tolerance: VisualTolerance,
) {
    assert_eq!(actual.size(), expected.size());
    let width = actual.size().width as usize;
    crate::gpu::visual::compare_rgba8(actual.pixels(), expected.pixels(), width, tolerance)
        .unwrap_or_else(|error| panic!("{error}"));
}

fn triangle(left: f64) -> motion_core::Result<Node> {
    let path = Path2D::builder()
        .move_to(Point::new(left + 0.1, 0.1))
        .line_to(Point::new(left + 0.7, 0.1))
        .line_to(Point::new(left + 0.1, 0.7))
        .close()
        .build()?;
    Ok(Node::from(path.fill(Color::WHITE)))
}

#[test]
fn device_float_coverage_layout_and_admission() -> motion_core::Result {
    let frame = prepare_node(
        Node::group([triangle(0.0)?, triangle(1.0)?]),
        PixelSize::new(3, 2),
    )?;
    assert_eq!(frame.geometry.device_fills.len(), 2);
    assert_eq!(
        frame.records.strips[frame.geometry.device_fills[0].strip].alpha_offset,
        0
    );
    assert_eq!(
        frame.records.strips[frame.geometry.device_fills[1].strip].alpha_offset,
        1
    );
    let limits = RasterLimits::default();
    let plan = Plan::new(&frame, limits)?;
    assert_eq!(plan.device.samples, 2);
    assert_eq!(plan.device.fills[0].coverage, 0);
    for (index, fill) in frame.geometry.device_fills.iter().enumerate() {
        let strip = 4 + fill.strip * 5;
        assert_eq!(plan.coverage[strip + 4] as usize, index);
        assert_eq!(plan.coverage[strip + 3] >> 31, 1);
    }
    assert!(
        Plan::new(
            &frame,
            RasterLimits {
                max_work: 1,
                ..limits
            }
        )
        .is_err()
    );
    Ok(())
}

#[test]
#[ignore = "requires native GPU; verifies unquantized coverage and mixed CPU/GPU records"]
fn device_float_coverage_survives_composition() -> motion_core::Result {
    let mixed = || -> motion_core::Result<Node> {
        Ok(Node::group([
            triangle(0.0)?,
            triangle(1.0)?,
            Node::from(motion_core::Rect::new(2.0, 0.0, 2.5, 1.0).fill(Color::WHITE)),
        ]))
    };
    let size = PixelSize::new(3, 2);
    let frame = prepare_node(mixed()?, size)?;
    let limits = RasterLimits::default();
    assert_eq!(frame.geometry.device_fills.len(), 2);
    assert!(!frame.records.alphas.is_empty());
    let mut gpu = Gpu::new()?;
    let actual = gpu.render(&frame.records, Plan::new(&frame, limits)?)?;
    for x in 0..2 {
        let pixel = actual.pixel(x, 0).unwrap();
        for value in pixel {
            assert!((value - 0.18).abs() < 1e-5, "{pixel:?}");
            assert!((value * 255.0 - (value * 255.0).round()).abs() > 0.05);
        }
    }
    let validation = ValidationLimits::default();
    let list = mixed()?.compile(motion_core::Size::new(3.0, 2.0), size, validation)?;
    let cpu = list.prepare(validation, limits)?.compose_linear(limits)?;
    assert_eq!(actual.pixel(2, 0), cpu.pixel(2, 0));
    assert_visually_equivalent(
        &actual.to_rgba8_frame(false, limits)?,
        &cpu.to_rgba8_frame(false, limits)?,
        VisualTolerance::COVERAGE,
    );
    Ok(())
}
