//! Image mip residency across changing draws, hardware/compute routes and disabled caches.

use crate::gpu::{Gpu, Plan};
use crate::{RasterLimits, RenderPrepare};
use motion_content::image::{Image, ImageAsset};
use motion_core::{CacheLimits, Node, NodeModifiers, PixelSize, Size, ValidationLimits};
use std::sync::Arc;

#[test]
#[ignore = "requires a native GPU; verifies image mip uploads across frames and draw routes"]
fn image_mips_stay_resident_when_draw_transforms_change() -> motion_core::Result {
    let mut pixels = Vec::new();
    for row in 0..8 {
        for column in 0..8 {
            pixels.extend([column * 31, row * 31, (row + column) * 15, 192]);
        }
    }
    let asset = Arc::new(ImageAsset::from_rgba8(8, 8, pixels)?);
    assert!(asset.levels().len() > 1);
    let mut gpu = Gpu::new()?;
    let limits = RasterLimits::default();
    gpu.context.clear_resident_resources()?;
    let initial = gpu.context.residency_stats();
    let mut first_upload = 0;
    for (index, (size, x, y)) in [(2.0, 1.5, 2.0), (3.0, 7.0, 6.5)].into_iter().enumerate() {
        let frame = Node::from(Image::from_asset(asset.clone()).size(size, size))
            .translate(x, y)
            .compile(
                Size::new(16.0, 16.0),
                PixelSize::new(16, 16),
                ValidationLimits::default(),
            )?
            .prepare(ValidationLimits::default(), limits)?;
        let reference = frame.compose_linear(limits)?;
        let plan = Plan::new(&frame, limits)?;
        assert_eq!(
            plan.steps
                .iter()
                .filter(|step| step.resource.is_some())
                .count(),
            1
        );
        let actual = gpu.render(&frame, plan)?;
        assert_pixels_close(actual.pixels(), reference.pixels());
        let stats = gpu.context.residency_stats();
        assert_eq!(stats.misses, initial.misses + 1);
        if index == 0 {
            first_upload = stats.upload_bytes;
            assert!(first_upload > initial.upload_bytes);
        } else {
            assert_eq!(stats.upload_bytes, first_upload);
            assert_eq!(stats.hits, initial.hits + 2);
        }
        let mut compute_context = gpu.context.clone();
        Arc::make_mut(&mut compute_context.textures).blends = None;
        let mut compute = Gpu::from_context(compute_context);
        let compute_pixels = compute.render(&frame, Plan::new(&frame, limits)?)?;
        assert_pixels_close(compute_pixels.pixels(), actual.pixels());
        assert_eq!(gpu.context.residency_stats().upload_bytes, first_upload);
        if index == 1 {
            let before_uncached = gpu.context.residency_stats();
            gpu.context.set_residency_limits(CacheLimits {
                max_entries: 0,
                max_bytes: 0,
            })?;
            let uncached = gpu.render(&frame, Plan::new(&frame, limits)?)?;
            assert_eq!(uncached.pixels(), actual.pixels());
            assert_eq!(
                gpu.context.residency_stats().misses,
                before_uncached.misses + 1
            );
        }
    }
    Ok(())
}

fn assert_pixels_close(actual: &[[f32; 4]], reference: &[[f32; 4]]) {
    assert_eq!(actual.len(), reference.len());
    assert!(actual.iter().any(|pixel| pixel[3] > 0.1));
    for (actual, reference) in actual.iter().zip(reference) {
        for (actual, reference) in actual.iter().zip(reference) {
            assert!(
                (actual - reference).abs() <= 3e-4,
                "{actual} != {reference}"
            );
        }
    }
}
