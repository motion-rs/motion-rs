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
    Composition, FrameAt, FrameClock, FrameRenderer, FrameReport, FrameRequest, FrameStats,
    RenderSession,
};
pub use context::{GpuBackend, GpuInfo, RenderCapabilities, RenderContext};
pub use display::{Cdl, DisplayTransform, FilmGrain, OutputTransfer, ToneMap};
#[cfg(feature = "gpu")]
pub use gpu::GpuContext;
pub use look::{GateWeave, Look, SceneEffects};
pub use motion_core::{CachePartition, RenderCache, RenderQuality};
pub use options::{RenderOptions, render_still};
pub use output::{FrameBuffer, PixelFormat, PixelPlane, RenderTarget};
pub use prepared::{
    PixelRect, PreparationStats, PreparedFrame, RasterLimits, RenderPrepare, WorkStats,
};

/// Rendering contracts used alongside core authoring values.
pub mod prelude {
    pub use crate::{
        BackendPreference, FrameAt, FrameClock, FrameRenderer, FrameRequest, FrameStats,
        RenderOptions, RenderQuality, RenderSession, render_still,
    };
    pub use crate::{Cdl, DisplayTransform, FilmGrain, OutputTransfer, ToneMap};
    pub use crate::{
        FrameBuffer, PixelFormat, PixelPlane, PreparedFrame, RenderContext, RenderPrepare,
        RenderTarget, Renderer,
    };
}
