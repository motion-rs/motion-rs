//! Desktop workbench over shared scene, rendering and encoding contracts.

#![forbid(unsafe_code)]

use audio::settings::AudioOptions;
#[cfg(feature = "gpu")]
use motion_render::GpuContext;

mod app;
mod audio;
mod export;
mod panels;
mod playback;
mod preview;
mod session;
mod theme;
mod timeline_ui;
mod viewer;

pub use app::{StudioScene, run};

pub use audio::model::{AudioReadout, BusReadout};

#[cfg(feature = "studio")]
pub mod particles;

/// Shared scene, rendering and encoding operations consumed by the workbench.
pub mod prelude {
    #[cfg(feature = "studio")]
    pub use crate::particles::particle_presets;
    pub use crate::{StudioScene, run};
}
