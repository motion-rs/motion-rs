//! Immutable shared coverage and paint records.

pub(crate) use crate::cpu::LinearFrame;
pub use crate::cpu::WorkStats;
pub(crate) use crate::raster::coverage_cache::CoverageCache;
use crate::raster::paint;
pub use crate::raster::prepare::RenderPrepare;
pub(crate) use crate::raster::prepare::prepare_validated_with_cache;
pub use motion_core::RasterLimits;
use motion_core::{PixelSize, Rect};
use std::ops::Range;

/// Half-open device pixel bounds, clipped to the internal raster by preparation.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct PixelRect {
    /// Inclusive left pixel coordinate.
    pub left: u32,
    /// Inclusive top pixel coordinate.
    pub top: u32,
    /// Exclusive right pixel coordinate.
    pub right: u32,
    /// Exclusive bottom pixel coordinate.
    pub bottom: u32,
}

impl PixelRect {
    /// Horizontal extent in pixels.
    pub fn width(self) -> u32 {
        self.right - self.left
    }

    /// Vertical extent in pixels.
    pub fn height(self) -> u32 {
        self.bottom - self.top
    }

    /// Whether the rectangle covers no pixels.
    pub fn is_empty(self) -> bool {
        self.left >= self.right || self.top >= self.bottom
    }

    /// Overlap with `other`; disjoint rectangles produce an empty rectangle.
    pub fn intersect(self, other: Self) -> Self {
        let left = self.left.max(other.left);
        let top = self.top.max(other.top);
        Self {
            left,
            top,
            right: self.right.min(other.right).max(left),
            bottom: self.bottom.min(other.bottom).max(top),
        }
    }

    pub(crate) fn pixels(self) -> u64 {
        u64::from(self.width()) * u64::from(self.height())
    }
}

/// Edge coverage in a band of up to four rows, possibly shifted by cached translation.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Strip {
    /// First output column.
    pub(crate) x: u32,
    /// First output row; translated cached bands need not be four-row aligned.
    pub(crate) y: u32,
    /// Number of columns.
    pub(crate) width: u32,
    /// Number of rows, in 1..=4 and contained in the output.
    pub(crate) height: u32,
    /// Row-major start in PreparedFrame::alphas; no padding between rows.
    pub(crate) alpha_offset: usize,
}

impl Strip {
    /// First output column.
    pub fn x(&self) -> u32 {
        self.x
    }

    /// First output row.
    pub fn y(&self) -> u32 {
        self.y
    }

    /// Number of columns.
    pub fn width(&self) -> u32 {
        self.width
    }

    /// Number of rows, in 1..=4.
    pub fn height(&self) -> u32 {
        self.height
    }

    /// Row-major start in `PreparedFrame::alphas`; no padding between rows.
    pub fn alpha_offset(&self) -> usize {
        self.alpha_offset
    }
}

/// A fully covered interval in a band of up to four rows.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Span {
    /// Inclusive first column; cached translation may change its four-pixel phase.
    pub(crate) start_x: u32,
    /// Exclusive last column.
    pub(crate) end_x: u32,
    /// First row; cached translation may change its four-row phase.
    pub(crate) y: u32,
    /// Number of rows, in 1..=4 and contained in the output.
    pub(crate) height: u32,
}

impl Span {
    /// Inclusive first column.
    pub fn start_x(&self) -> u32 {
        self.start_x
    }

    /// Exclusive last column.
    pub fn end_x(&self) -> u32 {
        self.end_x
    }

    /// First row.
    pub fn y(&self) -> u32 {
        self.y
    }

    /// Number of rows, in 1..=4.
    pub fn height(&self) -> u32 {
        self.height
    }
}

/// A finite linear-sRGB premultiplied f32 constant paint record.
/// Spatial descriptors are retained separately; use PreparedFrame::paint_at
/// for a sample that also resolves gradients or image patterns.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct PaintRecord {
    /// Extended linear RGB is retained; alpha is in 0..=1.
    pub linear_premultiplied: [f32; 4],
}

/// path-mask or cumulative geometric clip coverage in shared arrays.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ClipRecord {
    /// Conservative device bounds, clipped to the internal raster.
    pub bounds: PixelRect,
    /// Disjoint partial coverage strips belonging to this clip.
    pub strips: Range<usize>,
    /// Fully covered intervals belonging to this clip.
    pub spans: Range<usize>,
}

/// A normalized isolated layer shared by CPU and GPU scope consumers.
#[derive(Clone, Debug, PartialEq)]
pub struct PreparedLayer {
    /// Once-per-group opacity applied on completion.
    pub opacity: f64,
    /// Conservative raster capture bounds.
    pub bounds: PixelRect,
    /// Blend against the immediate parent exactly once.
    pub blend_mode: motion_core::BlendMode,
    /// Independent multiplicative path-alpha coverage.
    pub alpha_clip: Option<usize>,
    /// Cumulative geometric intersection; None inherits the parent domain.
    pub geometric_clip: Option<usize>,
    /// Source-only effects create an independent geometric domain.
    pub filters: Option<PreparedFilters>,
}

/// Ordered source-only effects and the unfiltered raster-space input bounds.
#[derive(Clone, Debug, PartialEq)]
pub struct PreparedFilters {
    /// Device input region before effects expand or shift it.
    pub source_bounds: Rect,
    /// Validated effects in declaration order.
    pub operations: Vec<motion_core::Filter>,
}

/// Prepared operations in exactly the original command order.
#[derive(Clone, Debug, PartialEq)]
#[non_exhaustive]
pub enum PreparedItem {
    /// analytic/sprite/segment instances, without path coverage expansion.
    ParticleInstances {
        /// Validated immutable payload.
        batch: std::sync::Arc<dyn motion_core::instances::InstanceBatch>,
        /// Local-to-raster matrix.
        transform: motion_core::Transform,
    },
    /// Begins a normalized isolated scope, with explicit clip and filter domains.
    PushLayer {
        /// Shared opacity, capture, composition, geometry and source-effect description.
        layer: PreparedLayer,
    },
    /// Ends the most recent isolated layer.
    PopLayer,
    /// Begins isolated artwork used to mask the current completed capture.
    BeginMask {
        /// Extraction mode applied at EndMask.
        mode: motion_core::drawing::MaskMode,
    },
    /// Multiplies the current capture by completed mask artwork once.
    EndMask,
    /// Disjoint strip/span coverage and an index into the shared paint records.
    Fill {
        /// Edge coverage strip range.
        strips: Range<usize>,
        /// Fully covered span range.
        spans: Range<usize>,
        /// Solid paint record index.
        paint: usize,
    },
}

/// Logical preparation totals, excluding allocator capacity and source storage.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct PreparationStats {
    /// Frame value, items, records, strips, spans and alpha bytes retained.
    pub retained_bytes: u64,
    /// Peak live logical storage, including temporary geometry and coverage.
    pub peak_bytes: u64,
    /// Admitted geometry, sorting, bands and coverage work.
    pub work: u64,
}

/// Immutable coverage shared by future CPU/GPU consumers.
///
/// Supports transformed rectangles, bounded flattened Bezier contours with either
/// fill rule and isolated linear-blended layers.
/// Private storage prevents callers forging ranges or editing validated geometry.
#[derive(Clone, Debug, PartialEq)]
pub struct PreparedFrame {
    pub(crate) quality: motion_core::RenderQuality,
    pub(crate) blend_space: motion_core::drawing::BlendSpace,
    pub(crate) size: PixelSize,
    pub(crate) output_bounds: PixelRect,
    pub(crate) filter_halo: u32,
    pub(crate) background: Option<PaintRecord>,
    pub(crate) items: Vec<PreparedItem>,
    pub(crate) strips: Vec<Strip>,
    pub(crate) spans: Vec<Span>,
    pub(crate) alphas: Vec<u8>,
    pub(crate) paints: Vec<PaintRecord>,
    pub(crate) paint_sources: Vec<Option<paint::Source>>,
    pub(crate) clips: Vec<ClipRecord>,
    pub(crate) stats: PreparationStats,
}

impl PreparedFrame {
    /// Selects approximate Draft filter kernels; default preparation remains Final.
    pub fn quality(mut self, quality: motion_core::RenderQuality) -> Self {
        self.quality = quality;
        self
    }

    /// Selects the compositing color space while retaining linear working pixels.
    pub fn blend_space(mut self, space: motion_core::drawing::BlendSpace) -> Self {
        self.blend_space = space;
        self
    }

    /// Final integer output dimensions, excluding filter guard bands.
    pub fn size(&self) -> PixelSize {
        PixelSize::new(
            self.output_bounds.right - self.output_bounds.left,
            self.output_bounds.bottom - self.output_bounds.top,
        )
    }

    /// Internal raster dimensions, including any filter guard band.
    pub fn raster_size(&self) -> PixelSize {
        self.size
    }

    /// Output window in shared raster coordinates; coverage includes guard bands.
    pub fn output_bounds(&self) -> PixelRect {
        self.output_bounds
    }

    /// Optional linear-premultiplied background, not rasterized during preparation.
    pub fn background(&self) -> Option<PaintRecord> {
        self.background
    }

    /// Read-only operations in source order; collapsed instance transforms may be omitted.
    pub fn items(&self) -> &[PreparedItem] {
        &self.items
    }

    /// Shared edge strip metadata.
    pub fn strips(&self) -> &[Strip] {
        &self.strips
    }

    /// Shared interior intervals.
    pub fn spans(&self) -> &[Span] {
        &self.spans
    }

    /// Shared row-major edge coverage, nearest-rounded to u8.
    pub fn alphas(&self) -> &[u8] {
        &self.alphas
    }

    /// Constant records, one per fill; spatial fills have transparent placeholders.
    /// paint_at resolves the shared spatial descriptors and their color tables.
    pub fn paints(&self) -> &[PaintRecord] {
        &self.paints
    }

    /// Independent path masks and cumulative geometric regions in shared arrays.
    pub fn clips(&self) -> &[ClipRecord] {
        &self.clips
    }

    /// Logical retained/peak storage and admitted preparation work.
    pub fn stats(&self) -> PreparationStats {
        self.stats
    }
}
