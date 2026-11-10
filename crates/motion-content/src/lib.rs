//! Content preparation and drawing, independent of CPU/GPU backends.

#![forbid(unsafe_code)]

#[cfg(feature = "captions")]
use captions::{CaptionCue, CaptionTrack};
#[cfg(feature = "diagram")]
use diagram::Scale;

use motion_core::MorphPath;

pub mod asset;
pub mod atmosphere;
#[cfg(feature = "captions")]
pub mod captions;
#[cfg(feature = "diagram")]
pub mod diagram;
#[cfg(feature = "text")]
pub mod font;
pub mod image;
#[cfg(feature = "particles")]
pub mod particles;
#[cfg(feature = "svg")]
pub mod svg;
#[cfg(feature = "text")]
pub mod text;
pub mod vector;

pub use asset::AssetsExt;
#[cfg(feature = "text")]
pub use font::{FontFace, FontIdentifier, FontLibrary, MAX_FONT_FACE_BYTES};
#[cfg(feature = "image")]
pub use image::{AnimatedImage, AnimatedImageAsset, ImageLoopMode};
pub use image::{Fit, Image, ImageAsset, ImageLevel, Sampling};
#[cfg(feature = "particles")]
pub use particles::{
    Attractor, Ballistic, BounceBox, BurstEmitter, ColorOverLifetime, Converge, CurlNoise,
    DiskEmitter, Drag, Gravity, NoiseDrift, Orbit, Particle, ParticleEmitter, ParticleModifier,
    ParticleSimulation, ParticleSimulationBuilder, ParticleStyle, ParticleTrajectory, PathEmitter,
    PointEmitter, RectEmitter, Seek, SimulationLimits, SimulationStats, SizeOverLifetime,
    SurfaceEmitter, TrajectoryModel, Vortex, Wind,
};

#[cfg(feature = "svg")]
pub use svg::{Svg, SvgAsset, SvgDiagnostic};
#[cfg(feature = "text")]
pub use text::{
    FontWeight, ParagraphLayout, PartStyle, Text, TextAlignment, TextGlyph, TextPart, TextSpan,
    TextSplit,
};
pub use vector::{
    Morph, SurfaceTriangle, VectorGroup, VectorImage, VectorItem, VectorLeaf, VectorMask,
    VectorPath, VectorSurface, VectorTree, WriteOptions,
};

/// Content authoring and core contracts without a renderer dependency.
pub mod prelude {
    #[cfg(feature = "image")]
    pub use crate::{AnimatedImage, AnimatedImageAsset, ImageLoopMode};
    pub use crate::{
        AssetsExt, Fit, Image, ImageAsset, ImageLevel, Morph, Sampling, VectorGroup, VectorImage,
        VectorItem, VectorLeaf, VectorMask, VectorPath, VectorSurface, VectorTree, WriteOptions,
    };
    #[cfg(feature = "text")]
    pub use crate::{
        FontFace, FontIdentifier, FontLibrary, FontWeight, ParagraphLayout, PartStyle, Text,
        TextAlignment, TextGlyph, TextPart, TextSpan, TextSplit,
    };
    #[cfg(test)]
    pub use motion_render::{Composition, RenderOptions, RenderPrepare, Renderer, render_still};
}

/// Maximum accepted `.cube` LUT text size in bytes.
const MAX_CUBE_BYTES: u64 = 8 * 1024 * 1024;

/// Loads explicitly requested LUT data without implicit resource discovery.
pub fn load_lut(path: impl AsRef<std::path::Path>) -> motion_core::Result<motion_core::Lut3d> {
    use std::io::Read;
    let file = std::fs::File::open(path)?;
    let mut text = String::new();
    file.take(MAX_CUBE_BYTES + 1).read_to_string(&mut text)?;
    Ok(motion_core::Lut3d::from_cube(&text)?)
}

/// Loads an explicitly requested standalone 1D `.cube` shaper.
pub fn load_shaper(path: impl AsRef<std::path::Path>) -> motion_core::Result<motion_core::Lut1d> {
    use std::io::Read;
    let file = std::fs::File::open(path)?;
    let mut text = String::new();
    file.take(MAX_CUBE_BYTES + 1).read_to_string(&mut text)?;
    Ok(motion_core::Lut1d::from_cube(&text)?)
}

#[cfg(feature = "image")]
pub(crate) fn image_error(error: ::image::ImageError) -> motion_core::Error {
    motion_core::Error::Other(Box::new(error))
}
