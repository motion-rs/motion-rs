//! Bounded force, population, collider and contact-event capability specimens.

use super::{SOURCE_SECONDS, Study, VIEWPORT, base, text};
use crate::DemoParameters;
use motion::particles::{
    Attractor, Boids, BounceBox, BurstEmitter, CircleCollider, CollisionResponse, CurlNoise,
    DiskEmitter, Drag, FlowField, FlowSource, ForceFalloff, Gravity, Inherit, Particle,
    ParticleCollisions, ParticleCurve, ParticleEmitter, ParticleEvent, ParticleEventKind,
    ParticleSimulationBuilder, ParticleStyle, PointEmitter, PolygonCollider, RadialForce,
    RectEmitter, SdfCollider, Seek, SegmentCollider, SignedDistanceGrid, SimulationLimits, Spring,
    SubEmitter, Vortex, Wind,
};
use motion::prelude::*;
use std::sync::Arc;

const OBSTACLE: u32 = 0x426c59;

pub(super) fn gravity(parameters: DemoParameters) -> motion::Result<Study> {
    Ok(Study::new(
        base(parameters)
            .emitter(
                PointEmitter::new(Point::new(156.0, 248.0))
                    .rate(f64::from(parameters.count))
                    .speed(30.0)
                    .velocity(Vector2::new(22.0, -100.0))
                    .lifetime(3.seconds())
                    .color([0.3, 0.85, 0.65, 1.0]),
            )
            .modifier(Gravity::new(Vector2::new(0.0, 56.0)))
            .modifier(Wind::new(Vector2::new(16.0, 0.0)))
            .modifier(Drag::new(0.12)),
    ))
}

fn cloud(parameters: DemoParameters) -> ParticleSimulationBuilder {
    base(parameters).emitter(
        DiskEmitter::new(VIEWPORT.center(), 102.0)
            .rate(f64::from(parameters.count))
            .speed(12.0)
            .lifetime(3.8.seconds())
            .color([0.25, 0.85, 0.65, 1.0]),
    )
}

pub(super) fn attractor(parameters: DemoParameters) -> motion::Result<Study> {
    let mut study = Study::new(
        cloud(parameters)
            .modifier(Attractor {
                center: VIEWPORT.center(),
                strength: 14000.0,
            })
            .modifier(Drag::new(1.0)),
    );
    study.guides = Circle::new(VIEWPORT.center(), 4.0)?
        .fill(Color::from_hex(0xf6b863))
        .into();
    Ok(study)
}

pub(super) fn radial(parameters: DemoParameters) -> motion::Result<Study> {
    let mut simulation = base(parameters);
    let mut guides = Vec::new();
    let falloffs = [
        ForceFalloff::Constant,
        ForceFalloff::Linear,
        ForceFalloff::InverseSquare,
        ForceFalloff::Curve(ParticleCurve {
            stops: Arc::from([(0.0, 1.0), (0.5, 0.2), (1.0, 0.0)]),
            easing: Easing::CubicInOut,
        }),
    ];
    for (index, falloff) in falloffs.into_iter().enumerate() {
        let center = Point::new(68.0 + index as f64 * 124.0, 152.0);
        simulation = simulation
            .emitter(
                DiskEmitter::new(center, 46.0)
                    .rate(f64::from(parameters.count) / 4.0)
                    .speed(0.0)
                    .lifetime(3.8.seconds())
                    .color([0.3, 0.7, 1.0, 1.0]),
            )
            .modifier(RadialForce {
                center,
                strength: if index == 2 { 9000.0 } else { 22.0 },
                radius: 56.0,
                falloff,
            });
        guides.push(
            Circle::new(center, 56.0)?
                .fill(Color::from_hex(0x243b31))
                .into(),
        );
        guides.push(text(
            ["Constant", "Linear", "Inverse", "Curve"][index],
            13.0,
            0xb4c8bd,
            center.x - 26.0,
            224.0,
        ));
    }
    let mut study = Study::new(simulation.modifier(Drag::new(0.6)));
    study.guides = Node::group(guides);
    Ok(study)
}

pub(super) fn vortex(parameters: DemoParameters) -> motion::Result<Study> {
    Ok(Study::new(
        cloud(parameters)
            .modifier(Vortex {
                center: VIEWPORT.center(),
                strength: 28.0,
            })
            .modifier(Drag::new(0.2)),
    ))
}

pub(super) fn seek(parameters: DemoParameters) -> motion::Result<Study> {
    let targets: Arc<[Point]> = (0..parameters.count)
        .map(|index| {
            let angle = f64::from(index) / f64::from(parameters.count) * std::f64::consts::TAU;
            Point::new(
                256.0 + math::cos(angle) * 88.0,
                152.0 + math::sin(angle) * 88.0,
            )
        })
        .collect();
    Ok(Study::new(cloud(parameters).modifier(Seek {
        targets,
        strength: 2.4,
    })))
}

pub(super) fn curl(parameters: DemoParameters) -> motion::Result<Study> {
    Ok(Study::new(
        cloud(parameters)
            .modifier(CurlNoise {
                frequency: 0.025,
                strength: 14.0,
                seed: parameters.seed as u32,
                phase_speed: 0.4,
            })
            .modifier(Drag::new(0.3)),
    ))
}

pub(super) fn flow(parameters: DemoParameters) -> motion::Result<Study> {
    let image = Arc::new(ImageAsset::from_rgba8(
        2,
        2,
        vec![
            255, 160, 0, 255, 192, 255, 0, 255, 64, 128, 0, 255, 128, 32, 0, 255,
        ],
    )?);
    Ok(Study::new(
        cloud(parameters)
            .modifier(FlowField {
                source: FlowSource::Image {
                    image,
                    bounds: VIEWPORT,
                },
                strength: 26.0,
                phase_speed: 0.1,
            })
            .modifier(FlowField::curl(parameters.seed as u32, 0.02, 6.0))
            .modifier(Drag::new(0.3)),
    ))
}

pub(super) fn boids(parameters: DemoParameters) -> motion::Result<Study> {
    Ok(Study::new(
        cloud(parameters)
            .group_modifier(Boids {
                radius: 38.0,
                separation: 240.0,
                alignment: 1.8,
                cohesion: 0.6,
                ..Default::default()
            })
            .modifier(Vortex {
                center: VIEWPORT.center(),
                strength: 14.0,
            })
            .modifier(BounceBox::new(Rect::new(40.0, 36.0, 472.0, 268.0)))
            .style(ParticleStyle::Trails {
                length: 0.22.seconds(),
                width: 1.8,
            }),
    ))
}

pub(super) fn spring(parameters: DemoParameters) -> motion::Result<Study> {
    let mut simulation = base(parameters);
    for index in 0..24 {
        simulation = simulation.emitter(
            BurstEmitter::new(
                Time::ZERO,
                1,
                Point::new(
                    84.0 + index as f64 * 14.0,
                    152.0 + math::sin(index as f64 * 0.55) * 28.0,
                ),
            )
            .speed(12.0)
            .lifetime(SOURCE_SECONDS.seconds())
            .color([0.5, 0.75, 1.0, 1.0]),
        );
    }
    Ok(Study::new(
        simulation
            .group_modifier(Spring {
                rest_length: 14.0,
                stiffness: 16.0,
                damping: 1.8,
                closed: false,
            })
            .modifier(Drag::new(0.15))
            .style(ParticleStyle::Plexus {
                radius: 3.0,
                max_distance: 22.0,
            }),
    ))
}

pub(super) fn contacts(parameters: DemoParameters) -> motion::Result<Study> {
    Ok(Study::new(
        base(parameters)
            .emitter(
                DiskEmitter::new(VIEWPORT.center(), 72.0)
                    .rate(f64::from(parameters.count) * 0.4)
                    .speed(40.0)
                    .size(5.0)
                    .lifetime(3.5.seconds())
                    .color([0.9, 0.6, 0.3, 1.0]),
            )
            .group_modifier(ParticleCollisions {
                response: CollisionResponse {
                    restitution: 0.9,
                    ..Default::default()
                },
            })
            .modifier(BounceBox::new(Rect::new(64.0, 48.0, 448.0, 256.0)))
            .style(ParticleStyle::Dots { radius: 1.0 }),
    ))
}

pub(super) fn bounce(parameters: DemoParameters) -> motion::Result<Study> {
    let bounds = Rect::new(72.0, 52.0, 440.0, 252.0);
    let mut study = Study::new(
        base(parameters)
            .emitter(
                BurstEmitter::new(Time::ZERO, parameters.count as usize, VIEWPORT.center())
                    .speed(100.0)
                    .lifetime(SOURCE_SECONDS.seconds())
                    .color([0.3, 0.8, 1.0, 1.0]),
            )
            .modifier(BounceBox::new(bounds)),
    );
    study.guides = bounds.fill(Color::from_hex(0x243b31)).into();
    Ok(study)
}

fn falling(parameters: DemoParameters, rate: f64) -> ParticleSimulationBuilder {
    base(parameters)
        .emitter(
            RectEmitter::new(Rect::new(112.0, 16.0, 400.0, 18.0))
                .rate(rate)
                .speed(4.0)
                .velocity(Vector2::new(0.0, 72.0))
                .size(2.0)
                .lifetime(3.8.seconds())
                .color([0.3, 0.8, 1.0, 1.0]),
        )
        .modifier(Gravity::new(Vector2::new(0.0, 20.0)))
        .style(ParticleStyle::Dots { radius: 1.0 })
}

fn response() -> CollisionResponse {
    CollisionResponse {
        restitution: 0.75,
        friction: 0.1,
        kill_on_hit: false,
    }
}

pub(super) fn circle(parameters: DemoParameters) -> motion::Result<Study> {
    let center = Point::new(256.0, 192.0);
    let simulation = base(parameters)
        .emitter(
            RectEmitter::new(Rect::new(232.0, 16.0, 280.0, 18.0))
                .rate(f64::from(parameters.count))
                .speed(0.0)
                .velocity(Vector2::new(0.0, 72.0))
                .size(2.0)
                .lifetime(3.8.seconds())
                .color([0.3, 0.8, 1.0, 1.0]),
        )
        .modifier(Gravity::new(Vector2::new(0.0, 20.0)))
        .style(ParticleStyle::Dots { radius: 1.0 });
    let mut study = Study::new(simulation.modifier(CircleCollider {
        center,
        radius: 60.0,
        response: response(),
    }));
    study.guides = Circle::new(center, 60.0)?
        .fill(Color::from_hex(OBSTACLE))
        .into();
    Ok(study)
}

pub(super) fn polygon(parameters: DemoParameters) -> motion::Result<Study> {
    let vertices: Arc<[Point]> = Arc::from([
        Point::new(128.0, 224.0),
        Point::new(256.0, 136.0),
        Point::new(384.0, 224.0),
    ]);
    let path = Path2D::builder()
        .move_to(vertices[0])
        .line_to(vertices[1])
        .line_to(vertices[2])
        .close()
        .build()?;
    let mut study = Study::new(
        falling(parameters, f64::from(parameters.count))
            .modifier(PolygonCollider::new(vertices, response())?),
    );
    study.guides = path.fill(Color::from_hex(OBSTACLE)).into();
    Ok(study)
}

pub(super) fn sdf(parameters: DemoParameters) -> motion::Result<Study> {
    let tree = super::super::particle_recipes::logo(VIEWPORT)?;
    let grid = Arc::new(SignedDistanceGrid::new(
        tree.surface()?,
        VIEWPORT,
        [65, 39],
        SimulationLimits::default(),
    )?);
    let simulation = base(parameters)
        .emitter(
            RectEmitter::new(Rect::new(192.0, 16.0, 320.0, 18.0))
                .rate(f64::from(parameters.count) / 3.0)
                .speed(0.0)
                .velocity(Vector2::new(0.0, 72.0))
                .size(2.0)
                .lifetime(3.8.seconds())
                .color([0.3, 0.8, 1.0, 1.0]),
        )
        .modifier(Gravity::new(Vector2::new(0.0, 20.0)))
        .style(ParticleStyle::Dots { radius: 1.0 });
    let mut study = Study::new(simulation.modifier(SdfCollider {
        grid,
        response: response(),
    }));
    study.guides = Rect::new(179.2, 91.2, 332.8, 212.8)
        .fill(Color::from_hex(OBSTACLE))
        .into();
    Ok(study)
}

pub(super) fn events(parameters: DemoParameters) -> motion::Result<Study> {
    let mut study = Study::new(
        falling(parameters, f64::from(parameters.count) * 0.35)
            .modifier(SegmentCollider {
                start: Point::new(60.0, 244.0),
                end: Point::new(452.0, 244.0),
                response: CollisionResponse {
                    kill_on_hit: true,
                    ..response()
                },
            })
            .sub_emitter(SubEmitter {
                on: ParticleEventKind::Collision,
                emitter: Arc::new(SecondaryBurst(
                    BurstEmitter::new(Time::ZERO, 5, Point::ORIGIN)
                        .speed(38.0)
                        .velocity(Vector2::new(0.0, -30.0))
                        .lifetime(0.4.seconds()),
                )),
                inherit: Inherit::Color,
                probability: 1.0,
            }),
    );
    study.guides = Rect::new(60.0, 244.0, 452.0, 247.0)
        .fill(Color::from_hex(OBSTACLE))
        .into();
    Ok(study)
}

pub(super) fn lifecycle(parameters: DemoParameters) -> motion::Result<Study> {
    let mut simulation = base(parameters).emitter(
        BurstEmitter::new(
            Time::ZERO,
            parameters.count as usize / 3 + 1,
            VIEWPORT.center(),
        )
        .speed(22.0)
        .lifetime(2.6.seconds())
        .color([0.3, 0.8, 0.7, 1.0]),
    );
    for (on, color) in [
        (ParticleEventKind::Birth, [0.2, 0.6, 1.0, 1.0]),
        (ParticleEventKind::AgeReached(1.3), [1.0, 0.65, 0.2, 1.0]),
        (ParticleEventKind::Death, [1.0, 0.25, 0.2, 1.0]),
    ] {
        simulation = simulation.sub_emitter(SubEmitter {
            on,
            emitter: Arc::new(SecondaryBurst(
                BurstEmitter::new(Time::ZERO, 7, Point::ORIGIN)
                    .speed(40.0)
                    .lifetime(0.65.seconds())
                    .color(color),
            )),
            inherit: Inherit::Position,
            probability: 1.0,
        });
    }
    Ok(Study::new(simulation))
}

struct SecondaryBurst(BurstEmitter);

impl ParticleEmitter for SecondaryBurst {
    fn accepts_event(&self, event: &ParticleEvent) -> bool {
        event.particle.custom[3] == 0.0
    }

    fn emit(
        &self,
        step: u64,
        time: Time,
        random: &mut Random,
        output: &mut Vec<Particle>,
    ) -> Result<(), ValidationError> {
        let start = output.len();
        self.0.emit(step, time, random, output)?;
        for particle in &mut output[start..] {
            particle.custom[3] = 1.0;
        }
        Ok(())
    }
}
