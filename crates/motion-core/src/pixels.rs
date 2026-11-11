//! Immutable image sampling contracts, independent of decoding and rendering.

use std::{any::Any, fmt::Debug};

/// Texture reconstruction requested by a drawing command.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
#[cfg_attr(
    feature = "serde",
    derive(serde::Serialize, serde::Deserialize),
    serde(rename_all = "snake_case")
)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
pub enum Sampling {
    /// Nearest base-level texel.
    Nearest,
    /// Bilinear base-level reconstruction.
    Bilinear,
    /// Bilinear reconstruction between adjacent mip levels.
    #[default]
    Trilinear,
}

/// A predecoded immutable linear-premultiplied image mip chain.
/// Content owns decoding/storage; backends consume this shared logical contract.
pub trait ImagePixels: Any + Debug + Send + Sync {
    /// Base-level width in texels.
    fn width(&self) -> u32;

    /// Base-level height in texels.
    fn height(&self) -> u32;

    /// Number of immutable mip levels.
    fn level_count(&self) -> usize;

    /// Width/height of an existing mip level.
    fn level_size(&self, level: usize) -> (u32, u32);

    /// One in-bounds finite linear-premultiplied texel.
    fn texel(&self, level: usize, column: usize, row: usize) -> [f32; 4];

    /// Appends every texel of an existing level in row-major order, four `f32` each.
    /// Overriding this avoids one dynamic call per texel for backends that upload whole levels.
    fn extend_level(&self, level: usize, out: &mut Vec<f32>) {
        let (width, height) = self.level_size(level);
        out.reserve(width as usize * height as usize * 4);
        for row in 0..height as usize {
            for column in 0..width as usize {
                out.extend(self.texel(level, column, row));
            }
        }
    }

    /// Whether another implementation owns the same immutable pixel content.
    fn content_eq(&self, other: &dyn ImagePixels) -> bool;

    /// Concrete storage identity, resolved only by its owning content implementation.
    fn as_any(&self) -> &dyn Any;
}
