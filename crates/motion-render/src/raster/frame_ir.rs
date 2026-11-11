//! Immutable, admitted device-space geometry and ordered composition semantics.
//!
//! The CPU recipe accompanies device curves to preserve reference transform arithmetic and
//! complete coverage-cache keys. It is not a display list and contains no coverage.

use crate::prepared::{PaintRecord, PixelRect, PreparationStats, PreparedLayer};
use motion_core::budget::Budget;
use motion_core::{
    FillRule, Paint, Path2D, PathClip, PathVerb, PixelSize, RasterLimits, Rect, RectClip,
    ShapeClip, Transform, ValidatedDisplayList, ValidationError, ValidationLimits,
};
use std::mem::size_of;

mod geometry;
mod normalize;
#[cfg(test)]
mod tests;

pub(super) use geometry::rectangle_verbs;

/// Maximum scratch bytes for a command-index error path, including every usize digit.
pub(super) const COMMAND_PATH_BYTES: u64 = 64;

/// Backend-neutral ordered frame, owning normalization but borrowing immutable resources.
pub(crate) struct FrameIr<'a> {
    size: PixelSize,
    output_bounds: PixelRect,
    background: Option<PaintRecord>,
    tolerance: f64,
    filter_halo: u32,
    items: Vec<IrItem>,
    paths: Vec<IrPath<'a>>,
    paints: Vec<IrPaint<'a>>,
    clips: Vec<IrClip>,
    segments: u64,
    resource_bytes: u64,
    stats: PreparationStats,
}

impl<'a> FrameIr<'a> {
    pub(crate) fn build(
        source: &'a ValidatedDisplayList,
        validation: ValidationLimits,
        limits: RasterLimits,
        tolerance: f64,
    ) -> Result<Self, ValidationError> {
        // Validate every branch before any normalization or backend visibility decisions.
        source.preflight(validation)?;
        if !tolerance.is_finite() || tolerance <= 0.0 {
            return Err(ValidationError::invalid(
                "prepared",
                "tolerance",
                "Curve tolerance must be finite and positive",
            ));
        }
        let list = source.display_list();
        let mut budget = Budget::new(list.output_size, limits)?;
        budget.bytes(size_of::<Self>() as u64, "frame_ir")?;
        let padding = normalize::padding(&list.commands, &mut budget)?;
        let dimension = |value: u32| {
            padding
                .checked_mul(2)
                .and_then(|extra| value.checked_add(extra))
                .ok_or_else(|| {
                    ValidationError::limit(
                        "prepared.filters",
                        "max_pixels",
                        "Filter raster dimensions overflow",
                    )
                })
        };
        let size = PixelSize::new(
            dimension(list.output_size.width)?,
            dimension(list.output_size.height)?,
        );
        limits.validate_size(size)?;
        let mut frame = Self {
            size,
            output_bounds: PixelRect {
                left: padding,
                top: padding,
                right: padding + list.output_size.width,
                bottom: padding + list.output_size.height,
            },
            background: list
                .background
                .map(|color| super::prepare::paint_record(color, "prepared.background"))
                .transpose()?,
            tolerance,
            filter_halo: 0,
            items: Vec::new(),
            paths: Vec::new(),
            paints: Vec::new(),
            clips: Vec::new(),
            segments: 0,
            resource_bytes: 0,
            stats: PreparationStats::default(),
        };
        frame.normalize(&list.commands, padding, &mut budget)?;
        frame.stats = PreparationStats {
            retained_bytes: budget.live_bytes(),
            peak_bytes: budget.peak_bytes(),
            work: budget.work_done(),
        };
        Ok(frame)
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

    pub(crate) fn filter_halo(&self) -> u32 {
        self.filter_halo
    }

    pub(super) fn items(&self) -> &[IrItem] {
        &self.items
    }

    pub(super) fn path(&self, index: usize) -> &IrPath<'a> {
        &self.paths[index]
    }

    pub(super) fn paint(&self, index: usize) -> &IrPaint<'a> {
        &self.paints[index]
    }

    pub(super) fn clip(&self, index: usize) -> &IrClip {
        &self.clips[index]
    }

    pub(crate) fn stats(&self) -> PreparationStats {
        self.stats
    }

    pub(crate) fn resource_bytes(&self) -> u64 {
        self.resource_bytes
    }
}

pub(super) struct IrPath<'a> {
    pub(super) source: GeometrySource<'a>,
    pub(super) transform: Transform,
    pub(super) rule: FillRule,
    pub(super) bounds: Rect,
    pub(super) device: Vec<PathVerb>,
    pub(super) segment_upper_bound: u64,
}

#[derive(Clone, Copy)]
pub(super) enum GeometrySource<'a> {
    Rectangle(Rect),
    Path(&'a Path2D),
}

pub(super) struct IrPaint<'a> {
    pub(super) source: &'a Paint,
    pub(super) bounds: Rect,
    pub(super) transform: Transform,
}

pub(super) enum IrClip {
    Alpha { source: PathClip, path: usize },
    Rectangle { source: RectClip, path: usize },
    Shape { source: ShapeClip, path: usize },
}

pub(super) enum IrItem {
    ParticleInstances {
        batch: std::sync::Arc<dyn motion_core::instances::InstanceBatch>,
        transform: Transform,
    },
    PushLayer {
        layer: PreparedLayer,
        alpha_clip: Option<usize>,
        rectangle_clip: Option<usize>,
        shape_clip: Option<usize>,
    },
    PopLayer,
    BeginMask {
        mode: motion_core::MaskMode,
    },
    EndMask,
    Fill {
        path: usize,
        paint: usize,
    },
}
