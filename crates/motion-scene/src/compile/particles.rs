//! Compilation of particle simulations and analytic trajectories.

use super::Compiler;
use super::element::Built;
use super::style::Style;
use crate::particles::{ParticlesSpec, TrajectorySpec};
#[cfg(feature = "particles")]
use motion_content::ParticleSimulationBuilder;
use std::sync::Arc;

#[cfg(feature = "particles")]
use super::failure::failure;
#[cfg(feature = "particles")]
use super::style::point;
#[cfg(feature = "particles")]
use super::value::Lift;
#[cfg(feature = "particles")]
use crate::particles::{
    EmitterShape, EmitterSpec, ModifierSpec, ParticleBlendSpec, ParticleShapeSpec,
    ParticleStyleSpec, TrajectoryModelSpec,
};
#[cfg(feature = "particles")]
use motion_content::particles::{
    Attractor, Ballistic, BounceBox, BurstEmitter, ColorOverLifetime, CurlNoise, DiskEmitter, Drag,
    Gravity, NoiseDrift, Orbit, ParticleShape, ParticleSimulation, ParticleStyle,
    ParticleTrajectory, PathEmitter, PointEmitter, RectEmitter, Seek, SimulationLimits,
    SizeOverLifetime, Vortex, Wind,
};
#[cfg(feature = "particles")]
use motion_core::{Duration, Rect, Time, Vector2};

/// Most simultaneously living particles a document may request.
#[cfg(feature = "particles")]
const MAX_PARTICLES: u32 = 1_000_000;

/// Largest particle count of one burst or trajectory.
#[cfg(feature = "particles")]
const MAX_COUNT: u32 = 1_000_000;

/// Applies the birth settings shared by every continuous emitter type.
#[cfg(feature = "particles")]
macro_rules! continuous {
    ($emitter:expr, $birth:expr) => {{
        let birth: &Birth = $birth;
        let mut emitter = $emitter;
        if let Some(rate) = birth.rate {
            emitter = emitter.rate(rate);
        }
        if let Some(lifetime) = birth.lifetime {
            emitter = emitter.lifetime(lifetime);
        }
        if let Some(speed) = birth.speed {
            emitter = emitter.speed(speed);
        }
        if let Some(velocity) = birth.velocity {
            emitter = emitter.velocity(velocity);
        }
        if let Some(color) = birth.color {
            emitter = emitter.color(color);
        }
        if let Some(size) = birth.size {
            emitter = emitter.size(size);
        }
        if let Some((near, far)) = birth.depth {
            emitter = emitter.depth_range(near, far);
        }
        emitter
    }};
}

impl Compiler<'_> {
    /// Compiles a simulation; particles read the scene clock, cache and duration.
    #[cfg(feature = "particles")]
    pub(crate) fn particles(
        &mut self,
        spec: &ParticlesSpec,
        style: Option<Arc<Style>>,
        path: &str,
    ) -> Option<Built> {
        let duration = spec
            .duration
            .map(|seconds| self.positive(seconds, path, "duration"));
        let step = spec
            .step
            .map(|seconds| self.positive(seconds, path, "step"));
        let count = spec.max_particles.map(|count| {
            self.bounded(
                f64::from(count),
                1.0,
                f64::from(MAX_PARTICLES),
                path,
                "max_particles",
            )
        });
        if spec.emitters.is_empty() {
            self.error(path, "emitters", "A simulation needs at least one emitter");
        }
        let mut simulation = ParticleSimulation::new(spec.seed);
        let mut valid = !spec.emitters.is_empty();
        for (index, emitter) in spec.emitters.iter().enumerate() {
            let here = format!("{path}.emitters[{index}]");
            match self.emitter(emitter, &here) {
                Some(Emitter::Point(emitter)) => simulation = simulation.emitter(emitter),
                Some(Emitter::Disk(emitter)) => simulation = simulation.emitter(emitter),
                Some(Emitter::Rect(emitter)) => simulation = simulation.emitter(emitter),
                Some(Emitter::Path(emitter)) => simulation = simulation.emitter(emitter),
                Some(Emitter::Burst(emitter)) => simulation = simulation.emitter(emitter),
                None => valid = false,
            }
        }
        for (index, modifier) in spec.modifiers.iter().enumerate() {
            let here = format!("{path}.modifiers[{index}]");
            simulation = self.modifier(simulation, modifier, &here)?;
        }
        let particle_style = self.particle_style(spec.style.as_ref(), spec.blend, path);
        let (duration, step, count) = (duration.lift()?, step.lift()?, count.lift()?);
        style?;
        if !valid {
            return None;
        }
        if let Some(duration) = duration {
            simulation = simulation.duration(duration);
        }
        if let Some(step) = step {
            simulation = simulation.step(step);
        }
        if let Some(count) = count {
            simulation = simulation.limits(SimulationLimits {
                max_particles: count as usize,
                ..SimulationLimits::default()
            });
        }
        let simulation = match simulation.style(particle_style?).build() {
            Ok(simulation) => simulation,
            Err(error) => {
                self.error(path, "simulation", error.to_string());
                return None;
            }
        };
        Some(Built::with_context(true, move |context| {
            simulation.try_render(context).unwrap_or_else(failure_node)
        }))
    }

    /// Compiles an analytic trajectory.
    #[cfg(feature = "particles")]
    pub(crate) fn trajectory(
        &mut self,
        spec: &TrajectorySpec,
        style: Option<Arc<Style>>,
        path: &str,
    ) -> Option<Built> {
        let path_model = format!("{path}.model");
        let particle_style = self.particle_style(spec.style.as_ref(), spec.blend, path);
        let trajectory = match &spec.model {
            TrajectoryModelSpec::Ballistic {
                center,
                count,
                gravity,
                drag,
                speed,
                seed,
                lifetime,
            } => {
                let path = format!("{path_model}.ballistic");
                let count = self.particle_count(*count, &path);
                let lifetime = lifetime.map(|value| self.positive(value, &path, "lifetime"));
                let mut model = Ballistic::burst(point(*center), u64::from(count?)).seed(*seed);
                if let Some([x, y]) = gravity {
                    model = model.gravity(Vector2::new(*x, *y));
                }
                if let Some(drag) = drag {
                    model = model.drag(*drag);
                }
                if let Some([minimum, maximum]) = speed {
                    model = model.speed(*minimum, *maximum);
                }
                if let Some(lifetime) = lifetime.lift()? {
                    model = model.lifetime(lifetime);
                }
                ParticleTrajectory::new(model)
            }
            TrajectoryModelSpec::Orbit {
                center,
                radius,
                count,
                angular_velocity,
                seed,
                lifetime,
            } => {
                let path = format!("{path_model}.orbit");
                let count = self.particle_count(*count, &path);
                let lifetime = lifetime.map(|value| self.positive(value, &path, "lifetime"));
                let mut model = Orbit::new(point(*center), *radius, u64::from(count?)).seed(*seed);
                if let Some(velocity) = angular_velocity {
                    model = model.angular_velocity(*velocity);
                }
                if let Some(lifetime) = lifetime.lift()? {
                    model = model.lifetime(lifetime);
                }
                ParticleTrajectory::new(model)
            }
            TrajectoryModelSpec::NoiseDrift {
                origin,
                count,
                velocity,
                noise,
                seed,
                lifetime,
            } => {
                let path = format!("{path_model}.noise_drift");
                let count = self.particle_count(*count, &path);
                let lifetime = lifetime.map(|value| self.positive(value, &path, "lifetime"));
                let mut model = NoiseDrift::new(point(*origin), u64::from(count?)).seed(*seed);
                if let Some([x, y]) = velocity {
                    model = model.velocity(Vector2::new(*x, *y));
                }
                if let Some([amplitude, frequency]) = noise {
                    model = model.noise(*amplitude, *frequency);
                }
                if let Some(lifetime) = lifetime.lift()? {
                    model = model.lifetime(lifetime);
                }
                ParticleTrajectory::new(model)
            }
        };
        style?;
        let trajectory = trajectory.style(particle_style?);
        Some(Built::with_context(true, move |context| {
            trajectory.try_render(context).unwrap_or_else(failure_node)
        }))
    }

    #[cfg(feature = "particles")]
    fn positive(&mut self, seconds: f64, path: &str, field: &str) -> Option<Duration> {
        let time = self.seconds(seconds, path, field)?;
        if time > Time::ZERO {
            Some(time)
        } else {
            self.error(path, field, "Duration must be positive");
            None
        }
    }

    #[cfg(feature = "particles")]
    fn particle_count(&mut self, count: u32, path: &str) -> Option<u32> {
        if (1..=MAX_COUNT).contains(&count) {
            Some(count)
        } else {
            self.error(path, "count", "Count must be in 1..=1000000");
            None
        }
    }

    #[cfg(feature = "particles")]
    fn emitter(&mut self, spec: &EmitterSpec, path: &str) -> Option<Emitter> {
        let birth = self.birth(spec, path)?;
        Some(match &spec.shape {
            EmitterShape::Point(position) => {
                Emitter::Point(continuous!(PointEmitter::new(point(*position)), &birth))
            }
            EmitterShape::Disk { center, radius } => Emitter::Disk(continuous!(
                DiskEmitter::new(point(*center), *radius),
                &birth
            )),
            EmitterShape::Rect([left, top, right, bottom]) => Emitter::Rect(continuous!(
                RectEmitter::new(Rect::new(*left, *top, *right, *bottom)),
                &birth
            )),
            EmitterShape::Path(data) => {
                let geometry = self.svg_path(data, &format!("{path}.path"))?;
                Emitter::Path(continuous!(PathEmitter::new(Arc::new(geometry)), &birth))
            }
            EmitterShape::Burst {
                time,
                count,
                position,
            } => {
                if spec.rate.is_some() {
                    self.error(path, "rate", "Bursts are instantaneous and take no rate");
                    return None;
                }
                let time = self.seconds(*time, &format!("{path}.burst"), "time");
                let count = self.particle_count(*count, &format!("{path}.burst"));
                let mut emitter = BurstEmitter::new(time?, count? as usize, point(*position));
                if let Some(lifetime) = birth.lifetime {
                    emitter = emitter.lifetime(lifetime);
                }
                if let Some(speed) = birth.speed {
                    emitter = emitter.speed(speed);
                }
                if let Some(velocity) = birth.velocity {
                    emitter = emitter.velocity(velocity);
                }
                if let Some(color) = birth.color {
                    emitter = emitter.color(color);
                }
                if let Some(size) = birth.size {
                    emitter = emitter.size(size);
                }
                if let Some((near, far)) = birth.depth {
                    emitter = emitter.depth_range(near, far);
                }
                Emitter::Burst(emitter)
            }
        })
    }

    #[cfg(feature = "particles")]
    fn birth(&mut self, spec: &EmitterSpec, path: &str) -> Option<Birth> {
        let rate = spec
            .rate
            .map(|rate| self.bounded(rate, 0.0, 1e7, path, "rate"));
        let lifetime = spec
            .lifetime
            .map(|seconds| self.positive(seconds, path, "lifetime"));
        let speed = spec
            .speed
            .map(|speed| self.bounded(speed, -1e9, 1e9, path, "speed"));
        let size = spec
            .size
            .map(|size| self.bounded(size, 0.0, 4096.0, path, "size"));
        let velocity = spec.velocity.map(|[x, y]| {
            let x = self.bounded(x, -1e9, 1e9, path, "velocity");
            let y = self.bounded(y, -1e9, 1e9, path, "velocity");
            Some(Vector2::new(x?, y?))
        });
        let depth = spec.depth_range.map(|[near, far]| {
            let near = self.bounded(near, -1e6, 1e6, path, "depth_range");
            let far = self.bounded(far, -1e6, 1e6, path, "depth_range");
            Some((near? as f32, far? as f32))
        });
        Some(Birth {
            rate: rate.lift()?,
            lifetime: lifetime.lift()?,
            speed: speed.lift()?,
            velocity: velocity.lift()?,
            color: spec
                .color
                .as_ref()
                .map(|color| premultiplied(color.color())),
            size: size.lift()?.map(|size| size as f32),
            depth: depth.lift()?,
        })
    }

    #[cfg(feature = "particles")]
    fn modifier(
        &mut self,
        simulation: ParticleSimulationBuilder,
        modifier: &ModifierSpec,
        path: &str,
    ) -> Option<ParticleSimulationBuilder> {
        let vector = |[x, y]: [f64; 2]| Vector2::new(x, y);
        Some(match modifier {
            ModifierSpec::Gravity(value) => simulation.modifier(Gravity::new(vector(*value))),
            ModifierSpec::Wind(value) => simulation.modifier(Wind::new(vector(*value))),
            ModifierSpec::Drag(value) => simulation.modifier(Drag::new(*value)),
            ModifierSpec::Vortex { center, strength } => simulation.modifier(Vortex {
                center: point(*center),
                strength: *strength,
            }),
            ModifierSpec::Attractor { center, strength } => simulation.modifier(Attractor {
                center: point(*center),
                strength: *strength,
            }),
            ModifierSpec::CurlNoise {
                frequency,
                strength,
                seed,
                phase_speed,
            } => simulation.modifier(CurlNoise {
                frequency: *frequency,
                strength: *strength,
                seed: *seed,
                phase_speed: *phase_speed,
            }),
            ModifierSpec::Seek { targets, strength } => {
                if targets.is_empty() || targets.len() > 65_536 {
                    self.limit(path, "targets", "Seek takes 1 to 65536 targets");
                    return None;
                }
                simulation.modifier(Seek {
                    targets: targets.iter().copied().map(point).collect(),
                    strength: *strength,
                })
            }
            ModifierSpec::BounceBox([left, top, right, bottom]) => {
                simulation.modifier(BounceBox::new(Rect::new(*left, *top, *right, *bottom)))
            }
            ModifierSpec::ColorOverLifetime(stops) => {
                if stops.is_empty() || stops.len() > 256 {
                    self.limit(
                        path,
                        "color_over_lifetime",
                        "Color ramps take 1 to 256 stops",
                    );
                    return None;
                }
                let stops: Vec<[f32; 4]> = stops
                    .iter()
                    .map(|stop| premultiplied(stop.color()))
                    .collect();
                simulation.modifier(ColorOverLifetime::new(stops))
            }
            ModifierSpec::SizeOverLifetime { start, end } => {
                simulation.modifier(SizeOverLifetime {
                    start: *start,
                    end: *end,
                })
            }
        })
    }

    #[cfg(feature = "particles")]
    fn particle_style(
        &mut self,
        style: Option<&ParticleStyleSpec>,
        blend: Option<ParticleBlendSpec>,
        path: &str,
    ) -> Option<ParticleStyle> {
        let path = format!("{path}.style");
        let mut particle = match style {
            None => ParticleStyle::default(),
            Some(ParticleStyleSpec::Dots { radius }) => ParticleStyle::Dots { radius: *radius },
            Some(ParticleStyleSpec::GlowDots { radius }) => {
                ParticleStyle::GlowDots { radius: *radius }
            }
            Some(ParticleStyleSpec::Trails { length, width }) => ParticleStyle::Trails {
                length: self.positive(*length, &path, "length")?,
                width: *width,
            },
            Some(ParticleStyleSpec::Plexus {
                radius,
                max_distance,
            }) => ParticleStyle::Plexus {
                radius: *radius,
                max_distance: *max_distance,
            },
            Some(ParticleStyleSpec::Shape { radius, shape }) => ParticleStyle::Shape {
                radius: *radius,
                shape: match *shape {
                    ParticleShapeSpec::Polygon { sides } => ParticleShape::Polygon { sides },
                    ParticleShapeSpec::Ring { thickness } => ParticleShape::Ring {
                        thickness: thickness as f32,
                    },
                    ParticleShapeSpec::Capsule { aspect } => ParticleShape::Capsule {
                        aspect: aspect as f32,
                    },
                    ParticleShapeSpec::Soft { hardness } => ParticleShape::Soft {
                        hardness: hardness as f32,
                    },
                    ParticleShapeSpec::Quad { aspect } => ParticleShape::Quad {
                        aspect: aspect as f32,
                    },
                },
            },
        };
        if let Some(blend) = blend {
            particle = particle.blend(match blend {
                ParticleBlendSpec::SourceOver => motion_core::instances::InstanceBlend::SourceOver,
                ParticleBlendSpec::Plus => motion_core::instances::InstanceBlend::Plus,
                ParticleBlendSpec::Screen => motion_core::instances::InstanceBlend::Screen,
                ParticleBlendSpec::Multiply => motion_core::instances::InstanceBlend::Multiply,
            });
        }
        Some(particle)
    }

    #[cfg(not(feature = "particles"))]
    pub(crate) fn particles(
        &mut self,
        _: &ParticlesSpec,
        _: Option<Arc<Style>>,
        path: &str,
    ) -> Option<Built> {
        self.unsupported(path, "Particles require feature `particles`");
        None
    }

    #[cfg(not(feature = "particles"))]
    pub(crate) fn trajectory(
        &mut self,
        _: &TrajectorySpec,
        _: Option<Arc<Style>>,
        path: &str,
    ) -> Option<Built> {
        self.unsupported(path, "Particles require feature `particles`");
        None
    }
}

/// Birth settings shared by every emitter, already validated.
#[cfg(feature = "particles")]
struct Birth {
    rate: Option<f64>,
    lifetime: Option<Duration>,
    speed: Option<f64>,
    velocity: Option<Vector2>,
    color: Option<[f32; 4]>,
    size: Option<f32>,
    depth: Option<(f32, f32)>,
}

#[cfg(feature = "particles")]
enum Emitter {
    Point(PointEmitter),
    Disk(DiskEmitter),
    Rect(RectEmitter),
    Path(PathEmitter),
    Burst(BurstEmitter),
}

/// Authored encoded sRGB with straight alpha to the engine's premultiplied form.
#[cfg(feature = "particles")]
fn premultiplied(color: motion_core::Color) -> [f32; 4] {
    let [red, green, blue, alpha] = color.to_rgba_float();
    [
        (red * alpha) as f32,
        (green * alpha) as f32,
        (blue * alpha) as f32,
        alpha as f32,
    ]
}

#[cfg(feature = "particles")]
fn failure_node(error: motion_core::ValidationError) -> motion_core::Node {
    failure(error)
}
