//! Deterministic particle recipes returning ordinary, further configurable simulations.
//!
//! ```
//! use motion_content::particles::presets::{Preset, PresetOptions};
//! use motion_core::{Rect, TimeLiteral};
//! let sparks = Preset::Sparks.simulation(PresetOptions::new(7, 2.seconds(), Rect::new(0.0, 0.0, 320.0, 180.0)))?.build()?;
//! assert!(!sparks.sample(100.milliseconds().after_origin())?.is_empty());
//! # Ok::<(), motion_core::Error>(())
//! ```

use super::*;
use motion_core::{Point, Rect, Vector2};

/// Curated, independently configurable recipes.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Preset {
    /// Tapered HDR ballistic burst.
    Sparks,
    /// Cooling, upward-moving glowing embers.
    Embers,
    /// Slow depth-aware dust.
    Dust,
    /// Soft floating motes.
    Motes,
    /// Three parallax depth layers of wind-driven snow.
    Snow,
    /// Stretched rain with collision-triggered splashes.
    Rain,
    /// Expanding procedural smoke sprites.
    Smoke,
    /// Buoyant procedural fire cores.
    Fire,
    /// Flipping, colored falling quads.
    Confetti,
    /// Rocket death, spark burst, then secondary spark deaths.
    Fireworks,
    /// Drifting and breathing light points.
    Fireflies,
    /// Color-preserving vector surface dispersal.
    LogoDissolve,
    /// Color-preserving vector surface convergence.
    LogoAssemble,
}

/// Adjustable inputs shared by the recipe gallery and Studio.
#[derive(Clone)]
pub struct PresetOptions {
    /// Explicit random seed.
    pub seed: u64,
    /// Visible clip duration.
    pub duration: Duration,
    /// Local design bounds.
    pub bounds: Rect,
    /// Density multiplier in (0,16].
    pub density: f64,
    /// Motion multiplier in (0,16].
    pub speed: f64,
    /// Finite HDR intensity multiplier in 0..=64.
    pub intensity: f32,
    /// Shared colored source required by the logo recipes.
    pub logo: Option<Arc<crate::VectorTree>>,
}

impl PresetOptions {
    /// Creates a recipe input with unit density, motion and intensity.
    pub fn new(seed: u64, duration: Duration, bounds: Rect) -> Self {
        Self {
            seed,
            duration,
            bounds,
            density: 1.0,
            speed: 1.0,
            intensity: 1.0,
            logo: None,
        }
    }
}

impl Preset {
    /// All recipes in a stable presentation order.
    pub const ALL: [Self; 13] = [
        Self::Sparks,
        Self::Embers,
        Self::Dust,
        Self::Motes,
        Self::Snow,
        Self::Rain,
        Self::Smoke,
        Self::Fire,
        Self::Confetti,
        Self::Fireworks,
        Self::Fireflies,
        Self::LogoDissolve,
        Self::LogoAssemble,
    ];

    /// Stable display name used by preview registries.
    pub fn name(self) -> &'static str {
        match self {
            Self::Sparks => "Sparks",
            Self::Embers => "Embers",
            Self::Dust => "Dust",
            Self::Motes => "Motes",
            Self::Snow => "Snow",
            Self::Rain => "Rain",
            Self::Smoke => "Smoke",
            Self::Fire => "Fire",
            Self::Confetti => "Confetti",
            Self::Fireworks => "Fireworks",
            Self::Fireflies => "Fireflies",
            Self::LogoDissolve => "Logo Dissolve",
            Self::LogoAssemble => "Logo Assemble",
        }
    }

    /// Starts a lazy simulation; callers can append operators, replace style or override limits.
    pub fn simulation(
        self,
        options: PresetOptions,
    ) -> Result<ParticleSimulationBuilder, ValidationError> {
        motion_core::drawing::validate_rect(options.bounds, "particles.preset", "bounds")?;
        if options.duration <= Duration::ZERO
            || options.duration.as_seconds() > 3600.0
            || options.bounds.width() <= 0.0
            || options.bounds.height() <= 0.0
            || !options.density.is_finite()
            || !(0.0..=16.0).contains(&options.density)
            || options.density == 0.0
            || !options.speed.is_finite()
            || !(0.0..=16.0).contains(&options.speed)
            || options.speed == 0.0
            || !options.intensity.is_finite()
            || !(0.0..=64.0).contains(&options.intensity)
        {
            return Err(invalid(
                "preset",
                "Invalid recipe bounds, duration, density, speed or intensity",
            ));
        }
        let bounds = options.bounds;
        let center = Point::new(
            (bounds.left + bounds.right) * 0.5,
            (bounds.top + bounds.bottom) * 0.5,
        );
        let bottom = Rect::new(
            bounds.left,
            bounds.bottom - 2.0,
            bounds.right,
            bounds.bottom,
        );
        let top = Rect::new(bounds.left, bounds.top, bounds.right, bounds.top + 2.0);
        let speed = options.speed;
        let count = |amount: f64| (amount * options.density).ceil() as usize;
        let trail = ParticleStyle::Trails {
            length: Duration::from_seconds(0.12),
            width: 2.0,
        }
        .trails(TrailOptions {
            segments: 16,
            taper: Some(ParticleCurve::linear(0.0, 1.0)),
            fade: Some(ParticleCurve::linear(0.0, 1.0)),
            smooth: true,
            ..Default::default()
        })
        .blend(ParticleBlend::Plus);
        let mut simulation = ParticleSimulation::new(options.seed).duration(options.duration);
        let curl_seed = options.seed as u32 ^ (options.seed >> 32) as u32;
        simulation = match self {
            Self::Sparks => simulation
                .emitter(
                    BurstEmitter::new(Time::ZERO, count(140.0), center)
                        .lifetime(Duration::from_seconds(1.8))
                        .speed(100.0 * speed),
                )
                .modifier(Gravity::new(Vector2::new(0.0, 95.0 * speed)))
                .modifier(Drag::new(0.6))
                .modifier(Temperature::new(ParticleCurve::linear(6500.0, 1200.0)))
                .style(trail),
            Self::Embers => simulation
                .emitter(
                    RectEmitter::new(bottom)
                        .rate(90.0 * options.density)
                        .speed(12.0)
                        .velocity(Vector2::new(0.0, -24.0 * speed))
                        .lifetime(Duration::from_seconds(3.0)),
                )
                .modifier(CurlNoise {
                    frequency: 0.03,
                    strength: 20.0 * speed,
                    seed: curl_seed,
                    phase_speed: 1.0,
                })
                .modifier(Temperature::new(ParticleCurve::linear(4200.0, 1100.0)))
                .prewarm(Duration::from_seconds(1.0))
                .style(ParticleStyle::GlowDots { radius: 1.8 }),
            Self::Dust | Self::Motes => simulation
                .emitter(
                    RectEmitter::new(bounds)
                        .rate(35.0 * options.density)
                        .speed(3.0 * speed)
                        .lifetime(Duration::from_seconds(5.0))
                        .depth_range(-2.0, 2.0)
                        .color([0.45, 0.65, 0.75, 0.75]),
                )
                .modifier(Drift {
                    seed: options.seed,
                    strength: 3.0 * speed,
                })
                .prewarm(Duration::from_seconds(1.5))
                .style(
                    ParticleStyle::Bokeh {
                        radius: if self == Self::Dust { 1.0 } else { 2.0 },
                        aperture: 0.8,
                        focus: 0.0,
                        shape: BokehShape::Polygon(6),
                        ring: 0.3,
                        hardness: 0.6,
                        highlight: 1.0,
                    }
                    .parallax([4.0, 1.0])
                    .blend(ParticleBlend::Screen),
                ),
            Self::Snow | Self::Rain => {
                for depth in [-1.0, 0.0, 1.0] {
                    simulation = simulation.emitter(
                        RectEmitter::new(top)
                            .rate(50.0 * options.density)
                            .speed(0.0)
                            .velocity(Vector2::new(
                                5.0 * speed,
                                if self == Self::Snow {
                                    32.0 * speed
                                } else {
                                    160.0 * speed
                                },
                            ))
                            .lifetime(Duration::from_seconds(4.0))
                            .depth_range(depth, depth)
                            .color([0.65, 0.85, 1.0, 0.8]),
                    );
                }
                simulation = simulation.modifier(Wind::new(Vector2::new(2.0, 0.0)));
                if self == Self::Rain {
                    simulation = simulation
                        .modifier(SegmentCollider {
                            start: Point::new(bounds.left, bounds.bottom),
                            end: Point::new(bounds.right, bounds.bottom),
                            response: CollisionResponse {
                                restitution: 0.3,
                                friction: 0.2,
                                kill_on_hit: true,
                            },
                        })
                        .sub_emitter(SubEmitter {
                            on: ParticleEventKind::Collision,
                            emitter: Arc::new(StageBurst {
                                parent: 0.0,
                                stage: 1.0,
                                burst: BurstEmitter::new(Time::ZERO, 4, Point::ORIGIN)
                                    .lifetime(Duration::from_seconds(0.2))
                                    .speed(20.0),
                            }),
                            inherit: Inherit::Color,
                            probability: 1.0,
                        });
                    simulation.style(
                        ParticleStyle::Shape {
                            radius: 1.2,
                            shape: ParticleShape::Capsule { aspect: 1.0 },
                        }
                        .stretch_by_speed(0.03, 8.0)
                        .parallax([3.0, 0.0]),
                    )
                } else {
                    simulation
                        .prewarm(Duration::from_seconds(1.0))
                        .style(ParticleStyle::Dots { radius: 1.8 }.parallax([8.0, 0.0]))
                }
            }
            Self::Smoke | Self::Fire => {
                let sprite = if self == Self::Smoke {
                    ProceduralSprite::Smoke {
                        scale: 3.0,
                        evolution: 0.0,
                        fractal: motion_core::noise::Fractal::default(),
                    }
                } else {
                    ProceduralSprite::FireCore
                }
                .generate(48, options.seed)?;
                let smoke = self == Self::Smoke;
                simulation = simulation
                    .emitter(
                        RectEmitter::new(bottom)
                            .rate(if smoke { 24.0 } else { 70.0 } * options.density)
                            .speed(8.0)
                            .velocity(Vector2::new(0.0, -20.0 * speed))
                            .lifetime(Duration::from_seconds(if smoke { 3.0 } else { 1.0 })),
                    )
                    .modifier(CurlNoise {
                        frequency: 0.025,
                        strength: 10.0,
                        seed: curl_seed,
                        phase_speed: 1.0,
                    })
                    .modifier(Wind::new(Vector2::new(0.0, -12.0 * speed)))
                    .modifier(SizeOverLifetime {
                        start: if smoke { 0.4 } else { 1.0 },
                        end: if smoke { 2.0 } else { 0.1 },
                    })
                    .modifier(ColorOverLifetime::new(Arc::from(if smoke {
                        [[0.18, 0.2, 0.22, 0.25], [0.0; 4]]
                    } else {
                        [[1.0; 4], [0.0; 4]]
                    })));
                if !smoke {
                    simulation = simulation
                        .modifier(Temperature::new(ParticleCurve::linear(6500.0, 1100.0)));
                }
                simulation.style(
                    ParticleStyle::Sprites {
                        image: motion_core::Assets::default()
                            .preload(
                                motion_core::Source::Bytes {
                                    key: "particle-preset-sprite".into(),
                                    data: Arc::from([]),
                                },
                                sprite,
                            )
                            .map_err(|_| {
                                invalid(
                                    "sprite",
                                    "Procedural preset material exceeds resource allowance",
                                )
                            })?,
                        size: if smoke { 30.0 } else { 14.0 },
                        align_to_velocity: false,
                    }
                    .blend(if smoke {
                        ParticleBlend::SourceOver
                    } else {
                        ParticleBlend::Plus
                    }),
                )
            }
            Self::Confetti => simulation
                .emitter(
                    RectEmitter::new(top)
                        .rate(60.0 * options.density)
                        .speed(25.0 * speed)
                        .lifetime(Duration::from_seconds(4.0))
                        .palette(Arc::from([
                            [0.8, 0.1, 0.2, 1.0],
                            [0.05, 0.7, 0.3, 1.0],
                            [0.1, 0.3, 0.9, 1.0],
                            [0.9, 0.7, 0.05, 1.0],
                        ])),
                )
                .modifier(Gravity::new(Vector2::new(0.0, 50.0 * speed)))
                .modifier(Drag::new(0.3))
                .modifier(FlipPhase(options.seed))
                .style(
                    ParticleStyle::Shape {
                        radius: 2.5,
                        shape: ParticleShape::Quad { aspect: 1.5 },
                    }
                    .flip(),
                ),
            Self::Fireworks => simulation
                .emitter(
                    BurstEmitter::new(Time::ZERO, 1, Point::new(center.x, bounds.bottom - 5.0))
                        .lifetime(Duration::from_seconds(0.8))
                        .speed(0.0)
                        .velocity(Vector2::new(0.0, -160.0 * speed)),
                )
                .modifier(Gravity::new(Vector2::new(0.0, 70.0 * speed)))
                .sub_emitter(SubEmitter {
                    on: ParticleEventKind::Death,
                    emitter: Arc::new(StageBurst {
                        parent: 0.0,
                        stage: 1.0,
                        burst: BurstEmitter::new(Time::ZERO, count(70.0), Point::ORIGIN)
                            .lifetime(Duration::from_seconds(0.7))
                            .speed(80.0 * speed),
                    }),
                    inherit: Inherit::Position,
                    probability: 1.0,
                })
                .sub_emitter(SubEmitter {
                    on: ParticleEventKind::Death,
                    emitter: Arc::new(StageBurst {
                        parent: 1.0,
                        stage: 2.0,
                        burst: BurstEmitter::new(Time::ZERO, 2, Point::ORIGIN)
                            .lifetime(Duration::from_seconds(0.25))
                            .speed(22.0),
                    }),
                    inherit: Inherit::Position,
                    probability: 0.5,
                })
                .modifier(Temperature::new(ParticleCurve::linear(6500.0, 1300.0)))
                .style(trail),
            Self::Fireflies => simulation
                .emitter(
                    RectEmitter::new(bounds)
                        .rate(10.0 * options.density)
                        .speed(2.0 * speed)
                        .lifetime(Duration::from_seconds(5.0))
                        .color([1.0, 1.0, 0.1, 1.0]),
                )
                .modifier(Drift {
                    seed: options.seed,
                    strength: 5.0 * speed,
                })
                .modifier(Breathe(options.seed))
                .style(ParticleStyle::GlowDots { radius: 2.0 }),
            Self::LogoDissolve | Self::LogoAssemble => {
                let tree = options
                    .logo
                    .clone()
                    .ok_or_else(|| invalid("logo", "Logo presets require a shared VectorTree"))?;
                tree.surface()?;
                simulation
                    .emitter(LogoEmitter {
                        tree,
                        count: count(600.0),
                        lifetime: options.duration.as_seconds() as f32,
                        seed: options.seed,
                        distance: bounds.width() * 0.65,
                        assemble: self == Self::LogoAssemble,
                    })
                    .modifier(LogoMotion {
                        seed: options.seed,
                        distance: bounds.width() * 0.65,
                        assemble: self == Self::LogoAssemble,
                    })
                    .style(ParticleStyle::Dots { radius: 1.0 })
            }
        };
        let (start, end) = match self {
            Self::Sparks | Self::Fireworks => (8.0, 0.0),
            Self::Embers | Self::Fire => (4.0, 0.0),
            _ => (1.0, 1.0),
        };
        Ok(
            simulation.modifier(IntensityOverLifetime::new(ParticleCurve::linear(
                options.intensity * start,
                options.intensity * end,
            ))),
        )
    }
}

struct StageBurst {
    parent: f32,
    stage: f32,
    burst: BurstEmitter,
}

impl ParticleEmitter for StageBurst {
    fn accepts_event(&self, event: &ParticleEvent) -> bool {
        event.particle.custom[3] == self.parent
    }

    fn emit(
        &self,
        step: u64,
        time: Time,
        random: &mut Random,
        output: &mut Vec<Particle>,
    ) -> Result<(), ValidationError> {
        let start = output.len();
        self.burst.emit(step, time, random, output)?;
        for particle in &mut output[start..] {
            particle.custom[3] = self.stage;
        }
        Ok(())
    }
}

struct Drift {
    seed: u64,
    strength: f64,
}

impl ParticleModifier for Drift {
    fn apply(
        &self,
        particle: &mut Particle,
        step: f32,
        time: Time,
        _: &mut Random,
    ) -> Result<(), ValidationError> {
        let phase =
            Random::for_particle(self.seed, particle.index, 0).next_float() * std::f64::consts::TAU;
        particle.velocity[0] +=
            (libm::sin(time.as_seconds() + phase) * self.strength) as f32 * step;
        particle.velocity[1] +=
            (libm::cos(time.as_seconds() * 0.7 + phase) * self.strength) as f32 * step;
        particle.validate()
    }
}

struct Breathe(u64);

impl ParticleModifier for Breathe {
    fn resets_color(&self) -> bool {
        true
    }

    fn apply(
        &self,
        particle: &mut Particle,
        _: f32,
        time: Time,
        _: &mut Random,
    ) -> Result<(), ValidationError> {
        let phase =
            Random::for_particle(self.0, particle.index, 0).next_float() * std::f64::consts::TAU;
        let energy = (1.5 + 1.4 * libm::sin(time.as_seconds() * 2.0 + phase)) as f32;
        for channel in &mut particle.color[..3] {
            *channel *= energy;
        }
        particle.validate()
    }
}

struct FlipPhase(u64);

impl ParticleModifier for FlipPhase {
    fn apply(
        &self,
        particle: &mut Particle,
        _: f32,
        _: Time,
        _: &mut Random,
    ) -> Result<(), ValidationError> {
        let phase = Random::for_particle(self.0, particle.index, 0).next_float() as f32
            * std::f32::consts::TAU;
        particle.custom[2] = phase;
        particle.rotation = phase + particle.age * 0.5;
        particle.validate()
    }
}

struct LogoEmitter {
    tree: Arc<crate::VectorTree>,
    count: usize,
    lifetime: f32,
    seed: u64,
    distance: f64,
    assemble: bool,
}

impl ParticleEmitter for LogoEmitter {
    fn emit(
        &self,
        step: u64,
        _: Time,
        random: &mut Random,
        output: &mut Vec<Particle>,
    ) -> Result<(), ValidationError> {
        if step != 0 {
            return Ok(());
        }
        let seed = (random.next_float() * u64::MAX as f64) as u64;
        for (index, (point, color)) in self
            .tree
            .sample_colored_points(self.count, seed)?
            .into_iter()
            .enumerate()
        {
            let mut keyed = Random::for_particle(self.seed, index as u64, 0);
            let direction = keyed.unit_vector();
            let distance = self.distance * (0.5 + keyed.next_float());
            let position = if self.assemble {
                [
                    (point.x + direction.x * distance) as f32,
                    (point.y + direction.y * distance) as f32,
                ]
            } else {
                [point.x as f32, point.y as f32]
            };
            output.push(Particle {
                position,
                custom: [point.x as f32, 0.0, point.y as f32, 0.0],
                color,
                lifetime: self.lifetime,
                ..Default::default()
            });
        }
        Ok(())
    }
}

struct LogoMotion {
    seed: u64,
    distance: f64,
    assemble: bool,
}

impl ParticleModifier for LogoMotion {
    fn apply(
        &self,
        particle: &mut Particle,
        _: f32,
        _: Time,
        _: &mut Random,
    ) -> Result<(), ValidationError> {
        let progress = (particle.age / particle.lifetime).clamp(0.0, 1.0);
        let eased = progress * progress * (3.0 - 2.0 * progress);
        let amount = if self.assemble { 1.0 - eased } else { eased };
        let mut random = Random::for_particle(self.seed, particle.index, 0);
        let direction = random.unit_vector();
        let distance = self.distance * (0.5 + random.next_float());
        particle.position = [
            particle.custom[0] + (direction.x * distance) as f32 * amount,
            particle.custom[2] + (direction.y * distance) as f32 * amount,
        ];
        particle.velocity = [0.0; 2];
        particle.validate()
    }
}
