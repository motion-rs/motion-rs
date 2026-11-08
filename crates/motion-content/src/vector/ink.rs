//! Deterministic ink materials and pressure-sensitive drawing.

use std::io;

mod brush;
mod lettering;

pub use lettering::StrokeFont;

pub use brush::{
    BrushCap, BrushPreset, BrushStroke, BrushTiming, InkFill, InputSample, NibMode, Paper,
    PigmentMix, PigmentPainting,
};

type Point = (f64, f64);

/// A deterministic ink material for strokes and even-odd path fills.
///
/// [`BrushStroke::ink`] uses all effects. [`InkFill`] also uses
/// bounded wash, rough edges and pooling; only directional fibers are stroke-only.
///
/// The default is solid ink with seed zero and no dry gaps, feathering, roughness,
/// granulation, directional fibers, paper imprint, or edge pooling. Parameters
/// use layer-local coordinates (except canvas-anchored paper) and are evaluated from
/// geometry, not previous frames. This is a bounded procedural wash, not a fluid
/// or paper-fiber simulation. Both rendering backends share the CPU coverage.
///
/// Builders store values; invalid settings are reported during rendering. Use a
/// stable seed and vary bleed or pigment settings explicitly with frame-local time.
///
/// # Example
///
/// ```
/// use motion_content::prelude::*;
/// use motion_core::prelude::*;
/// use motion_core::{
///     DisplayList, RasterLimits, ResourcePool, ValidatedDisplayList, MAX_PATH_VERBS,
///     MAX_STROKE_VERBS, MAX_SVG_PATH_BYTES, sample_offsets,
/// };
/// use motion_render::{Composition, RenderPrepare, RenderOptions, Renderer, render_still};
/// use motion_content::vector::ink::*;
/// # fn main() -> motion_core::Result {
/// let brush = Ink::new(17)
///     .dryness(0.25)
///     .fibers(1.0)
///     .bleed(4.0)
///     .edge_roughness(1.5)
///     .granulation(0.4)
///     .paper(211, 0.45, 1.5)
///     .edge_pooling(0.6, 2.0);
/// let path = Path2D::parse_svg("M0 0L60 0")?;
/// let stroke = BrushStroke::new(path, 24.0, Color::from_hex(0x23272c))
///     .pressure(0.15, 1.2, 0.05)
///     .ink_load(1.0, 0.25)
///     .ink(brush);
/// # let _ = stroke;
/// # Ok(())
/// # }
/// ```
#[derive(Clone, Copy, Debug, Default)]
pub struct Ink {
    pub(super) seed: u32,
    pub(super) dryness: f32,
    pub(super) bleed: f32,
    pub(super) roughness: f32,
    pub(super) granulation: f32,
    fibers: f32,
    paper: Option<(u32, f32, f32)>,
    pooling: Option<(f32, f32)>,
}

impl Ink {
    /// Creates solid ink with a fixed seed and all effects initially disabled.
    pub fn new(seed: u32) -> Self {
        Self {
            seed,
            ..Self::default()
        }
    }

    /// Sets dry-brush strength in `0..=1`; one permits fully dry gaps.
    /// Zero retains solid coverage unless other material effects are enabled.
    pub fn dryness(mut self, strength: f32) -> Self {
        self.dryness = strength;
        self
    }

    /// Sets the maximum soft wash extent beyond a fill or stroke edge, in local pixels.
    /// Must be finite and nonnegative. Zero disables feathering. The halo has
    /// finite support, is affected by the layer transform, and respects clips.
    /// Animate this value to suggest absorption without retaining frame history.
    pub fn bleed(mut self, radius: f32) -> Self {
        self.bleed = radius;
        self
    }

    /// Sets the maximum inward/outward edge displacement in local pixels.
    /// Must be finite and nonnegative. Seeded low-frequency noise shapes the edge;
    /// zero preserves the geometric silhouette. It is not a blur radius.
    pub fn edge_roughness(mut self, amplitude: f32) -> Self {
        self.roughness = amplitude;
        self
    }

    /// Sets pigment variation in `0..=1`, from uniform to strongly mottled.
    /// This attenuates coverage, not RGB channels or the composition background.
    /// Coarse local noise produces density variation without per-frame randomness.
    pub fn granulation(mut self, strength: f32) -> Self {
        self.granulation = strength;
        self
    }

    /// Blends axis-oriented dry gaps towards path-following fibers in `0..=1`.
    /// One uses full-path arc length and signed distance across the sampled
    /// flattened segment. Requires dryness or depleted [`BrushStroke::ink_load`]
    /// to expose gaps. Trim preserves the full-path phase. Changing geometry can
    /// change arc length; corners, closed seams, and crossings are not simulated
    /// bristle transport. Zero retains the original local-axis texture.
    pub fn fibers(mut self, strength: f32) -> Self {
        self.fibers = strength;
        self
    }

    /// Imprints seeded paper fibers into pigment coverage, not the background.
    /// Strength must be in `0..=1`, scale finite and positive in output pixels.
    /// Paper is sampled in canvas coordinates, so moving/scaling a layer does not
    /// move the paper. Reuse seed and scale on fills and strokes for one sheet.
    /// This attenuates alpha only; it is not a paper height or fluid simulation.
    pub fn paper(mut self, seed: u32, strength: f32, scale: f32) -> Self {
        self.paper = Some((seed, strength, scale));
        self
    }

    /// Adds an inner edge deposit by diluting the body relative to the rim.
    /// Strength must be in `0..=1`; width must be finite and positive local pixels.
    /// Applied to the fill boundary or stroke union, not individual segment caps. Does not
    /// expand bounds, change RGB, fill completely dry gaps, or simulate drying.
    pub fn edge_pooling(mut self, strength: f32, width: f32) -> Self {
        self.pooling = Some((strength, width));
        self
    }

    pub(crate) fn padding(self) -> io::Result<f32> {
        let padding = self.bleed + self.roughness;
        if ![
            self.dryness,
            self.bleed,
            self.roughness,
            self.granulation,
            self.fibers,
            padding,
        ]
        .iter()
        .all(|value| value.is_finite())
            || !(0.0..=1.0).contains(&self.dryness)
            || !(0.0..=1.0).contains(&self.granulation)
            || !(0.0..=1.0).contains(&self.fibers)
            || self.paper.is_some_and(|(_, strength, scale)| {
                !strength.is_finite()
                    || !(0.0..=1.0).contains(&strength)
                    || !scale.is_finite()
                    || scale <= 0.0
            })
            || self.pooling.is_some_and(|(strength, width)| {
                !strength.is_finite()
                    || !(0.0..=1.0).contains(&strength)
                    || !width.is_finite()
                    || width <= 0.0
            })
            || self.bleed < 0.0
            || self.roughness < 0.0
        {
            return Err(invalid(
                "Ink strengths must be in 0..=1, bleed/roughness nonnegative, and paper/pooling scales positive and finite",
            ));
        }
        Ok(padding)
    }

    #[cfg(test)]
    pub(super) fn has_fill_edges(self) -> bool {
        self.bleed > 0.0
            || self.roughness > 0.0
            || self.pooling.is_some_and(|(strength, _)| strength > 0.0)
    }

    pub(super) fn sample(self, point: Point, canvas: Point) -> InkSample {
        let dry = (self.dryness > 0.0).then(|| DrySample::new(point, self.seed));
        let mut density = dry.map_or(1.0, |dry| dry.coverage(f64::from(self.dryness)));
        let mut pigment = 1.0;
        if self.granulation > 0.0 {
            let cloud = noise((point.0 * 0.08, point.1 * 0.08), self.seed.wrapping_add(43));
            let grain = noise((point.0 * 0.65, point.1 * 0.65), self.seed.wrapping_add(71));
            let factor =
                1.0 - f64::from(self.granulation) * (0.2 + 0.8 * (cloud * 0.7 + grain * 0.3));
            density *= factor;
            pigment *= factor;
        }
        if let Some((seed, strength, scale)) = self.paper {
            let paper = (canvas.0 / f64::from(scale), canvas.1 / f64::from(scale));
            let horizontal = noise((paper.0 * 0.08, paper.1 * 2.8), seed);
            let vertical = noise((paper.0 * 2.5, paper.1 * 0.12), seed.wrapping_add(101));
            let grain = noise((paper.0 * 0.7, paper.1 * 0.7), seed.wrapping_add(137));
            let factor = 1.0
                - f64::from(strength)
                    * (0.15 + 0.85 * (horizontal * 0.5 + vertical * 0.3 + grain * 0.2));
            density *= factor;
            pigment *= factor;
        }
        InkSample {
            edge: if self.roughness > 0.0 {
                f64::from(self.roughness)
                    * (noise((point.0 * 0.22, point.1 * 0.22), self.seed.wrapping_add(29)) * 2.0
                        - 1.0)
            } else {
                0.0
            },
            bleed: if self.bleed > 0.0 {
                f64::from(self.bleed)
                    * (0.65
                        + 0.35
                            * noise((point.0 * 0.35, point.1 * 0.35), self.seed.wrapping_add(61)))
            } else {
                0.0
            },
            density,
            pigment,
            point,
            ink: Some(self),
            dry,
        }
    }
}

#[derive(Clone, Copy)]
pub(super) struct InkSample {
    edge: f64,
    bleed: f64,
    density: f64,
    pigment: f64,
    point: Point,
    ink: Option<Ink>,
    dry: Option<DrySample>,
}

impl Default for InkSample {
    fn default() -> Self {
        Self {
            edge: 0.0,
            bleed: 0.0,
            density: 1.0,
            pigment: 1.0,
            point: (0.0, 0.0),
            ink: None,
            dry: None,
        }
    }
}

impl InkSample {
    #[cfg(test)]
    pub(super) fn fill_coverage(self, coverage: f64) -> f64 {
        coverage * self.density
    }

    pub(super) fn fill_edge_coverage(self, distance: f64, metric: f64) -> f64 {
        self.coverage(distance, metric) * (self.density * self.pool_density(distance))
    }

    fn pool_density(self, distance: f64) -> f64 {
        self.ink
            .and_then(|ink| ink.pooling)
            .map_or(1.0, |(strength, width)| {
                let rim = (1.0 - (distance - self.edge).abs() / f64::from(width)).clamp(0.0, 1.0);
                1.0 - f64::from(strength) * 0.45 * (1.0 - rim * rim)
            })
    }

    pub(super) fn coverage(self, distance: f64, metric: f64) -> f64 {
        let distance = distance - self.edge;
        let core = (0.5 - distance / metric).clamp(0.0, 1.0);
        // A finite-support wash leaves no tails beyond the validated local padding.
        let halo = if self.bleed > 0.0 {
            let remaining = (1.0 - distance.max(0.0) / self.bleed).clamp(0.0, 1.0);
            remaining * remaining * 0.35
        } else {
            0.0
        };
        core + (1.0 - core) * halo
    }

    pub(super) fn shade(self, coverage: f64, distance: f64, fiber: Point, load: f64) -> f64 {
        if load <= 0.0 {
            return 0.0;
        }
        let mut density = self.density;
        if let Some(ink) = self.ink {
            if ink.fibers > 0.0 || load < 1.0 {
                let dryness = 1.0 - (1.0 - f64::from(ink.dryness)) * load;
                let isotropic = self
                    .dry
                    .unwrap_or_else(|| DrySample::new(self.point, ink.seed))
                    .coverage(dryness);
                let dry = if ink.fibers > 0.0 {
                    let directional = dry_coverage(fiber, ink.seed, dryness);
                    isotropic + (directional - isotropic) * f64::from(ink.fibers)
                } else {
                    isotropic
                };
                density = dry * self.pigment;
            }
            density *= self.pool_density(distance);
        }
        coverage * density * load
    }
}

fn invalid(message: &str) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidInput, message)
}

fn dry_coverage(point: Point, seed: u32, dryness: f64) -> f64 {
    DrySample::new(point, seed).coverage(dryness)
}

#[derive(Clone, Copy)]
struct DrySample {
    grain: f64,
    fiber: f64,
}

impl DrySample {
    fn new(point: Point, seed: u32) -> Self {
        Self {
            grain: noise((point.0 * 1.3, point.1 * 1.3), seed),
            fiber: noise((point.0 * 0.15, point.1 * 3.0), seed.wrapping_add(17)),
        }
    }

    fn coverage(self, dryness: f64) -> f64 {
        let density =
            ((self.fiber - dryness * 0.7) * 8.0).clamp(0.0, 1.0) * (0.5 + 0.5 * self.grain);
        1.0 - dryness + dryness * density
    }
}

/// Lattice value noise in 0..=1, stateless so effects stay attached to the path.
fn noise(point: Point, seed: u32) -> f64 {
    motion_core::noise::value2([point.0, point.1], seed) * 0.5 + 0.5
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fill_edges_have_bounded_wash_and_relative_rim_density() {
        let point = (13.5, 24.5);
        let ink = Ink::new(17).bleed(4.0).edge_pooling(1.0, 2.0);
        assert!(ink.has_fill_edges());
        assert!(
            !Ink::new(17)
                .fibers(1.0)
                .edge_pooling(0.0, 2.0)
                .has_fill_edges()
        );
        let sample = ink.sample(point, point);
        assert!(sample.fill_edge_coverage(1.0, 1.0) > 0.0);
        assert_eq!(sample.fill_edge_coverage(5.0, 1.0), 0.0);
        assert!(sample.fill_edge_coverage(-0.5, 1.0) > sample.fill_edge_coverage(-5.0, 1.0));
        let fibers = ink.fibers(1.0).sample(point, point);
        assert_eq!(
            sample.fill_edge_coverage(-0.5, 1.0),
            fibers.fill_edge_coverage(-0.5, 1.0)
        );
    }

    #[test]
    fn fill_density_preserves_coverage_and_is_stateless() {
        let point = (13.5, 24.5);
        let neutral = Ink::new(17).sample(point, point);
        let ink = Ink::new(17)
            .dryness(0.4)
            .granulation(0.6)
            .paper(211, 0.7, 1.5);
        for coverage in [0.0, 0.25, 0.5, 0.75, 1.0] {
            assert_eq!(neutral.fill_coverage(coverage), coverage);
            let first = ink.sample(point, point).fill_coverage(coverage);
            let _ = ink.sample((40.5, 8.5), (8.5, 40.5));
            assert_eq!(first, ink.sample(point, point).fill_coverage(coverage));
            assert!(first >= 0.0 && first <= coverage);
        }
        assert!(ink.sample(point, point).fill_coverage(1.0) < 1.0);
        let paper = Ink::new(17).paper(211, 0.7, 1.5);
        assert_eq!(
            paper.sample(point, point).fill_coverage(1.0),
            paper.sample((90.0, 12.0), point).fill_coverage(1.0)
        );
    }

    #[test]
    fn neutral_fibers_preserve_reference_dry_coverage() {
        for point in [(2.5, 3.5), (15.5, 9.5), (37.5, 24.5)] {
            let material = Ink::new(17).dryness(0.7).sample(point, point);
            for distance in [-4.0, -0.2, 0.3, 2.0] {
                let expected = (0.5_f64 - distance).clamp(0.0, 1.0)
                    * dry_coverage(point, 17, f64::from(0.7_f32));
                assert_eq!(
                    material.shade(material.coverage(distance, 1.0), distance, (0.0, 0.0), 1.0),
                    expected
                );
            }
        }
    }

    #[test]
    fn paper_is_canvas_anchored_and_pooling_keeps_a_darker_rim() {
        let paper = Ink::new(17).paper(211, 0.7, 1.5);
        let canvas = (13.5, 24.5);
        assert_eq!(
            paper.sample((0.0, 0.0), canvas).density,
            paper.sample((90.0, 12.0), canvas).density
        );
        assert_eq!(
            paper.sample((0.0, 0.0), canvas).density,
            Ink::new(99)
                .paper(211, 0.7, 1.5)
                .sample((0.0, 0.0), canvas)
                .density
        );
        assert!((1..20).any(|offset| {
            paper
                .sample((0.0, 0.0), (canvas.0 + f64::from(offset), canvas.1))
                .density
                != paper.sample((0.0, 0.0), canvas).density
        }));
        let pooled = Ink::new(17).edge_pooling(1.0, 2.0).sample(canvas, canvas);
        assert!(
            pooled.shade(1.0, -0.5, (0.0, 0.0), 1.0) > pooled.shade(1.0, -5.0, (0.0, 0.0), 1.0)
        );
        assert_eq!(pooled.shade(1.0, -0.5, (0.0, 0.0), 0.0), 0.0);
    }
}
