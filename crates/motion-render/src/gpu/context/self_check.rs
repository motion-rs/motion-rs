//! Startup visual fixtures that prepare CPU and GPU coverage independently.

use crate::gpu::{Gpu, GpuContext, Plan, PreparedGpuFrame, visual};
use crate::{RasterLimits, RenderPrepare};
use motion_core::{
    BlendMode, Circle, ClipMode, Color, FillRule, Filter, LinearGradient, Node, NodeModifiers,
    Path2D, PixelSize, Point, Rect, Shape, Size, ValidationError, ValidationLimits,
};

/// RMS allowance for spatial paint fixtures.
const PAINT_VISUAL_RMS: f64 = 0.005;

/// Small self-intersecting contours permit localized differences in antialiasing area.
const GEOMETRY_VISUAL_RMS: f64 = 0.02;

/// RMS allowance after filtering the clipped geometry fixture.
const FILTER_VISUAL_RMS: f64 = 0.01;

impl GpuContext {
    /// Checks independently prepared CPU/GPU geometry, alpha, paints, clipping and filters.
    ///
    /// Comparisons allow visual error rather than requiring identical bytes. The result is
    /// shared and sticky for this context; device loss remains an error even after success.
    pub fn self_check(&self) -> motion_core::Result<()> {
        self.loss.check()?;
        let result = self
            .self_check_result
            .get_or_init(|| run(self).map_err(|error| error.to_string()));
        result.clone().map_err(|message| {
            motion_core::Error::GpuUnavailable(Box::new(std::io::Error::other(message)))
        })
    }
}

fn run(context: &GpuContext) -> motion_core::Result<()> {
    let limits = RasterLimits::default();
    let validation = ValidationLimits::default();
    let mut gpu = Gpu::from_context(context.clone());
    for (label, node, requires_device_coverage, tolerance) in fixtures()? {
        let list = node.compile(Size::new(64.0, 64.0), PixelSize::new(64, 64), validation)?;
        let reference = list.prepare(validation, limits)?;
        let device = crate::gpu::raster::prepare::prepare_validated_device(
            &list,
            validation,
            limits,
            1.0 / 256.0,
            None,
        )?;
        if requires_device_coverage && !device.has_device_coverage() {
            return Err(ValidationError::invalid(
                "gpu.self_check",
                "coverage",
                format!("{label} did not exercise GPU coverage"),
            )
            .into());
        }
        let expected =
            crate::FrameBuffer::from_linear_rgba8(&reference.compose_linear(limits)?, limits)?;
        let actual = render(&mut gpu, &device, limits)?;
        visual::compare_rgba8(
            actual.bytes().as_chunks::<4>().0,
            expected.bytes().as_chunks::<4>().0,
            tolerance,
        )
        .map_err(|error| ValidationError::invalid("gpu.self_check", label, error.to_string()))?;
        let replay = render(&mut gpu, &device, limits)?;
        visual::compare_rgba8(
            replay.bytes().as_chunks::<4>().0,
            actual.bytes().as_chunks::<4>().0,
            tolerance,
        )
        .map_err(|error| ValidationError::invalid("gpu.self_check", label, error.to_string()))?;
    }
    Ok(())
}

fn render(
    gpu: &mut Gpu,
    frame: &PreparedGpuFrame,
    limits: RasterLimits,
) -> motion_core::Result<crate::FrameBuffer> {
    Ok(crate::FrameBuffer::from_linear_rgba8(
        &gpu.render(&frame.records, Plan::new(frame, limits)?)?,
        limits,
    )?)
}

fn fixtures() -> Result<Vec<(&'static str, Node, bool, f64)>, ValidationError> {
    let paints = Node::group([
        Node::from(
            Rect::new(0.25, 0.5, 12.75, 9.25).fill(
                LinearGradient::new(Point::ORIGIN, Point::new(1.0, 1.0))
                    .stop(0.0, Color::rgb(235, 30, 75))
                    .stop(1.0, Color::rgb(20, 190, 230)),
            ),
        )
        .opacity(0.7),
        Node::from(Rect::new(4.5, 3.25, 15.0, 12.0).fill(Color::WHITE))
            .opacity(0.4)
            .blend_mode(BlendMode::Multiply),
    ])
    .opacity(0.8);
    let curve = Path2D::builder()
        .move_to(Point::new(4.1, 40.2))
        .cubic_to(
            Point::new(20.3, 10.1),
            Point::new(50.7, 63.0),
            Point::new(59.3, 30.4),
        )
        .line_to(Point::new(59.3, 30.8))
        .cubic_to(
            Point::new(50.7, 63.4),
            Point::new(20.3, 10.5),
            Point::new(4.1, 40.6),
        )
        .close()
        .build()?;
    let geometry = Node::group([
        star(18.25, FillRule::NonZero)?,
        star(45.75, FillRule::EvenOdd)?,
        Node::from(curve.fill(Color::rgb(240, 180, 30))),
        Node::from(Circle::new(Point::new(32.0, 48.5), 10.3)?.fill(Color::WHITE.with_alpha(0.4))),
    ]);
    let filtered = geometry
        .clone()
        .clip_with(Rect::new(8.25, 4.5, 56.75, 59.5), ClipMode::Geometric)
        .clip_with(
            Circle::new(Point::new(32.25, 32.5), 28.5)?,
            ClipMode::Geometric,
        )
        .filter(Filter::Blur { sigma: 0.6 });
    Ok(vec![
        ("paints", paints, false, PAINT_VISUAL_RMS),
        ("geometry", geometry, true, GEOMETRY_VISUAL_RMS),
        ("filtered_clips", filtered, true, FILTER_VISUAL_RMS),
    ])
}

fn star(center_x: f64, rule: FillRule) -> Result<Node, ValidationError> {
    let mut path = Path2D::new();
    for index in 0..5 {
        let angle = f64::from(index) * 4.0 * std::f64::consts::PI / 5.0 - 1.5;
        let point = Point::new(center_x + 14.0 * angle.cos(), 19.25 + 14.0 * angle.sin());
        if index == 0 {
            path.try_move_to(point)?;
        } else {
            path.try_line_to(point)?;
        }
    }
    path.try_close()?;
    Ok(Node::from(
        path.fill(Color::rgb(210, 80, 150).with_alpha(0.7))
            .fill_rule(rule),
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn startup_fixtures_exercise_independent_coverage() -> motion_core::Result<()> {
        let limits = RasterLimits::default();
        let validation = ValidationLimits::default();
        let mut device_cases = 0;
        for (label, node, requires_device, _) in fixtures()? {
            let list = node.compile(Size::new(64.0, 64.0), PixelSize::new(64, 64), validation)?;
            let reference = list.prepare(validation, limits)?;
            let device = crate::gpu::raster::prepare::prepare_validated_device(
                &list,
                validation,
                limits,
                1.0 / 256.0,
                None,
            )?;
            assert!(!reference.has_device_coverage());
            if requires_device {
                assert!(device.has_device_coverage(), "{label}");
                device_cases += 1;
            }
        }
        assert_eq!(device_cases, 2);
        Ok(())
    }

    #[test]
    #[ignore = "requires a native GPU; checks layer attachment capabilities without a window"]
    fn native_layer_attachment_capabilities() -> motion_core::Result {
        pollster::block_on(async {
            let instance = crate::gpu::instance(crate::BackendKind::Gpu)?;
            let adapter = instance
                .request_adapter(&wgpu::RequestAdapterOptions::default())
                .await
                .map_err(|error| motion_core::Error::GpuUnavailable(Box::new(error)))?;
            let info = crate::gpu::adapter_info(&adapter)?;
            for format in [
                wgpu::TextureFormat::Rgba32Float,
                wgpu::TextureFormat::Rgba16Float,
            ] {
                let features = adapter.get_texture_format_features(format);
                eprintln!(
                    "Layer attachment {format:?} on {info:?}: usages={:?}, flags={:?}, adapter_specific={}",
                    features.allowed_usages,
                    features.flags,
                    adapter
                        .features()
                        .contains(wgpu::Features::TEXTURE_ADAPTER_SPECIFIC_FORMAT_FEATURES)
                );
                assert!(
                    features
                        .allowed_usages
                        .contains(wgpu::TextureUsages::RENDER_ATTACHMENT)
                );
            }
            let context = crate::GpuContext::new()?;
            let supported = adapter
                .features()
                .contains(wgpu::Features::TEXTURE_ADAPTER_SPECIFIC_FORMAT_FEATURES)
                && adapter
                    .get_texture_format_features(wgpu::TextureFormat::Rgba32Float)
                    .flags
                    .contains(wgpu::TextureFormatFeatureFlags::BLENDABLE);
            assert_eq!(context.textures.blends.is_some(), supported);
            Ok(())
        })
    }
}
