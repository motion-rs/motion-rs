//! Backend-neutral frame description built before any coverage exists.
//!
//! The validated display list already stores f64 paths that only a backend flattens. `FrameIr`
//! adds the frame facts every backend needs before it allocates: the padded raster, the output
//! window and the background.

use crate::prepared::{PaintRecord, PixelRect};
use motion_core::{DisplayList, PixelSize, ValidationError, ValidationLimits};

use super::prepare::{filter_padding, paint_record};

/// Immutable per-frame description shared by backend preparation.
pub(crate) struct FrameIr<'a> {
    source: &'a DisplayList,
    padding: u32,
    size: PixelSize,
    output_bounds: PixelRect,
    background: Option<PaintRecord>,
    tolerance: f64,
    device_coverage: bool,
}

impl<'a> FrameIr<'a> {
    /// Validates the list and measures it. Fails before any raster allocation.
    pub(crate) fn build(
        source: &'a motion_core::ValidatedDisplayList,
        validation: ValidationLimits,
        tolerance: f64,
    ) -> Result<Self, ValidationError> {
        source.preflight(validation)?;
        Self::from_list(source.display_list(), tolerance)
    }

    pub(crate) fn from_list(
        source: &'a DisplayList,
        tolerance: f64,
    ) -> Result<Self, ValidationError> {
        if !tolerance.is_finite() || tolerance <= 0.0 {
            return Err(ValidationError::invalid(
                "prepared",
                "tolerance",
                "Curve tolerance must be finite and positive",
            ));
        }
        let padding = filter_padding(&source.commands)?;
        let dimension = |value: u32, side: &'static str| {
            padding
                .checked_mul(2)
                .and_then(|extra| value.checked_add(extra))
                .ok_or_else(|| {
                    ValidationError::limit(
                        "prepared.filters",
                        "max_pixels",
                        if side == "width" {
                            "Filter raster width overflow"
                        } else {
                            "Filter raster height overflow"
                        },
                    )
                })
        };
        let size = PixelSize::new(
            dimension(source.output_size.width, "width")?,
            dimension(source.output_size.height, "height")?,
        );
        Ok(Self {
            source,
            padding,
            size,
            output_bounds: PixelRect {
                left: padding,
                top: padding,
                right: padding + source.output_size.width,
                bottom: padding + source.output_size.height,
            },
            background: source
                .background
                .map(|color| paint_record(color, "prepared.background"))
                .transpose()?,
            tolerance,
            device_coverage: false,
        })
    }

    /// Lets eligible fills leave coverage to the GPU instead of building CPU strips.
    pub(crate) fn with_device_coverage(mut self, enabled: bool) -> Self {
        self.device_coverage = enabled;
        self
    }

    pub(crate) fn device_coverage(&self) -> bool {
        self.device_coverage
    }

    pub(crate) fn list(&self) -> &'a DisplayList {
        self.source
    }

    /// Extra border on every side that keeps filter taps inside the raster.
    pub(crate) fn padding(&self) -> u32 {
        self.padding
    }

    pub(crate) fn size(&self) -> PixelSize {
        self.size
    }

    pub(crate) fn output_bounds(&self) -> PixelRect {
        self.output_bounds
    }

    pub(crate) fn background(&self) -> Option<PaintRecord> {
        self.background
    }

    pub(crate) fn tolerance(&self) -> f64 {
        self.tolerance
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use motion_core::{Node, RasterLimits};

    #[test]
    fn ir_reports_padded_raster_and_output_window() -> Result<(), ValidationError> {
        let list = Node::empty().compile(
            motion_core::Size::new(8.0, 4.0),
            PixelSize::new(8, 4),
            ValidationLimits::default(),
        )?;
        let ir = FrameIr::build(&list, ValidationLimits::default(), 1.0 / 256.0)?;
        assert_eq!(ir.padding(), 0);
        assert_eq!(ir.size(), PixelSize::new(8, 4));
        assert_eq!(ir.output_bounds().width(), 8);
        assert!(FrameIr::build(&list, ValidationLimits::default(), 0.0).is_err());
        let frame = super::super::prepare::prepare_ir(&ir, RasterLimits::default(), None)?;
        assert_eq!(frame.size(), ir.size());
        Ok(())
    }
}
