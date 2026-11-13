//! Validated simulation clocks, work allowances and batch presentation policies.

use super::Compiler;
use crate::particles::{ParticleConfigurationSpec, ParticleCurveSpec};
use motion_content::particles::{
    ParticleCurve, ParticleSimulationBuilder, ParticleStyle, SimulationLimits, TrailOptions,
};
use motion_core::Duration;

/// Maximum explicitly configured particle worker count.
const MAX_WORKERS: u8 = 64;

/// Maximum scalar-curve stop count.
const MAX_CURVE_STOPS: usize = 256;

impl Compiler<'_> {
    pub(super) fn particle_configuration(
        &mut self,
        mut simulation: ParticleSimulationBuilder,
        spec: &ParticleConfigurationSpec,
        max_particles: Option<u32>,
        path: &str,
    ) -> Option<ParticleSimulationBuilder> {
        let path = format!("{path}.configuration");
        let prewarm = self.seconds(spec.prewarm, &path, "prewarm")?;
        simulation = simulation.prewarm(prewarm);
        if let Some(period) = spec.loop_period {
            let period = self.seconds(period, &path, "loop_period")?;
            if period <= Duration::ZERO {
                self.error(&path, "loop_period", "Loop period must be positive");
                return None;
            }
            simulation = simulation.loop_period(period);
        }
        if let Some([scale, offset]) = spec.time_remap {
            let scale = self.bounded(scale, -1e6, 1e6, &path, "time_remap.scale")?;
            let offset = self.seconds(offset, &path, "time_remap.offset")?;
            if scale < 0.0 && !spec.allow_reverse {
                self.error(
                    &path,
                    "allow_reverse",
                    "Negative query scales require explicit opt-in",
                );
                return None;
            }
            simulation = simulation.time_remap(scale, offset, spec.allow_reverse);
        }
        if let Some(workers) = spec.workers {
            if !(1..=MAX_WORKERS).contains(&workers) {
                self.limit(&path, "workers", "Worker count must be in 1..=64");
                return None;
            }
            simulation = simulation.workers(usize::from(workers));
        }
        if let Some(work) = spec.max_work {
            if work == 0 {
                self.limit(&path, "max_work", "Work allowance must be positive");
                return None;
            }
            let defaults = SimulationLimits::default();
            simulation = simulation.limits(SimulationLimits {
                max_particles: max_particles.map_or(defaults.max_particles, |count| count as usize),
                max_work: work,
                ..defaults
            });
        }
        Some(simulation)
    }

    pub(super) fn particle_curve(
        &mut self,
        spec: &ParticleCurveSpec,
        path: &str,
    ) -> Option<ParticleCurve> {
        if spec.stops.is_empty() || spec.stops.len() > MAX_CURVE_STOPS {
            self.limit(path, "stops", "Particle curves take 1 to 256 stops");
            return None;
        }
        let curve = ParticleCurve {
            stops: spec
                .stops
                .iter()
                .map(|[time, value]| (*time, *value))
                .collect(),
            easing: self.easing(spec.easing, path)?,
        };
        curve
            .sample(0.0)
            .map_err(|error| self.error(path, "stops", error.to_string()))
            .ok()?;
        Some(curve)
    }

    pub(super) fn particle_presentation(
        &mut self,
        mut style: ParticleStyle,
        configuration: Option<&ParticleConfigurationSpec>,
        path: &str,
    ) -> Option<ParticleStyle> {
        let Some(spec) = configuration else {
            return Some(style);
        };
        if let Some(trails) = &spec.trails {
            if !(1..=64).contains(&trails.segments) {
                self.limit(
                    path,
                    "configuration.trails.segments",
                    "Trail segments must be in 1..=64",
                );
                return None;
            }
            let taper = match &trails.taper {
                Some(curve) => Some(self.particle_curve(curve, path)?),
                None => None,
            };
            let fade = match &trails.fade {
                Some(curve) => Some(self.particle_curve(curve, path)?),
                None => None,
            };
            style = style.trails(TrailOptions {
                segments: trails.segments,
                taper,
                fade,
                smooth: trails.smooth,
                ribbon: trails.ribbon,
            });
        }
        if let Some([scale, maximum]) = spec.stretch {
            self.bounded(
                f64::from(scale),
                0.0,
                1e6,
                path,
                "configuration.stretch.scale",
            )?;
            self.bounded(
                f64::from(maximum),
                1.0,
                64.0,
                path,
                "configuration.stretch.maximum",
            )?;
            style = style.stretch_by_speed(scale, maximum);
        }
        if let Some(offset) = spec.parallax {
            for value in offset {
                self.bounded(f64::from(value), -1e6, 1e6, path, "configuration.parallax")?;
            }
            style = style.parallax(offset);
        }
        if spec.flip {
            style = style.flip();
        }
        Some(style)
    }
}
