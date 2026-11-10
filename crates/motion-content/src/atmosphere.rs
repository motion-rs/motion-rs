//! Deterministic advected fog, sparse dust and HDR light-leak image generators.
//!
//! Generated assets use the ordinary CPU/GPU image path. Time and global pixel
//! coordinates are explicit; these are 2D overlays, not volumetric scattering.
//!
//! ```
//! use motion_content::{Image, atmosphere::Atmosphere};
//! use motion_core::{BlendMode, Node, NodeModifiers, PixelSize, RasterLimits, TimeLiteral};
//! let asset = Atmosphere::light_leak(42).render(
//!     PixelSize::new(32, 24), 1.seconds(), RasterLimits::sandbox(),
//! )?;
//! let overlay = Node::from(Image::from_asset(std::sync::Arc::new(asset)))
//!     .blend_mode(BlendMode::Plus);
//! # let _ = overlay;
//! # Ok::<(), motion_core::Error>(())
//! ```

use motion_core::{
    Color, PixelSize, RasterLimits, Time, ValidationError,
    budget::Budget,
    noise::{self, Fractal},
};

use crate::ImageAsset;

/// Spatial envelope applied to shared fractal noise.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum AtmosphereKind {
    /// Soft fog with top-to-bottom nonnegative density multipliers.
    Fog {
        /// Density at the top and bottom of the full canvas, in 0..=8.
        gradient: [f64; 2],
    },
    /// Sparse soft discs, one possible dust mote per noise cell.
    Dust {
        /// Maximum mote radius in pixels, at most half the cell scale.
        radius: f64,
        /// Probability of a mote in each cell, in 0..=1.
        coverage: f64,
    },
    /// Elliptical edge-light envelope multiplied by moving fractal noise.
    LightLeak {
        /// Center in normalized full-canvas coordinates, in -1..=2.
        center: [f64; 2],
        /// Positive elliptical extents in canvas fractions, at most 4.
        extent: [f64; 2],
    },
}

/// Editable bounded recipe; sampling retains no playback or random state.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Atmosphere {
    /// Envelope and effect-specific parameters.
    pub kind: AtmosphereKind,
    /// Explicit noise and mote seed.
    pub seed: u32,
    /// Encoded-sRGB tint, converted once to linear light.
    pub color: Color,
    /// Alpha/density multiplier, in 0..=1.
    pub opacity: f64,
    /// Straight-linear RGB multiplier, in 0..=64; values above one are HDR.
    pub intensity: f64,
    /// Base noise wavelength / dust-cell spacing in pixels, in 0.25..=1000000.
    pub scale: f64,
    /// Pattern velocity in pixels per second.
    pub drift: [f64; 2],
    /// Shared amplitude-normalized fBm parameters, with at most eight octaves.
    pub fractal: Fractal,
}

impl Atmosphere {
    /// A pale, low-contrast fog layer, denser near the bottom edge.
    pub fn fog(seed: u32) -> Self {
        Self {
            kind: AtmosphereKind::Fog {
                gradient: [0.15, 1.0],
            },
            seed,
            color: Color::rgb(220, 233, 230),
            opacity: 0.45,
            intensity: 1.0,
            scale: 96.0,
            drift: [6.0, -2.0],
            fractal: Fractal::default(),
        }
    }

    /// Sparse warm HDR dust, suitable for Plus or Screen composition.
    pub fn dust(seed: u32) -> Self {
        Self {
            kind: AtmosphereKind::Dust {
                radius: 2.0,
                coverage: 0.3,
            },
            seed,
            color: Color::rgb(255, 238, 209),
            opacity: 0.65,
            intensity: 4.0,
            scale: 24.0,
            drift: [5.0, -12.0],
            fractal: Fractal::default(),
        }
    }

    /// Warm HDR light entering from the left edge.
    pub fn light_leak(seed: u32) -> Self {
        Self {
            kind: AtmosphereKind::LightLeak {
                center: [0.0, 0.5],
                extent: [0.3, 0.65],
            },
            seed,
            color: Color::rgb(255, 100, 32),
            opacity: 0.8,
            intensity: 3.0,
            scale: 128.0,
            drift: [2.0, 1.0],
            fractal: Fractal::default(),
        }
    }

    /// Validates the full recipe before noise evaluation or allocation.
    pub fn validate(&self) -> Result<(), ValidationError> {
        self.fractal.validate()?;
        let bounded =
            |value: f64, min: f64, max: f64| value.is_finite() && (min..=max).contains(&value);
        let valid_kind = match self.kind {
            AtmosphereKind::Fog { gradient } => {
                gradient.into_iter().all(|value| bounded(value, 0.0, 8.0))
            }
            AtmosphereKind::Dust { radius, coverage } => {
                radius.is_finite()
                    && radius > 0.0
                    && radius <= self.scale * 0.5
                    && bounded(coverage, 0.0, 1.0)
            }
            AtmosphereKind::LightLeak { center, extent } => {
                center.into_iter().all(|value| bounded(value, -1.0, 2.0))
                    && extent
                        .into_iter()
                        .all(|value| value.is_finite() && value > 0.0 && value <= 4.0)
            }
        };
        if !valid_kind
            || !bounded(self.opacity, 0.0, 1.0)
            || !bounded(self.intensity, 0.0, 64.0)
            || !bounded(self.scale, 0.25, 1e6)
            || self.drift.into_iter().any(|value| !value.is_finite())
        {
            return Err(ValidationError::invalid(
                "atmosphere",
                "parameters",
                "Require finite bounded envelope, opacity, intensity, scale and drift",
            ));
        }
        Ok(())
    }

    /// Blend mode that composes this recipe without darkening: Screen for fog, Plus for HDR light.
    pub fn blend_mode(&self) -> motion_core::BlendMode {
        match self.kind {
            AtmosphereKind::Fog { .. } => motion_core::BlendMode::Screen,
            AtmosphereKind::Dust { .. } | AtmosphereKind::LightLeak { .. } => {
                motion_core::BlendMode::Plus
            }
        }
    }

    /// Renders the recipe at an explicit time into a bilinear-sampled node using
    /// [`Self::blend_mode`]. The node has the pixel size as its design size, so it
    /// fills a canvas of the same dimensions.
    pub fn overlay(
        &self,
        size: PixelSize,
        time: Time,
        limits: RasterLimits,
    ) -> motion_core::Result<motion_core::Node> {
        use motion_core::NodeModifiers;
        let asset = std::sync::Arc::new(self.render(size, time, limits)?);
        Ok(motion_core::Node::from(
            crate::Image::from_asset(asset).sampling(motion_core::Sampling::Bilinear),
        )
        .blend_mode(self.blend_mode()))
    }

    /// Generates a full image at explicit time after pixel, byte and work admission.
    pub fn render(
        &self,
        size: PixelSize,
        time: Time,
        limits: RasterLimits,
    ) -> motion_core::Result<ImageAsset> {
        self.render_region(size, [0, 0], size, time, limits)
    }

    /// Generates a base-level region using full-canvas envelope and global pixels.
    /// Adjacent regions match full-image base texels exactly; mipmaps are regional.
    pub fn render_region(
        &self,
        canvas: PixelSize,
        origin: [u32; 2],
        size: PixelSize,
        time: Time,
        limits: RasterLimits,
    ) -> motion_core::Result<ImageAsset> {
        self.validate()?;
        let valid_size = |size: PixelSize| {
            size.width > 0 && size.height > 0 && size.width <= 16384 && size.height <= 16384
        };
        if !valid_size(canvas)
            || !valid_size(size)
            || u64::from(origin[0]) + u64::from(size.width) > u64::from(canvas.width)
            || u64::from(origin[1]) + u64::from(size.height) > u64::from(canvas.height)
        {
            return Err(ValidationError::invalid(
                "atmosphere",
                "region",
                "Require a positive bounded region inside a 1..=16384 canvas",
            )
            .into());
        }
        let pixels = u64::from(size.width) * u64::from(size.height);
        if pixels > 16_777_216 {
            return Err(ValidationError::limit(
                "atmosphere",
                "max_pixels",
                "Image source exceeds 16777216 pixels",
            )
            .into());
        }
        let phase = self.drift.map(|velocity| -velocity * time.as_seconds());
        for horizontal in [origin[0], origin[0] + size.width - 1] {
            for vertical in [origin[1], origin[1] + size.height - 1] {
                let point = [
                    (f64::from(horizontal) + 0.5 + phase[0]) / self.scale,
                    (f64::from(vertical) + 0.5 + phase[1]) / self.scale,
                ];
                noise::fbm(|_| 0.0, point, self.fractal)?;
            }
        }
        let mut mip_pixels = pixels;
        let mut mip_size = size;
        while mip_size.width > 1 || mip_size.height > 1 {
            mip_size = PixelSize::new(mip_size.width.div_ceil(2), mip_size.height.div_ceil(2));
            mip_pixels += u64::from(mip_size.width) * u64::from(mip_size.height);
        }
        let mut budget = Budget::new(size, limits)?;
        budget.bytes(pixels * 32 + mip_pixels * 16, "atmosphere")?;
        budget.work(
            pixels * (u64::from(self.fractal.octaves) * 192 + 256) + mip_pixels * 128,
            "atmosphere",
        )?;
        let tint = self.color.to_linear_premultiplied();
        let alpha = tint[3];
        let rgb = [tint[0], tint[1], tint[2]].map(|value| {
            if alpha > 0.0 {
                value / alpha * self.intensity
            } else {
                0.0
            }
        });
        let mut values = Vec::new();
        values.try_reserve_exact(pixels as usize).map_err(|_| {
            ValidationError::limit(
                "atmosphere",
                "max_bytes",
                "Could not reserve generated pixels",
            )
        })?;
        for row in 0..size.height {
            for column in 0..size.width {
                let pixel = [
                    f64::from(origin[0] + column) + 0.5,
                    f64::from(origin[1] + row) + 0.5,
                ];
                let point = [
                    (pixel[0] + phase[0]) / self.scale,
                    (pixel[1] + phase[1]) / self.scale,
                ];
                let cloud = (0.5
                    + 0.5
                        * noise::fbm(
                            |point| noise::perlin2(point, self.seed),
                            point,
                            self.fractal,
                        )?)
                .clamp(0.0, 1.0);
                let envelope = match self.kind {
                    AtmosphereKind::Fog { gradient } => {
                        let vertical = pixel[1] / f64::from(canvas.height);
                        let density = gradient[0] * (1.0 - vertical) + gradient[1] * vertical;
                        1.0 - libm::exp(-self.opacity * density * (0.35 + cloud))
                    }
                    AtmosphereKind::Dust { radius, coverage } => {
                        self.opacity
                            * dust(point, self.seed, radius / self.scale, coverage)
                            * (0.7 + 0.3 * cloud)
                    }
                    AtmosphereKind::LightLeak { center, extent } => {
                        let horizontal =
                            (pixel[0] / f64::from(canvas.width) - center[0]) / extent[0];
                        let vertical =
                            (pixel[1] / f64::from(canvas.height) - center[1]) / extent[1];
                        self.opacity
                            * libm::exp(-2.0 * (horizontal * horizontal + vertical * vertical))
                            * (0.6 + 0.4 * cloud)
                    }
                };
                values.push([rgb[0], rgb[1], rgb[2], (envelope * alpha).clamp(0.0, 1.0)]);
            }
        }
        ImageAsset::from_rgba_float(size.width, size.height, values)
    }
}

fn dust(point: [f64; 2], seed: u32, radius: f64, coverage: f64) -> f64 {
    let cell = point.map(|value| value.floor() as i32);
    let unit = |value: u32| f64::from(value) / f64::from(u32::MAX);
    let mut result: f64 = 0.0;
    for dy in -1i32..=1 {
        for dx in -1i32..=1 {
            let horizontal = cell[0].wrapping_add(dx);
            let vertical = cell[1].wrapping_add(dy);
            let hash = noise::hash2(horizontal, vertical, seed);
            if unit(hash) >= coverage {
                continue;
            }
            let center = [
                f64::from(horizontal) + unit(noise::hash32(hash ^ 0xa511_e9b3)),
                f64::from(vertical) + unit(noise::hash32(hash ^ 0x63d8_3595)),
            ];
            let radius = radius * (0.6 + 0.4 * unit(noise::hash32(hash ^ 0x9e37_79b9)));
            let distance = (point[0] - center[0]).powi(2) + (point[1] - center[1]).powi(2);
            result = result.max((1.0 - distance / (radius * radius)).max(0.0).powi(2));
        }
    }
    result
}

#[cfg(test)]
mod tests {
    use super::*;
    use motion_core::Shape;
    use motion_core::{BlendMode, Node, NodeModifiers, Rect, Size, TimeLiteral};
    use motion_render::{Composition, Renderer};

    fn compose(
        recipe: Atmosphere,
        mode: BlendMode,
        renderer: Renderer,
        time: Time,
    ) -> motion_core::Result<motion_render::Snapshot> {
        let size = PixelSize::new(64, 48);
        let image = std::sync::Arc::new(recipe.render(size, time, RasterLimits::default())?);
        Composition::new(64, 48, motion_render::FrameClock::new(30, 150))
            .with_options(motion_render::RenderOptions::default().backend(renderer.into()))?
            .render_frame(0, Size::new(64.0, 48.0), |_| {
                Node::group([
                    Rect::new(0.0, 0.0, 64.0, 48.0)
                        .fill(Color::rgb(18, 38, 45))
                        .into(),
                    Node::from(
                        crate::Image::from_asset(image.clone())
                            .sampling(motion_core::Sampling::Bilinear),
                    )
                    .blend_mode(mode),
                ])
            })
    }

    #[test]
    fn overlay_helper_matches_manual_composition_and_picks_blend_modes() -> motion_core::Result {
        assert_eq!(Atmosphere::fog(1).blend_mode(), BlendMode::Screen);
        assert_eq!(Atmosphere::dust(1).blend_mode(), BlendMode::Plus);
        assert_eq!(Atmosphere::light_leak(1).blend_mode(), BlendMode::Plus);
        let size = PixelSize::new(64, 48);
        for recipe in [Atmosphere::fog(7), Atmosphere::light_leak(7)] {
            let overlay = recipe.overlay(size, 1.seconds(), RasterLimits::default())?;
            let frame = Composition::new(64, 48, motion_render::FrameClock::new(30, 150))
                .with_options(
                    motion_render::RenderOptions::default().backend(Renderer::Cpu.into()),
                )?
                .render_frame(0, Size::new(64.0, 48.0), |_| {
                    Node::group([
                        Rect::new(0.0, 0.0, 64.0, 48.0)
                            .fill(Color::rgb(18, 38, 45))
                            .into(),
                        overlay.clone(),
                    ])
                })?;
            let manual = compose(recipe, recipe.blend_mode(), Renderer::Cpu, 1.seconds())?;
            assert_eq!(frame.pixels(), manual.pixels());
        }
        let invalid = Atmosphere {
            opacity: 2.0,
            ..Atmosphere::fog(1)
        };
        assert!(
            invalid
                .overlay(size, 0.seconds(), RasterLimits::default())
                .is_err()
        );
        Ok(())
    }

    #[test]
    fn generated_overlays_compose_on_cpu() -> motion_core::Result {
        for recipe in [
            Atmosphere::fog(42),
            Atmosphere::dust(42),
            Atmosphere::light_leak(42),
        ] {
            for mode in [BlendMode::Plus, BlendMode::Screen] {
                let frame = compose(recipe, mode, Renderer::Cpu, 0.seconds())?;
                assert_eq!(
                    frame.pixels(),
                    compose(recipe, mode, Renderer::Cpu, 0.seconds())?.pixels()
                );
                assert_ne!(
                    frame.pixels(),
                    compose(recipe, mode, Renderer::Cpu, 1.seconds())?.pixels()
                );
                assert!(
                    frame
                        .pixels()
                        .windows(2)
                        .any(|pixels| pixels[0] != pixels[1])
                );
            }
        }
        Ok(())
    }

    #[test]
    #[ignore = "requires native GPU; atmosphere image/composite parity, no timing"]
    fn generated_overlays_cpu_gpu_parity() -> motion_core::Result {
        for recipe in [
            Atmosphere::fog(42),
            Atmosphere::dust(42),
            Atmosphere::light_leak(42),
        ] {
            for mode in [BlendMode::Plus, BlendMode::Screen] {
                let cpu = compose(recipe, mode, Renderer::Cpu, 1.seconds())?;
                let gpu = compose(recipe, mode, Renderer::Gpu, 1.seconds())?;
                assert_eq!(
                    gpu.pixels(),
                    compose(recipe, mode, Renderer::Gpu, 1.seconds())?.pixels()
                );
                for (cpu, gpu) in cpu.pixels().iter().zip(gpu.pixels()) {
                    for (cpu, gpu) in cpu.to_le_bytes().into_iter().zip(gpu.to_le_bytes()) {
                        assert!(
                            cpu.abs_diff(gpu) <= 1,
                            "{:?} {mode:?}: {cpu} != {gpu}",
                            recipe.kind
                        );
                    }
                }
            }
        }
        Ok(())
    }

    #[test]
    fn presets_replay_animate_and_regions_match_full_base_pixels() -> motion_core::Result {
        let size = PixelSize::new(65, 49);
        for mut recipe in [
            Atmosphere::fog(42),
            Atmosphere::dust(42),
            Atmosphere::light_leak(42),
        ] {
            if matches!(recipe.kind, AtmosphereKind::Dust { .. }) {
                recipe.kind = AtmosphereKind::Dust {
                    radius: 3.0,
                    coverage: 1.0,
                };
                recipe.scale = 12.0;
            }
            let time = 0.seconds();
            let full = recipe.render(size, time, RasterLimits::default())?;
            assert_eq!(full, recipe.render(size, time, RasterLimits::default())?);
            assert_ne!(
                full,
                recipe.render(size, 1.seconds(), RasterLimits::default())?
            );
            let region = recipe.render_region(
                size,
                [17, 11],
                PixelSize::new(31, 23),
                time,
                RasterLimits::default(),
            )?;
            let mut energy = 0.0;
            for row in 0..size.height {
                for column in 0..size.width {
                    let pixel = full.levels()[0].pixel(column, row).unwrap();
                    assert!(pixel.into_iter().all(f32::is_finite));
                    assert!((0.0..=1.0).contains(&pixel[3]));
                    energy += f64::from(pixel[0]);
                }
            }
            assert!(energy > 0.0, "{:?}", recipe.kind);
            for row in 0..23 {
                for column in 0..31 {
                    assert_eq!(
                        region.levels()[0].pixel(column, row),
                        full.levels()[0].pixel(column + 17, row + 11)
                    );
                }
            }
            recipe.seed += 1;
            assert_ne!(full, recipe.render(size, time, RasterLimits::default())?);
        }
        Ok(())
    }

    #[test]
    fn admission_rejects_bad_parameters_regions_and_budgets() {
        let size = PixelSize::new(8, 8);
        let time = 0.seconds();
        let base = Atmosphere::fog(42);
        for limits in [
            RasterLimits {
                max_pixels: 63,
                ..RasterLimits::default()
            },
            RasterLimits {
                max_bytes: 1,
                ..RasterLimits::default()
            },
            RasterLimits {
                max_work: 1,
                ..RasterLimits::default()
            },
        ] {
            assert!(base.render(size, time, limits).is_err());
        }
        assert!(
            base.render(PixelSize::new(0, 1), time, RasterLimits::default())
                .is_err()
        );
        assert!(
            base.render_region(size, [8, 0], size, time, RasterLimits::default())
                .is_err()
        );
        for recipe in [
            Atmosphere { scale: 0.0, ..base },
            Atmosphere {
                intensity: f64::NAN,
                ..base
            },
            Atmosphere {
                drift: [f64::MAX, 0.0],
                ..base
            },
            Atmosphere {
                fractal: Fractal {
                    octaves: 9,
                    ..Fractal::default()
                },
                ..base
            },
            Atmosphere {
                kind: AtmosphereKind::Dust {
                    radius: 100.0,
                    coverage: 0.5,
                },
                ..base
            },
        ] {
            assert!(
                recipe
                    .render(size, 1.seconds(), RasterLimits::default())
                    .is_err()
            );
        }
    }

    #[test]
    fn zero_opacity_and_hdr_intensity_obey_image_contract() -> motion_core::Result {
        let size = PixelSize::new(16, 16);
        for mut recipe in [
            Atmosphere::fog(1),
            Atmosphere::dust(1),
            Atmosphere::light_leak(1),
        ] {
            recipe.opacity = 0.0;
            let image = recipe.render(size, 0.seconds(), RasterLimits::default())?;
            for level in image.levels() {
                for row in 0..level.height() {
                    for column in 0..level.width() {
                        assert_eq!(level.pixel(column, row), Some([0.0; 4]));
                    }
                }
            }
        }
        let mut leak = Atmosphere::light_leak(42);
        leak.intensity = 16.0;
        let image = leak.render(size, 0.seconds(), RasterLimits::default())?;
        assert!(image.levels()[0].pixel(0, 8).unwrap()[0] > 1.0);
        Ok(())
    }
}
