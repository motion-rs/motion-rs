//! Shared bounded raster frontend for CPU and GPU consumers.

use crate::prepared::{ClipRecord, PaintRecord, PixelRect, PreparedScene, Span, Strip};
use motion_core::budget::Budget;
use std::mem::size_of;

pub(crate) mod coverage_cache;
pub(crate) mod frame_ir;
pub(crate) mod paint;
pub(crate) mod polygon;
pub mod prepare;
pub(crate) mod rect_clip;

pub(crate) use coverage_cache::CoverageCache;
use prepare::paint_record;
pub(crate) use prepare::pixel_bounds;

#[cfg(test)]
use crate::prepared::{PreparationStats, PreparedFrame, PreparedItem};
#[cfg(test)]
use motion_core::RasterLimits;
#[cfg(test)]
use prepare::RenderPrepare;

#[cfg(test)]
mod property_tests;
#[cfg(test)]
pub(crate) mod tests;

/// Bias added before rounding scaled coverage. Accumulated areas can land a few ulps below an
/// exact half byte (`k + 0.5`) after scaling by 255; the bias rounds them up like the exact
/// value, so equal coverage maps to the same byte regardless of accumulation order.
const COVERAGE_ROUNDING_EPSILON: f64 = 1e-10;

/// Quantizes exact edge coverage to an 8-bit alpha.
pub(crate) fn coverage_to_byte(area: f64) -> u8 {
    (area.clamp(0.0, 1.0) * 255.0 + COVERAGE_ROUNDING_EPSILON).round() as u8
}

/// Budget and error path that every raster kernel charges and reports against.
pub(crate) struct RasterScope<'a> {
    pub(crate) budget: &'a mut Budget,
    pub(crate) path: &'a str,
}

impl<'a> RasterScope<'a> {
    pub(crate) fn new(budget: &'a mut Budget, path: &'a str) -> Self {
        Self { budget, path }
    }
}

/// A horizontal slice of the raster: its first row and the half-open column range.
#[derive(Clone, Copy)]
pub(crate) struct Band {
    pub(crate) row: u32,
    pub(crate) left: u32,
    pub(crate) right: u32,
}
