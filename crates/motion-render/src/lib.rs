//! Bounded preparation and rendering of backend-independent drawing commands.

#![forbid(unsafe_code)]

pub mod backend;
pub mod composition;
pub mod context;
pub(crate) mod cpu;
pub mod display;
#[cfg(feature = "gpu")]
pub mod gpu;
pub mod look;
pub mod options;
pub mod output;
pub mod prepared;
pub(crate) mod raster;

pub use backend::{BackendDecision, BackendPreference, Renderer};
pub use composition::{
    Composition, FrameClock, FrameRenderer, FrameRequest, FrameStats, RenderSession,
};
pub use context::{GpuBackend, GpuInfo, RenderCapabilities, RenderContext};
pub use display::{Cdl, DisplayTransform, FilmGrain, OutputTransfer, ToneMap};
#[cfg(feature = "gpu")]
pub use gpu::GpuContext;
pub use look::{GateWeave, Look};
pub use motion_core::{CachePartition, RenderCache, RenderQuality};
pub use options::{RenderOptions, composition, render_still};
pub use output::{Bitmap, FrameBuffer, PixelFormat, PixelPlane, RenderTarget, Snapshot};
pub use prepared::{
    ClipRecord, LinearFrame, LinearRenderStats, OutputStats, PaintRecord, PixelRect,
    PreparationStats, PreparedFilters, PreparedFrame, PreparedItem, PreparedLayer, RasterLimits,
    RenderPrepare, Rgba8Frame, Span, Strip,
};

/// Rendering contracts used alongside core authoring values.
pub mod prelude {
    pub use crate::{
        BackendPreference, FrameClock, FrameRenderer, FrameRequest, FrameStats, RenderOptions,
        RenderQuality, RenderSession, render_still,
    };
    pub use crate::{
        Bitmap, FrameBuffer, LinearFrame, PixelFormat, PixelPlane, PreparedFrame, PreparedItem,
        RenderContext, RenderPrepare, RenderTarget, Renderer,
    };
    pub use crate::{Cdl, DisplayTransform, FilmGrain, OutputTransfer, ToneMap};
}
