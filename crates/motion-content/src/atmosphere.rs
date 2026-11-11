//! Deterministic advected fog, sparse dust and HDR light-leak image generators.
//!
//! Explicitly generated assets use the ordinary CPU/GPU image path; [`Atmosphere::paint`]
//! instead captures independent procedural evaluation without baking. Time and global
//! pixel coordinates are explicit; these are 2D overlays, not volumetric scattering.
//!
//! ```
//! use motion_content::{Image, atmosphere::Atmosphere};
//! use motion_core::{BlendMode, Node, NodeModifiers, PixelSize, RasterLimits, TimeLiteral};
//! let asset = Atmosphere::light_leak(42).render(
//!     PixelSize::new(32, 24), 1.seconds().after_origin(), RasterLimits::sandbox(),
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

pub use motion_core::AtmosphereKind;

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
        self.parameters().validate()
    }

    /// Captures the same recipe as an independently evaluated CPU/GPU paint.
    ///
    /// Canvas pixels determine wavelength, drift and envelope coordinates; painted
    /// shape bounds map to that canvas. This does not bake or allocate an image.
    pub fn paint(
        &self,
        canvas: PixelSize,
        time: Time,
    ) -> Result<motion_core::AtmospherePaint, ValidationError> {
        motion_core::AtmospherePaint::new(self.parameters(), canvas, time)
    }

    fn parameters(&self) -> motion_core::AtmosphereParameters {
        motion_core::AtmosphereParameters {
            kind: self.kind,
            seed: self.seed,
            color: self.color,
            opacity: self.opacity,
            intensity: self.intensity,
            scale: self.scale,
            drift: self.drift,
            fractal: self.fractal,
        }
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
        let parameters = self.parameters();
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
                values.push(parameters.sample(pixel, canvas, phase)?);
            }
        }
        ImageAsset::from_rgba_float(size.width, size.height, values)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use motion_core::Shape;
    use motion_core::{BlendMode, Node, NodeModifiers, Rect, Size, TimeLiteral};
    use motion_render::{Composition, Renderer};

    #[test]
    fn procedural_paint_preserves_recipe_time_and_unquantized_bake_reference() -> motion_core::Result
    {
        let size = PixelSize::new(32, 24);
        let time = 0.75.seconds().after_origin();
        for recipe in [
            Atmosphere::fog(42),
            Atmosphere::dust(42),
            Atmosphere::light_leak(42),
        ] {
            let paint = recipe.paint(size, time)?;
            assert_eq!(paint.parameters(), &recipe.parameters());
            assert_eq!(paint.time(), time);
            assert_eq!(paint.canvas(), size);
            let image = recipe.render(size, time, RasterLimits::default())?;
            for row in 0..size.height {
                for column in 0..size.width {
                    let analytic = paint.sample(motion_core::Point::new(
                        (f64::from(column) + 0.5) / f64::from(size.width),
                        (f64::from(row) + 0.5) / f64::from(size.height),
                    ))?;
                    let baked = image.levels()[0].pixel(column, row).unwrap();
                    for (analytic, baked) in analytic.into_iter().zip(baked) {
                        assert!((analytic - baked).abs() <= 2e-3);
                    }
                }
            }
        }
        Ok(())
    }

    fn compose(
        recipe: Atmosphere,
        mode: BlendMode,
        renderer: Renderer,
        time: Time,
    ) -> motion_core::Result<motion_render::FrameBuffer> {
        let size = PixelSize::new(64, 48);
        let image = std::sync::Arc::new(recipe.render(size, time, RasterLimits::default())?);
        Composition::new(
            64,
            48,
            motion_render::FrameClock::new(motion_core::fps!(30), 150),
        )
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
            let overlay =
                recipe.overlay(size, 1.seconds().after_origin(), RasterLimits::default())?;
            let frame = Composition::new(
                64,
                48,
                motion_render::FrameClock::new(motion_core::fps!(30), 150),
            )
            .with_options(motion_render::RenderOptions::default().backend(Renderer::Cpu.into()))?
            .render_frame(0, Size::new(64.0, 48.0), |_| {
                Node::group([
                    Rect::new(0.0, 0.0, 64.0, 48.0)
                        .fill(Color::rgb(18, 38, 45))
                        .into(),
                    overlay.clone(),
                ])
            })?;
            let manual = compose(
                recipe,
                recipe.blend_mode(),
                Renderer::Cpu,
                1.seconds().after_origin(),
            )?;
            assert_eq!(frame.packed_rgb(), manual.packed_rgb());
        }
        let invalid = Atmosphere {
            opacity: 2.0,
            ..Atmosphere::fog(1)
        };
        assert!(
            invalid
                .overlay(size, 0.seconds().after_origin(), RasterLimits::default())
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
                let frame = compose(recipe, mode, Renderer::Cpu, 0.seconds().after_origin())?;
                assert_eq!(
                    frame.packed_rgb(),
                    compose(recipe, mode, Renderer::Cpu, 0.seconds().after_origin())?.packed_rgb()
                );
                assert_ne!(
                    frame.packed_rgb(),
                    compose(recipe, mode, Renderer::Cpu, 1.seconds().after_origin())?.packed_rgb()
                );
                assert!(
                    frame
                        .packed_rgb()
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
                let cpu = compose(recipe, mode, Renderer::Cpu, 1.seconds().after_origin())?;
                let gpu = compose(recipe, mode, Renderer::Gpu, 1.seconds().after_origin())?;
                assert_eq!(
                    gpu.packed_rgb(),
                    compose(recipe, mode, Renderer::Gpu, 1.seconds().after_origin())?.packed_rgb()
                );
                for (cpu, gpu) in cpu.packed_rgb().iter().zip(gpu.packed_rgb()) {
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
            let full = recipe.render(size, time.after_origin(), RasterLimits::default())?;
            assert_eq!(
                full,
                recipe.render(size, time.after_origin(), RasterLimits::default())?
            );
            assert_ne!(
                full,
                recipe.render(size, 1.seconds().after_origin(), RasterLimits::default())?
            );
            let region = recipe.render_region(
                size,
                [17, 11],
                PixelSize::new(31, 23),
                time.after_origin(),
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
            assert_ne!(
                full,
                recipe.render(size, time.after_origin(), RasterLimits::default())?
            );
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
            assert!(base.render(size, time.after_origin(), limits).is_err());
        }
        assert!(
            base.render(
                PixelSize::new(0, 1),
                time.after_origin(),
                RasterLimits::default()
            )
            .is_err()
        );
        assert!(
            base.render_region(
                size,
                [8, 0],
                size,
                time.after_origin(),
                RasterLimits::default()
            )
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
                    .render(size, 1.seconds().after_origin(), RasterLimits::default())
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
            let image = recipe.render(size, 0.seconds().after_origin(), RasterLimits::default())?;
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
        let image = leak.render(size, 0.seconds().after_origin(), RasterLimits::default())?;
        assert!(image.levels()[0].pixel(0, 8).unwrap()[0] > 1.0);
        Ok(())
    }
}
