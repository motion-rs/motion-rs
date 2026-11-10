//! Paper models shared by ink fills and brush strokes.

use crate::ImageAsset;
use motion_core::{GradientExtend, ImagePattern, Point, Transform, ValidationError};
use std::sync::Arc;

/// The sheet an [`Ink`](super::Ink) is absorbed into, set with [`Ink::paper`](super::Ink::paper).
///
/// Both variants only attenuate pigment coverage and are sampled in paper space, which is local
/// coordinates mapped by [`Ink::paper_transform`](super::Ink::paper_transform).
#[derive(Clone, Debug)]
pub enum Paper {
    /// Seeded procedural fibers; needs no resources.
    Procedural {
        /// Seed of the fiber noise.
        seed: u32,
        /// Coverage attenuation in `0..=1`.
        strength: f32,
        /// Finite positive paper-space pixels per noise cell.
        scale: f32,
    },
    /// Repeating immutable paper height map.
    Texture {
        /// Already resolved image; no implicit file loading during rendering.
        texture: Arc<ImageAsset>,
        /// Paper-space units per texel; finite and positive.
        grain_scale: f64,
        /// Coverage attenuation by the paper height, in `0..=1`.
        absorbency: f64,
    },
}

impl Paper {
    pub(super) fn is_valid(&self) -> bool {
        match self {
            Self::Procedural {
                strength, scale, ..
            } => {
                strength.is_finite()
                    && (0.0..=1.0).contains(strength)
                    && scale.is_finite()
                    && *scale > 0.0
            }
            Self::Texture {
                grain_scale,
                absorbency,
                ..
            } => {
                grain_scale.is_finite()
                    && *grain_scale > 0.0
                    && absorbency.is_finite()
                    && (0.0..=1.0).contains(absorbency)
            }
        }
    }
}

/// A texture paper resolved once for a baked material.
pub(super) struct PaperSampler {
    pattern: ImagePattern,
    absorbency: f64,
}

impl PaperSampler {
    pub(super) fn new(texture: Arc<ImageAsset>, grain_scale: f64, absorbency: f64) -> Self {
        Self {
            pattern: ImagePattern::from_asset(texture)
                .transform(Transform::scale(grain_scale))
                .extend(GradientExtend::Repeat),
            absorbency,
        }
    }

    /// Returns the coverage multiplier at a paper-space point.
    pub(super) fn factor(&self, point: Point) -> Result<f64, ValidationError> {
        let sample = image_sample(&self.pattern, point)?;
        let height = if sample[3] > 0.0 {
            (sample[0] * 0.2126 + sample[1] * 0.7152 + sample[2] * 0.0722) / sample[3]
        } else {
            0.0
        };
        Ok(1.0 - self.absorbency * height.clamp(0.0, 1.0))
    }
}

/// Bilinear premultiplied sample of an image pattern at an absolute point.
pub(super) fn image_sample(
    pattern: &ImagePattern,
    point: Point,
) -> Result<[f64; 4], ValidationError> {
    let inverse = pattern.transform.inverse().ok_or_else(|| {
        ValidationError::invalid("brush_stroke", "paint", "Noninvertible paint transform")
    })?;
    let point = inverse.apply(point);
    let width = pattern.image.width() as usize;
    let height = pattern.image.height() as usize;
    if pattern.decal
        && (point.x < 0.0 || point.y < 0.0 || point.x >= width as f64 || point.y >= height as f64)
    {
        return Ok([0.0; 4]);
    }
    let horizontal = point.x - 0.5;
    let vertical = point.y - 0.5;
    let column = horizontal.floor();
    let row = vertical.floor();
    let fraction = [horizontal - column, vertical - row];
    let address = |index: i64, length: usize| match pattern.extend {
        GradientExtend::Pad => index.clamp(0, length as i64 - 1) as usize,
        GradientExtend::Repeat => index.rem_euclid(length as i64) as usize,
        GradientExtend::Reflect => {
            let index = index.rem_euclid(length as i64 * 2) as usize;
            if index >= length {
                length * 2 - index - 1
            } else {
                index
            }
        }
    };
    let mut result = [0.0; 4];
    for vertical in 0..2 {
        for horizontal in 0..2 {
            let weight = (if horizontal == 0 {
                1.0 - fraction[0]
            } else {
                fraction[0]
            }) * (if vertical == 0 {
                1.0 - fraction[1]
            } else {
                fraction[1]
            });
            let texel = pattern.image.texel(
                0,
                address(column as i64 + horizontal, width),
                address(row as i64 + vertical, height),
            );
            for channel in 0..4 {
                result[channel] += f64::from(texel[channel]) * weight;
            }
        }
    }
    Ok(result)
}
