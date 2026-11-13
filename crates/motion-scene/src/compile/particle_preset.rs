//! Compilation of shared particle recipes and their fixed colored vector sources.

use super::Compiler;
use crate::particles::{ParticlePresetSpec, ParticlesSpec};
use motion_content::particles::presets::{Preset, PresetOptions};
use motion_content::{ParticleSimulationBuilder, VectorGroup, VectorItem, VectorPath, VectorTree};
use motion_core::{Rect, Time};
use std::sync::Arc;

/// Maximum painted paths in one preset's vector source.
const MAX_LOGO_PATHS: usize = 256;

/// Maximum source simulation duration in seconds accepted by shared presets.
const MAX_PRESET_SECONDS: f64 = 3600.0;

/// Maximum density or motion multiplier accepted by shared presets.
const MAX_MULTIPLIER: f64 = 16.0;

/// Maximum linear HDR intensity accepted by shared presets.
const MAX_INTENSITY: f64 = 64.0;

impl Compiler<'_> {
    pub(super) fn particle_preset(
        &mut self,
        particles: &ParticlesSpec,
        spec: &ParticlePresetSpec,
        path: &str,
    ) -> Option<ParticleSimulationBuilder> {
        let mut conflicts = false;
        for (present, field) in [
            (!particles.emitters.is_empty(), "emitters"),
            (!particles.modifiers.is_empty(), "modifiers"),
            (particles.style.is_some(), "style"),
            (particles.blend.is_some(), "blend"),
            (!particles.sub_emitters.is_empty(), "sub_emitters"),
            (
                particles
                    .configuration
                    .as_ref()
                    .is_some_and(|configuration| {
                        configuration.trails.is_some()
                            || configuration.stretch.is_some()
                            || configuration.parallax.is_some()
                            || configuration.flip
                    }),
                "configuration.presentation",
            ),
        ] {
            if present {
                self.error(path, field, "Cannot be combined with a preset");
                conflicts = true;
            }
        }
        if conflicts {
            return None;
        }
        let Some(seconds) = particles.duration else {
            self.error(
                path,
                "duration",
                "Presets require an explicit simulation duration",
            );
            return None;
        };
        let mut valid = true;
        for (value, field, maximum) in [
            (seconds, "duration", MAX_PRESET_SECONDS),
            (
                spec.density.unwrap_or(1.0),
                "preset.density",
                MAX_MULTIPLIER,
            ),
            (spec.speed.unwrap_or(1.0), "preset.speed", MAX_MULTIPLIER),
        ] {
            if !value.is_finite() || value <= 0.0 || value > maximum {
                self.error(
                    path,
                    field,
                    format!("Value must be finite and in (0, {maximum}]"),
                );
                valid = false;
            }
        }
        if self
            .bounded(
                f64::from(spec.intensity.unwrap_or(1.0)),
                0.0,
                MAX_INTENSITY,
                path,
                "preset.intensity",
            )
            .is_none()
        {
            valid = false;
        }
        if spec.bounds.iter().any(|value| !value.is_finite())
            || !(spec.bounds[2] - spec.bounds[0]).is_finite()
            || !(spec.bounds[3] - spec.bounds[1]).is_finite()
            || spec.bounds[2] <= spec.bounds[0]
            || spec.bounds[3] <= spec.bounds[1]
        {
            self.error(
                path,
                "preset.bounds",
                "Bounds must be finite and have positive width and height",
            );
            valid = false;
        }
        if matches!(spec.kind, Preset::LogoDissolve | Preset::LogoAssemble) && spec.logo.is_empty()
        {
            self.error(
                path,
                "preset.logo",
                "Logo presets require painted vector paths",
            );
            valid = false;
        }
        if !valid {
            return None;
        }
        let duration = self.seconds(seconds, path, "duration")?;
        if spec.logo.len() > MAX_LOGO_PATHS {
            self.limit(
                path,
                "preset.logo",
                "A preset logo accepts at most 256 painted paths",
            );
            return None;
        }
        let [left, top, right, bottom] = spec.bounds;
        let bounds = Rect::new(left, top, right, bottom);
        let mut options = PresetOptions::new(particles.seed, duration, bounds);
        options.density = spec.density.unwrap_or(1.0);
        options.speed = spec.speed.unwrap_or(1.0);
        options.intensity = spec.intensity.unwrap_or(1.0);
        let mut paths = Vec::with_capacity(spec.logo.len());
        for (index, source) in spec.logo.iter().enumerate() {
            let here = format!("{path}.preset.logo[{index}]");
            let geometry = self.svg_path(&source.data, &format!("{here}.data"));
            let paint = self.paint(&source.fill, &format!("{here}.fill"));
            let (geometry, paint) = (geometry?, paint?);
            if paint.varying() {
                self.error(&here, "fill", "Particle source paints must be fixed");
                return None;
            }
            let paint = paint
                .at(Time::ZERO)
                .map_err(|error| self.error(&here, "fill", error.message))
                .ok()?;
            paths.push(VectorItem::Path(VectorPath::filled(geometry, paint)));
        }
        if !paths.is_empty() {
            options.logo = Some(Arc::new(
                VectorTree::new(
                    bounds,
                    VectorGroup {
                        children: paths,
                        ..Default::default()
                    },
                )
                .map_err(|error| self.error(path, "preset.logo", error.to_string()))
                .ok()?,
            ));
        }
        spec.kind
            .simulation(options)
            .map_err(|error| self.error(path, "preset", error.to_string()))
            .ok()
    }
}
