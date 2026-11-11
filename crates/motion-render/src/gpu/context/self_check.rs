//! Shared once-only startup parity check.

use super::*;
use motion_core::Shape;

impl GpuContext {
    /// Runs a shared once-only native/CPU parity scene, including alpha, geometry,
    /// spatial paints and group blending. Failure is sticky for this context.
    pub fn self_check(&self) -> motion_core::Result<()> {
        self.loss.check()?;
        let result = self.self_check_result.get_or_init(|| {
            use motion_core::NodeModifiers;
            let limits = RasterLimits::default();
            let node = motion_core::Node::group([
                motion_core::Node::from(
                    motion_core::Rect::new(0.25, 0.5, 12.75, 9.25).fill(
                        motion_core::LinearGradient::new(
                            motion_core::Point::ORIGIN,
                            motion_core::Point::new(1.0, 1.0),
                        )
                        .stop(0.0, motion_core::Color::rgb(235, 30, 75))
                        .stop(1.0, motion_core::Color::rgb(20, 190, 230)),
                    ),
                )
                .opacity(0.7),
                motion_core::Node::from(
                    motion_core::Rect::new(4.5, 3.25, 15.0, 12.0).fill(motion_core::Color::WHITE),
                )
                .opacity(0.4)
                .blend_mode(motion_core::BlendMode::Multiply),
            ])
            .opacity(0.8);
            let frame = node
                .compile(
                    motion_core::Size::new(16.0, 12.0),
                    PixelSize::new(16, 12),
                    motion_core::ValidationLimits::default(),
                )
                .and_then(|list| list.prepare(motion_core::ValidationLimits::default(), limits))
                .map_err(|error| error.to_string())?;
            let expected = frame
                .compose_linear(limits)
                .and_then(|frame| crate::FrameBuffer::from_linear_rgba8(&frame, limits))
                .map_err(|error| error.to_string())?;
            let actual = Gpu::from_context(self.clone())
                .render(
                    &frame,
                    Plan::new(&frame, limits).map_err(|error| error.to_string())?,
                )
                .and_then(|frame| {
                    crate::FrameBuffer::from_linear_rgba8(&frame, limits).map_err(Into::into)
                })
                .map_err(|error| error.to_string())?;
            let differences = expected
                .bytes()
                .as_chunks::<4>()
                .0
                .iter()
                .zip(actual.bytes().as_chunks::<4>().0)
                .map(|(expected, actual)| {
                    expected
                        .iter()
                        .zip(actual)
                        .map(|(a, b)| a.abs_diff(*b))
                        .max()
                        .unwrap_or(0)
                })
                .collect::<Vec<_>>();
            if differences.iter().any(|difference| *difference > 2)
                || differences
                    .iter()
                    .filter(|difference| **difference > 1)
                    .count()
                    * 1000
                    >= differences.len()
            {
                return Err("Native GPU self-check exceeded CPU parity tolerance".into());
            }
            Ok(())
        });
        result.clone().map_err(|message| {
            motion_core::Error::GpuUnavailable(Box::new(std::io::Error::other(message)))
        })
    }
}
