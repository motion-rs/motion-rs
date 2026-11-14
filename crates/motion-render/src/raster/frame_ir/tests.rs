//! Normalized geometry, admission and independent backend preparation regressions.

use super::*;
#[cfg(feature = "gpu")]
use crate::gpu::PreparedGpuFrame;
use crate::raster::{PreparationCache, prepare::prepare_ir};
use motion_core::{
    Color, DisplayList, DrawCommand, Filter, Node, NodeModifiers, Point, Shape, Size,
};
use std::sync::Arc;

fn curved_list(transform: Transform) -> Result<ValidatedDisplayList, ValidationError> {
    let geometry = Path2D::builder()
        .move_to(Point::new(0.25, 0.5))
        .quadratic_to(Point::new(4.5, 12.25), Point::new(8.75, 0.5))
        .close()
        .build()?;
    DisplayList {
        output_size: PixelSize::new(32, 24),
        background: None,
        commands: vec![DrawCommand::Fill {
            path: Arc::new(geometry),
            transform,
            paint: Color::WHITE.into(),
            rule: FillRule::EvenOdd,
        }],
    }
    .validate(ValidationLimits::default())
}

fn build(source: &ValidatedDisplayList) -> Result<FrameIr<'_>, ValidationError> {
    FrameIr::build(
        source,
        ValidationLimits::default(),
        RasterLimits::default(),
        1.0 / 256.0,
    )
}

#[test]
fn ir_preserves_device_f64_curves_and_reference_recipe() -> Result<(), ValidationError> {
    let transform =
        Transform::scale_non_uniform(1.25, 0.75).then(Transform::translate(3.125, 2.0625));
    let source = curved_list(transform)?;
    let ir = build(&source)?;
    assert_eq!(ir.paths.len(), 1);
    assert_eq!(ir.paints.len(), 1);
    assert!(matches!(
        ir.items.as_slice(),
        [IrItem::Fill { path: 0, paint: 0 }]
    ));
    let geometry = ir.path(0);
    assert_eq!(geometry.rule, FillRule::EvenOdd);
    assert_eq!(geometry.transform, transform);
    assert_eq!(
        geometry.device_verbs[0],
        PathVerb::MoveTo(transform.apply(Point::new(0.25, 0.5)))
    );
    assert_eq!(
        geometry.device_verbs[1],
        PathVerb::QuadraticTo(
            transform.apply(Point::new(4.5, 12.25)),
            transform.apply(Point::new(8.75, 0.5)),
        )
    );
    assert_eq!(geometry.device_verbs.len(), 3);
    assert!(geometry.segment_upper_bound > geometry.device_verbs.len() as u64);
    assert_eq!(ir.segments, geometry.segment_upper_bound);
    assert!(
        ir.stats().retained_bytes
            >= std::mem::size_of_val(&ir) as u64
                + std::mem::size_of_val(geometry.device_verbs.as_slice()) as u64
    );
    Ok(())
}

#[test]
fn ir_retains_analytic_rectangle_and_normalizes_clip_filter_scopes() -> Result<(), ValidationError>
{
    let source = Node::from(Rect::new(1.0, 1.0, 9.0, 7.0).fill(Color::WHITE))
        .clip_with(
            Rect::new(2.25, 0.5, 8.5, 7.5),
            motion_core::ClipMode::Geometric,
        )
        .filter(Filter::Blur { sigma: 1.0 })
        .compile(
            Size::new(12.0, 10.0),
            PixelSize::new(12, 10),
            ValidationLimits::default(),
        )?;
    let ir = build(&source)?;
    assert!(ir.output_bounds.left > 0);
    assert_eq!(ir.output_bounds.width(), 12);
    assert_eq!(ir.output_bounds.height(), 10);
    assert!(ir.filter_halo() > 0);
    assert!(
        ir.paths
            .iter()
            .any(|path| matches!(path.source, GeometrySource::Rectangle(_)))
    );
    assert!(
        ir.clips
            .iter()
            .any(|clip| matches!(clip, IrClip::Rectangle { .. } | IrClip::Shape { .. }))
    );
    let filters = ir
        .items
        .iter()
        .find_map(|item| match item {
            IrItem::PushLayer { layer, .. } => layer.filters.as_ref(),
            _ => None,
        })
        .unwrap();
    assert_eq!(filters.operations, [Filter::Blur { sigma: 1.0 }]);
    assert!(filters.source_bounds.left >= f64::from(ir.output_bounds.left));
    let cpu = prepare_ir(&ir, RasterLimits::default(), None)?;
    assert_eq!(cpu.filter_halo, ir.filter_halo());
    assert_eq!(cpu.size(), PixelSize::new(12, 10));
    Ok(())
}

#[test]
fn ir_admits_storage_work_wang_bound_and_padded_pixels_before_backend_work()
-> Result<(), ValidationError> {
    let source = curved_list(Transform::IDENTITY)?;
    let ir = build(&source)?;
    for limits in [
        RasterLimits {
            max_bytes: ir.stats().retained_bytes - 1,
            ..RasterLimits::default()
        },
        RasterLimits {
            max_work: ir.stats().work - 1,
            ..RasterLimits::default()
        },
        RasterLimits {
            max_pixels: 32 * 24 - 1,
            ..RasterLimits::default()
        },
    ] {
        assert!(FrameIr::build(&source, ValidationLimits::default(), limits, 1.0 / 256.0).is_err());
    }
    let limited = RasterLimits {
        max_work: 100,
        ..RasterLimits::default()
    };
    // Flattening work is charged by the backend that flattens, not reserved by the IR.
    let fine = FrameIr::build(&source, ValidationLimits::default(), limited, 1.0e-12)?;
    assert!(prepare_ir(&fine, limited, None).is_err());
    assert!(
        FrameIr::build(
            &source,
            ValidationLimits::default(),
            RasterLimits::default(),
            0.0
        )
        .is_err()
    );
    Ok(())
}

#[test]
fn ir_rechecks_hidden_content_against_requested_structural_limits() -> Result<(), ValidationError> {
    let source = Node::group([
        Node::from(Rect::new(0.0, 0.0, 4.0, 4.0).fill(Color::WHITE)),
        Node::from(Rect::new(100.0, 100.0, 104.0, 104.0).fill(Color::WHITE)),
    ])
    .opacity(0.0)
    .compile(
        Size::new(8.0, 8.0),
        PixelSize::new(8, 8),
        ValidationLimits::default(),
    )?;
    let limits = ValidationLimits {
        max_nodes: 1,
        ..ValidationLimits::default()
    };
    assert!(FrameIr::build(&source, limits, RasterLimits::default(), 1.0 / 256.0).is_err());
    Ok(())
}

#[test]
fn zero_opacity_layers_skip_nested_fill_coverage_and_preserve_siblings() -> motion_core::Result {
    let geometry = Path2D::builder()
        .move_to(Point::new(1.0, 1.0))
        .quadratic_to(Point::new(4.0, 7.0), Point::new(7.0, 1.0))
        .close()
        .build()?;
    let visible = Node::from(geometry.fill(Color::WHITE));
    let hidden = Node::group([
        Node::group((0..128).map(|_| visible.clone()))
            .clip_with(
                Rect::new(0.0, 0.0, 8.0, 8.0),
                motion_core::ClipMode::Geometric,
            )
            .filter(Filter::Blur { sigma: 0.5 })
            .mask(visible.clone(), motion_core::MaskMode::Alpha),
        visible.clone().opacity(0.0),
    ])
    .opacity(0.0);
    let source = Node::group([hidden, visible.clone()]).compile(
        Size::new(8.0, 8.0),
        PixelSize::new(8, 8),
        ValidationLimits::default(),
    )?;
    let reference = visible.compile(
        Size::new(8.0, 8.0),
        PixelSize::new(8, 8),
        ValidationLimits::default(),
    )?;
    let ir = build(&source)?;
    let limits = RasterLimits::default();
    let cpu = prepare_ir(&ir, limits, None)?;
    let expected = prepare_ir(&build(&reference)?, limits, None)?;
    let fills = cpu
        .items
        .iter()
        .filter_map(|item| match item {
            crate::prepared::PreparedItem::Fill { strips, spans, .. } => Some((strips, spans)),
            _ => None,
        })
        .collect::<Vec<_>>();
    assert!(
        fills[..fills.len() - 1]
            .iter()
            .all(|(strips, spans)| strips.is_empty() && spans.is_empty())
    );
    let (strips, spans) = fills.last().unwrap();
    assert!(!strips.is_empty() || !spans.is_empty());
    assert_eq!(
        cpu.render_linear(limits)?.bytes(),
        expected.render_linear(limits)?.bytes()
    );
    #[cfg(feature = "gpu")]
    {
        let gpu = PreparedGpuFrame::prepare(&ir, limits, None)?;
        assert_eq!(gpu.geometry.device_fills.len(), 1);
    }
    Ok(())
}

#[test]
fn zero_opacity_mask_content_preserves_inverted_mask_semantics() -> motion_core::Result {
    let visible = Node::from(Rect::new(0.0, 0.0, 4.0, 4.0).fill(Color::WHITE));
    for (mode, expected) in [
        (motion_core::MaskMode::Alpha, [0.0; 4]),
        (motion_core::MaskMode::Luminance, [0.0; 4]),
        (motion_core::MaskMode::InvertedAlpha, [1.0; 4]),
        (motion_core::MaskMode::InvertedLuminance, [1.0; 4]),
    ] {
        let source = visible
            .clone()
            .mask(visible.clone().opacity(0.0), mode)
            .compile(
                Size::new(4.0, 4.0),
                PixelSize::new(4, 4),
                ValidationLimits::default(),
            )?;
        let limits = RasterLimits::default();
        let cpu = prepare_ir(&build(&source)?, limits, None)?;
        assert_eq!(
            cpu.render_linear(limits)?.linear_pixel(1, 1),
            Some(expected)
        );
    }
    Ok(())
}

#[test]
#[cfg(feature = "gpu")]
fn cpu_reference_numeric_coverage_is_unchanged() -> Result<(), ValidationError> {
    let geometry = Path2D::builder()
        .move_to(Point::ORIGIN)
        .quadratic_to(Point::new(0.5, 2.0), Point::new(1.0, 0.0))
        .build()?;
    let source = Node::from(geometry.fill(Color::WHITE)).compile(
        Size::new(1.0, 1.0),
        PixelSize::new(1, 1),
        ValidationLimits::default(),
    )?;
    let limits = RasterLimits::default();
    let ir = FrameIr::build(&source, ValidationLimits::default(), limits, 1.0 / 1024.0)?;
    let cpu = prepare_ir(&ir, limits, None)?;
    assert_eq!(
        cpu.render_linear(limits)?.linear_pixel(0, 0),
        Some([170.0 / 255.0; 4])
    );
    let gpu = PreparedGpuFrame::prepare(&ir, limits, None)?;
    assert!(gpu.has_device_coverage());
    assert!(gpu.records.alphas.is_empty());
    // GPU preparation cannot mutate the IR or the independently owned CPU result.
    assert_eq!(
        cpu.render_linear(limits)?.linear_pixel(0, 0),
        Some([170.0 / 255.0; 4])
    );
    Ok(())
}

#[test]
#[cfg(feature = "gpu")]
fn warm_cpu_cache_does_not_make_gpu_preparation_depend_on_cpu_coverage()
-> Result<(), ValidationError> {
    let source = curved_list(Transform::translate(2.125, 1.0625))?;
    let ir = build(&source)?;
    let limits = RasterLimits::default();
    let mut cache = PreparationCache::default();
    let cold = prepare_ir(&ir, limits, Some(&mut cache))?;
    let gpu = PreparedGpuFrame::prepare(&ir, limits, Some(&mut cache))?;
    assert_eq!(gpu.geometry.device_fills.len(), 1);
    assert!(gpu.records.alphas.is_empty());
    assert!(gpu.records.spans.is_empty());
    let warm = prepare_ir(&ir, limits, Some(&mut cache))?;
    assert_eq!(warm.strips, cold.strips);
    assert_eq!(warm.spans, cold.spans);
    assert_eq!(warm.alphas, cold.alphas);
    assert_eq!(
        warm.render_linear(limits)?.bytes(),
        cold.render_linear(limits)?.bytes()
    );
    Ok(())
}

#[test]
fn repeated_pattern_fills_charge_shared_image_bytes_once() -> motion_core::Result {
    let image = Arc::new(motion_content::ImageAsset::from_rgba8(
        64,
        64,
        vec![200; 64 * 64 * 4],
    )?);
    let retained = |fills: usize| -> Result<u64, motion_core::Error> {
        let nodes = (0..fills).map(|_| {
            Node::from(
                Rect::new(0.0, 0.0, 8.0, 8.0)
                    .fill(motion_core::ImagePattern::from_asset(image.clone())),
            )
        });
        let source = Node::group(nodes).compile(
            Size::new(8.0, 8.0),
            PixelSize::new(8, 8),
            ValidationLimits::default(),
        )?;
        let ir = build(&source)?;
        Ok(ir.stats().retained_bytes)
    };
    let one = retained(1)?;
    let eight = retained(8)?;
    assert!(eight - one < 64 * 64 * 8, "{one} -> {eight}");
    Ok(())
}
