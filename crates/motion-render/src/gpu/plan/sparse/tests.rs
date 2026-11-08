//! Direct, sparse and conditional coverage encoding tests against CPU coverage.

use super::*;
use motion_core::Shape;

#[test]
fn direct_descriptors_preserve_ranges_bounds_and_alpha_bits() -> motion_core::Result {
    let limits = RasterLimits::default();
    let mut frame = motion_core::Node::from(
        motion_core::Rect::new(0.25, 0.5, 31.5, 63.5).fill(motion_core::Color::WHITE),
    )
    .compile(
        motion_core::Size::new(32.0, 64.0),
        PixelSize::new(32, 64),
        motion_core::ValidationLimits::default(),
    )?
    .prepare(motion_core::ValidationLimits::default(), limits)?;
    for (index, alpha) in frame.alphas.iter_mut().enumerate() {
        *alpha = (index % 256) as u8;
    }
    let (strips, spans) = frame
        .items
        .iter()
        .find_map(|item| {
            if let PreparedItem::Fill { strips, spans, .. } = item {
                Some((strips.clone(), spans.clone()))
            } else {
                None
            }
        })
        .unwrap();
    let mut planner = Planner {
        frame: &frame,
        steps: Vec::new(),
        budget: Budget::new(frame.size, limits)?,
    };
    let reference = planner.coverage(strips.clone(), spans.clone(), 0.0)?;
    let shared = planner.shared_coverage()?;
    assert_eq!(shared[0] as usize, frame.strips.len());
    assert_eq!(shared[1] as usize, frame.spans.len());
    assert_eq!(shared[2] as usize, frame.alphas.len());
    let decoded: Vec<u8> = shared[shared[3] as usize..]
        .iter()
        .flat_map(|word| word.to_le_bytes())
        .take(frame.alphas.len())
        .collect();
    assert_eq!(decoded, frame.alphas);
    assert_eq!(
        shared.len(),
        4 + 6 * frame.strips.len() + 4 * frame.spans.len() + frame.alphas.len().div_ceil(4)
    );
    for clip in [
        PixelRect {
            left: 0,
            top: 0,
            right: 32,
            bottom: 64,
        },
        PixelRect {
            left: 3,
            top: 5,
            right: 31,
            bottom: 61,
        },
        PixelRect {
            left: 32,
            top: 64,
            right: 32,
            bottom: 64,
        },
    ] {
        let data = planner.direct_coverage(strips.clone(), spans.clone(), clip)?;
        assert_eq!(data.len(), 10);
        assert_eq!(data[6].to_bits() as usize, strips.start);
        assert_eq!(data[7].to_bits() as usize, strips.end);
        assert_eq!(data[8].to_bits() as usize, spans.start);
        assert_eq!(data[9].to_bits() as usize, spans.end);
        for row in 0..64 {
            for column in 0..32 {
                let expected = if column >= clip.left
                    && column < clip.right
                    && row >= clip.top
                    && row < clip.bottom
                {
                    planner.coverage_at(&reference, column, row)
                } else {
                    0.0
                };
                assert_eq!(
                    planner.coverage_at(&data, column, row).to_bits(),
                    expected.to_bits()
                );
            }
        }
    }
    Ok(())
}

#[test]
fn sparse_records_preserve_original_u8_coverage_after_clipping() -> motion_core::Result {
    let mut path = motion_core::Path2D::new();
    for left in [1.25, 120.25] {
        path.move_to(motion_core::Point::new(left, 0.5))
            .line_to(motion_core::Point::new(left + 1.0, 0.5))
            .line_to(motion_core::Point::new(left + 1.0, 63.5))
            .line_to(motion_core::Point::new(left, 63.5))
            .close();
    }
    let limits = RasterLimits::default();
    let frame = motion_core::Node::from(path.fill(motion_core::Color::WHITE))
        .compile(
            motion_core::Size::new(128.0, 64.0),
            PixelSize::new(128, 64),
            motion_core::ValidationLimits::default(),
        )?
        .prepare(motion_core::ValidationLimits::default(), limits)?;
    let (strips, spans) = frame
        .items
        .iter()
        .find_map(|item| {
            if let PreparedItem::Fill { strips, spans, .. } = item {
                Some((strips.clone(), spans.clone()))
            } else {
                None
            }
        })
        .unwrap();
    let clip = PixelRect {
        left: 0,
        top: 3,
        right: 127,
        bottom: 63,
    };
    let mut planner = Planner {
        frame: &frame,
        steps: Vec::new(),
        budget: Budget::new(frame.size, limits)?,
    };
    let data = planner
        .sparse_coverage(strips.clone(), spans.clone(), clip)?
        .expect("sparse upload");
    assert_eq!(data[0].to_bits(), SPARSE_COVERAGE);
    assert_eq!(data[1].to_bits() as usize, data.len());
    assert!((data.len() as u64) < 4 + planner.coverage_bounds(&data).pixels().div_ceil(4));
    let mut expected = vec![0_u8; 128 * 64];
    for strip in &frame.strips[strips] {
        for row in 0..strip.height {
            for column in 0..strip.width {
                expected[((strip.y + row) * 128 + strip.x + column) as usize] =
                    frame.alphas[strip.alpha_offset + (row * strip.width + column) as usize];
            }
        }
    }
    for span in &frame.spans[spans] {
        for row in span.y..span.y + span.height {
            for column in span.start_x..span.end_x {
                expected[(row * 128 + column) as usize] = 255;
            }
        }
    }
    for row in 0..64 {
        for column in 0..128 {
            let reference = if column < clip.right && row >= clip.top && row < clip.bottom {
                f32::from(expected[(row * 128 + column) as usize]) / 255.0
            } else {
                0.0
            };
            assert_eq!(
                planner.coverage_at(&data, column, row).to_bits(),
                reference.to_bits(),
                "coverage at ({column}, {row})"
            );
        }
    }
    Ok(())
}

#[test]
fn conditional_packets_preserve_mixed_payloads_and_zero_denominators() -> motion_core::Result {
    let limits = RasterLimits::default();
    let frame = motion_core::Node::empty()
        .compile(
            motion_core::Size::new(4.0, 1.0),
            PixelSize::new(4, 1),
            motion_core::ValidationLimits::default(),
        )?
        .prepare(motion_core::ValidationLimits::default(), limits)?;
    let mut planner = Planner {
        frame: &frame,
        steps: Vec::new(),
        budget: Budget::new(frame.size, limits)?,
    };
    let words = |words: &[u32]| {
        words
            .iter()
            .copied()
            .map(f32::from_bits)
            .collect::<Vec<_>>()
    };
    let mut dense = words(&[0, 0, 4, 1]);
    dense.extend([0.0, 1.0 / 255.0, 128.0 / 255.0, 1.0]);
    let payloads = [
        vec![0.0],
        vec![1.0],
        words(&[1 << 31, 0, 4, 1, 0.5_f32.to_bits()]),
        words(&[0, 1 << 31, 4, 1, u32::from_le_bytes([0, 1, 128, 255])]),
        dense,
        words(&[
            SPARSE_COVERAGE,
            16,
            0,
            0,
            4,
            1,
            1,
            0,
            0,
            4,
            1,
            15,
            0,
            0,
            4,
            u32::from_le_bytes([0, 1, 128, 255]),
        ]),
    ];
    for alpha in &payloads {
        for child in &payloads {
            for ancestor in &payloads {
                let expected: [f32; 4] = std::array::from_fn(|column| {
                    let numerator = planner.coverage_at(child, column as u32, 0);
                    let denominator = planner.coverage_at(ancestor, column as u32, 0);
                    planner.coverage_at(alpha, column as u32, 0)
                        * if denominator == 0.0 {
                            0.0
                        } else {
                            (numerator / denominator).min(1.0)
                        }
                });
                let previous = planner.budget.live_bytes();
                planner.budget.bytes(
                    (alpha.len() + child.len() + ancestor.len()) as u64 * 4,
                    "test.clips",
                )?;
                let data =
                    planner.conditional_clip(alpha.clone(), child.clone(), ancestor.clone())?;
                assert_eq!(data[0].to_bits(), CONDITIONAL_COVERAGE);
                assert_eq!(data[1].to_bits() as usize, data.len());
                assert_eq!(data.len(), 8 + alpha.len() + child.len() + ancestor.len());
                assert_eq!(
                    planner.budget.live_bytes(),
                    previous + data.len() as u64 * 4
                );
                for (column, expected) in expected.iter().enumerate() {
                    assert_eq!(
                        planner.coverage_at(&data, column as u32, 0).to_bits(),
                        expected.to_bits(),
                    );
                }
                assert_eq!(planner.coverage_at(&data, 4, 0), 0.0);
                assert_eq!(planner.coverage_at(&data, 0, 1), 0.0);
                planner.budget.release(data.len() as u64 * 4);
            }
        }
    }
    Ok(())
}

#[test]
#[ignore = "requires native GPU; validates sparse nested coverage/geometric clips and exact replay"]
fn native_sparse_nested_clips_preserve_cpu_tolerance_and_replay() -> motion_core::Result {
    use motion_core::NodeModifiers;
    let limits = RasterLimits::default();
    let context = GpuContext::new()?;
    let mut gpu = Gpu::from_context(context);
    let mut sparse = motion_core::Path2D::new();
    for left in [1.25, 120.25] {
        sparse
            .move_to(motion_core::Point::new(left, 0.5))
            .line_to(motion_core::Point::new(left + 1.0, 0.5))
            .line_to(motion_core::Point::new(left + 1.0, 7.5))
            .line_to(motion_core::Point::new(left, 7.5))
            .close();
    }
    let ancestor = motion_core::Path2D::from_rect(motion_core::Rect::new(0.5, 1.25, 127.0, 61.5))?;
    for mode in [
        motion_core::ClipMode::Coverage,
        motion_core::ClipMode::Geometric,
    ] {
        for filtered in [false, true] {
            let mut child = motion_core::Node::from(
                motion_core::Rect::new(0.25, 0.5, 127.0, 63.0)
                    .fill(motion_core::Color::rgb(190, 40, 125)),
            )
            .opacity(0.7)
            .clip_with(sparse.clone(), mode);
            if filtered {
                child = child.filter(motion_core::Filter::Blur { sigma: 1.0 });
            }
            let node = motion_core::Node::group([child])
                .clip_with(ancestor.clone(), motion_core::ClipMode::Geometric)
                .clip(sparse.clone());
            let frame = node
                .compile(
                    motion_core::Size::new(128.0, 64.0),
                    PixelSize::new(128, 64),
                    motion_core::ValidationLimits::default(),
                )?
                .prepare(motion_core::ValidationLimits::default(), limits)?;
            let plan = Plan::new(&frame, limits)?;
            assert!(plan.steps.iter().any(|step| {
                step.data
                    .first()
                    .is_some_and(|word| word.to_bits() == CONDITIONAL_COVERAGE)
                    && Planner::uses_direct_coverage(&step.data)
            }));
            assert_eq!(plan.coverage[0] as usize, frame.strips.len());
            assert_eq!(plan.coverage[1] as usize, frame.spans.len());
            assert_eq!(plan.coverage[2] as usize, frame.alphas.len());
            let uploaded: Vec<u8> = plan.coverage[plan.coverage[3] as usize..]
                .iter()
                .flat_map(|word| word.to_le_bytes())
                .take(frame.alphas.len())
                .collect();
            assert_eq!(uploaded, frame.alphas);
            let expected = frame.render_linear(limits)?.to_srgb_rgba8(false, limits)?;
            let actual = gpu.render(&frame, plan)?.to_srgb_rgba8(false, limits)?;
            let replay = gpu
                .render(&frame, Plan::new(&frame, limits)?)?
                .to_srgb_rgba8(false, limits)?;
            assert_eq!(actual.pixels(), replay.pixels());
            let mut over_one = 0;
            for (actual, expected) in actual.pixels().iter().zip(expected.pixels()) {
                let difference = actual
                    .iter()
                    .zip(expected)
                    .map(|(actual, expected)| actual.abs_diff(*expected))
                    .max()
                    .unwrap();
                assert!(difference <= 2);
                over_one += usize::from(difference > 1);
            }
            assert!(over_one * 1000 < actual.pixels().len());
        }
    }
    Ok(())
}

#[test]
fn clips_with_many_records_use_dense_coverage_and_few_records_stay_direct() -> motion_core::Result {
    use motion_core::NodeModifiers;
    let limits = RasterLimits::default();
    let plan = |height: f64| -> motion_core::Result<Plan> {
        let mut path = motion_core::Path2D::new();
        for left in [1.25, 120.25] {
            path.move_to(motion_core::Point::new(left, 0.5))
                .line_to(motion_core::Point::new(left + 1.0, 0.5))
                .line_to(motion_core::Point::new(left + 1.0, height))
                .line_to(motion_core::Point::new(left, height))
                .close();
        }
        let node = motion_core::Node::group([motion_core::Node::from(
            motion_core::Rect::new(0.0, 0.0, 128.0, 64.0).fill(motion_core::Color::WHITE),
        )
        .opacity(0.5)])
        .clip(path);
        let frame = node
            .compile(
                motion_core::Size::new(128.0, 64.0),
                PixelSize::new(128, 64),
                motion_core::ValidationLimits::default(),
            )?
            .prepare(motion_core::ValidationLimits::default(), limits)?;
        Ok(Plan::new(&frame, limits)?)
    };
    let composites = |plan: &Plan| {
        plan.steps
            .iter()
            .filter(|step| step.parameters.operation.code == GpuOp::Composite as u32)
            .map(|step| Planner::uses_direct_coverage(&step.data))
            .collect::<Vec<_>>()
    };
    let few = composites(&plan(7.5)?);
    assert!(few.contains(&true), "{few:?}");
    let large = composites(&plan(63.5)?);
    assert!(!large.contains(&true), "{large:?}");
    Ok(())
}

#[test]
fn drawn_fills_are_not_charged_a_per_pixel_record_scan() -> motion_core::Result {
    let limits = RasterLimits::default();
    let fills = (0..400).map(|index| {
        let left = f64::from(index) * 4.0 + 0.5;
        motion_core::Node::from(
            motion_core::Rect::new(left, 0.5, left + 2.25, 1079.5).fill(motion_core::Color::WHITE),
        )
    });
    let frame = motion_core::Node::group(fills)
        .compile(
            motion_core::Size::new(1920.0, 1080.0),
            PixelSize::new(1920, 1080),
            motion_core::ValidationLimits::default(),
        )?
        .prepare(motion_core::ValidationLimits::default(), limits)?;
    assert!(frame.strips.len() > 1000);
    let plan = Plan::new(&frame, limits)?;
    let fills = plan
        .steps
        .iter()
        .filter(|step| step.parameters.operation.code == GpuOp::Fill as u32)
        .count();
    assert_eq!(fills, 400);
    assert!(
        plan.stats.work < 400 * 1920 * 1080 * 4,
        "{}",
        plan.stats.work
    );
    Ok(())
}
