//! Budgeted procedural sprite atlases and data-only vector particle presentation.

use super::Compiler;
use crate::particles::{ParticleMaterialSpec, ParticleSheetSpec, ParticleStyleSpec};
use motion_content::particles::{
    BokehShape, ParticleStyle, ProceduralSprite, SpriteAtlas, SpriteSheetMode,
};
use motion_core::{Circle, Node, PixelSize, Point, RasterLimits, Shape};
use std::sync::Arc;

/// Maximum retained bytes admitted for each small procedural atlas.
const ATLAS_BYTES: u64 = 4 * 1024 * 1024;

/// Conservative bytes charged per sprite texel for generation, mips and atlas packing.
const SPRITE_TEXEL_BYTES: u64 = 64;

/// Largest sprite side whose texels fit [`ATLAS_BYTES`] on their own.
const MAX_SPRITE_SIDE: u32 = 256;

/// Maximum explicit procedural frames per atlas.
const MAX_ATLAS_FRAMES: usize = 64;

impl Compiler<'_> {
    pub(super) fn particle_material_style(
        &mut self,
        spec: &ParticleStyleSpec,
        path: &str,
    ) -> Option<ParticleStyle> {
        match spec {
            ParticleStyleSpec::Bokeh {
                radius,
                aperture,
                focus,
                blades,
                ring,
                hardness,
                highlight,
            } => {
                let shape = match blades {
                    Some(blades) if (3..=32).contains(blades) => BokehShape::Polygon(*blades),
                    Some(_) => {
                        self.limit(path, "blades", "Bokeh blades must be in 3..=32");
                        return None;
                    }
                    None => BokehShape::Disc,
                };
                Some(ParticleStyle::Bokeh {
                    radius: *radius,
                    aperture: *aperture,
                    focus: *focus,
                    shape,
                    ring: *ring,
                    hardness: *hardness,
                    highlight: *highlight,
                })
            }
            ParticleStyleSpec::Sprites {
                seed,
                frames,
                size,
                padded,
                mode,
            } => {
                if frames.is_empty() || frames.len() > MAX_ATLAS_FRAMES {
                    self.limit(path, "frames", "Sprite atlases take 1 to 64 frames");
                    return None;
                }
                let mut budget = motion_core::budget::Budget::new(
                    PixelSize::new(1, 1),
                    RasterLimits {
                        max_bytes: ATLAS_BYTES,
                        ..Default::default()
                    },
                )
                .map_err(|error| self.error(path, "frames", error.to_string()))
                .ok()?;
                for frame in frames {
                    if !(2..=MAX_SPRITE_SIDE).contains(&frame.size) {
                        self.limit(path, "frames.size", "Sprite sides must be in 2..=256");
                        return None;
                    }
                    budget
                        .bytes(u64::from(frame.size).pow(2) * SPRITE_TEXEL_BYTES, path)
                        .map_err(|error| self.error(path, "frames", error.to_string()))
                        .ok()?;
                }
                let mut images = Vec::with_capacity(frames.len());
                for (index, frame) in frames.iter().enumerate() {
                    let here = format!("{path}.frames[{index}]");
                    let material = match frame.material {
                        ParticleMaterialSpec::SoftCircle => ProceduralSprite::SoftCircle,
                        ParticleMaterialSpec::Smoke { scale, evolution } => {
                            ProceduralSprite::Smoke {
                                scale,
                                evolution,
                                fractal: Default::default(),
                            }
                        }
                        ParticleMaterialSpec::Star => ProceduralSprite::Star,
                        ParticleMaterialSpec::Ring => ProceduralSprite::Ring,
                        ParticleMaterialSpec::FireCore => ProceduralSprite::FireCore,
                    };
                    let time = self.seconds(frame.time, &here, "time")?.after_origin();
                    let limits = RasterLimits {
                        max_bytes: ATLAS_BYTES,
                        ..Default::default()
                    };
                    let image = material
                        .generate_at(frame.size, *seed, time, limits)
                        .map_err(|error| self.error(&here, "material", error.to_string()))
                        .ok()?;
                    images.push(image);
                }
                let atlas = if *padded {
                    SpriteAtlas::new_padded(&images, ATLAS_BYTES)
                } else {
                    SpriteAtlas::new(&images, ATLAS_BYTES)
                };
                let mut atlas = atlas
                    .map_err(|error| self.error(path, "frames", error.to_string()))
                    .ok()?;
                atlas.sheet.mode = match *mode {
                    ParticleSheetSpec::OverLifetime => SpriteSheetMode::OverLifetime,
                    ParticleSheetSpec::Fps(fps) => {
                        self.bounded(fps, 0.0, 10000.0, path, "mode.fps")?;
                        SpriteSheetMode::Fps(fps)
                    }
                    ParticleSheetSpec::Random(seed) => SpriteSheetMode::Random(seed),
                    ParticleSheetSpec::Custom => SpriteSheetMode::Custom,
                };
                atlas
                    .style(*size, false)
                    .map_err(|error| self.error(path, "size", error.to_string()))
                    .ok()
            }
            ParticleStyleSpec::NodeCircles {
                radius,
                amplitude,
                frequency,
                fill,
            } => {
                let radius = self.bounded(*radius, 0.0, 4096.0, path, "radius")?;
                let amplitude = self.bounded(*amplitude, -radius, radius, path, "amplitude")?;
                let frequency = self.bounded(*frequency, -1e6, 1e6, path, "frequency")?;
                let fill = *fill;
                Some(ParticleStyle::Custom(Arc::new(move |particles, _| {
                    let nodes = particles
                        .iter()
                        .map(|particle| {
                            Circle::new(
                                Point::new(
                                    f64::from(particle.position[0]),
                                    f64::from(particle.position[1]),
                                ),
                                radius
                                    + motion_core::math::sin(f64::from(particle.age) * frequency)
                                        * amplitude,
                            )
                            .map(|circle| circle.fill(fill).into())
                        })
                        .collect::<Result<Vec<Node>, _>>()?;
                    Ok(Node::group(nodes))
                })))
            }
            _ => None,
        }
    }
}
