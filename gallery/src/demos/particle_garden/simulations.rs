//! The four Particle Garden simulations (fireworks, murmuration, dissolve and embers)
//! together with their shared styles, curves, limits and collider geometry.

use super::behaviors::{Cruise, Dissolve, Leash, Placed, Pulse, Solid, Stage};
use super::{FLOOR, HEIGHT, SANS, WIDTH, color, seconds};
use motion::extend::DrawContext;
use motion::particles::{
    Boids, BokehShape, BurstEmitter, CollisionResponse, CurlNoise, DiskEmitter, Drag, FlowField,
    Gravity, ImageDensity, ImageEmitter, Inherit, IntensityOverLifetime, ParticleBlend,
    ParticleCurve, ParticleEventKind, ParticleSimulation, ParticleStyle, PathEmitter,
    PolygonCollider, ProceduralSprite, RectEmitter, SegmentCollider, SimulationLimits,
    SizeOverLifetime, SpriteAtlas, SubEmitter, SurfaceEmitter, Temperature, TrailOptions, Wind,
};
use motion::prelude::*;
use std::sync::Arc;

fn trail(length: f64, width: f64, segments: u8) -> ParticleStyle {
    ParticleStyle::Trails {
        length: Duration::from_seconds(length),
        width,
    }
    .trails(TrailOptions {
        segments,
        taper: Some(ParticleCurve::linear(0.0, 1.0)),
        fade: Some(ParticleCurve::linear(0.0, 1.0)),
        smooth: true,
        ribbon: true,
    })
    .blend(ParticleBlend::Plus)
}

fn limits() -> SimulationLimits {
    SimulationLimits {
        max_work: 8_000_000_000,
        ..SimulationLimits::default()
    }
}

fn curve(stops: &[(f32, f32)]) -> ParticleCurve {
    ParticleCurve {
        stops: Arc::from(stops),
        easing: Easing::Linear,
    }
}

fn closed(points: &[(f64, f64)]) -> Vec<Point> {
    points.iter().map(|&(x, y)| Point::new(x, y)).collect()
}

pub(super) fn slabs() -> [Vec<Point>; 3] {
    [
        closed(&[
            (110.0, 372.0),
            (330.0, 330.0),
            (338.0, 350.0),
            (118.0, 394.0),
        ]),
        closed(&[
            (610.0, 336.0),
            (850.0, 390.0),
            (844.0, 410.0),
            (604.0, 356.0),
        ]),
        closed(&[
            (448.0, 408.0),
            (480.0, 384.0),
            (512.0, 408.0),
            (480.0, 428.0),
        ]),
    ]
}

pub(super) fn fireworks(seed: u64, duration: Duration) -> motion::Result<ParticleSimulation> {
    let response = CollisionResponse {
        restitution: 0.48,
        friction: 0.12,
        kill_on_hit: false,
    };
    let mut simulation = ParticleSimulation::new(seed).duration(duration);
    let launches = [
        (0.05, 392.0, 540.0, [2.6, 0.9, 0.35, 1.0]),
        (0.4, 70.0, 500.0, [0.5, 1.5, 2.6, 1.0]),
        (0.75, 566.0, 520.0, [2.4, 0.5, 1.8, 1.0]),
        (1.1, 900.0, 505.0, [1.2, 2.5, 0.9, 1.0]),
    ];
    for (time, x, rise, tint) in launches {
        simulation = simulation.emitter(
            BurstEmitter::new(Time::from_seconds(time), 1, Point::new(x, FLOOR - 6.0))
                .lifetime(seconds(0.8))
                .speed(0.0)
                .velocity(Vector2::new(0.0, -rise))
                .color(tint)
                .size(1.2),
        );
    }
    simulation = simulation
        .modifier(Gravity::new(Vector2::new(0.0, 230.0)))
        .modifier(Drag::new(0.35))
        .modifier(IntensityOverLifetime::new(curve(&[
            (0.0, 2.2),
            (0.55, 1.3),
            (1.0, 0.2),
        ])))
        .modifier(Solid(SegmentCollider {
            start: Point::new(0.0, FLOOR),
            end: Point::new(WIDTH, FLOOR),
            response: CollisionResponse {
                kill_on_hit: true,
                ..response
            },
        }));
    for slab in slabs() {
        simulation = simulation.modifier(Solid(PolygonCollider::new(Arc::from(slab), response)?));
    }
    simulation
        .limits(limits())
        .sub_emitter(SubEmitter {
            on: ParticleEventKind::Death,
            emitter: Arc::new(Stage {
                parent: 0.0,
                stage: 1.0,
                burst: BurstEmitter::new(Time::ZERO, 150, Point::ORIGIN)
                    .lifetime(seconds(1.5))
                    .speed(135.0)
                    .size(1.4),
            }),
            inherit: Inherit::Color,
            probability: 1.0,
        })
        .sub_emitter(SubEmitter {
            on: ParticleEventKind::Death,
            emitter: Arc::new(Stage {
                parent: 0.0,
                stage: 1.0,
                burst: BurstEmitter::new(Time::ZERO, 60, Point::ORIGIN)
                    .lifetime(seconds(1.2))
                    .speed(70.0)
                    .size(1.2),
            }),
            inherit: Inherit::Color,
            probability: 1.0,
        })
        .sub_emitter(SubEmitter {
            on: ParticleEventKind::Death,
            emitter: Arc::new(Stage {
                parent: 0.0,
                stage: 1.0,
                burst: BurstEmitter::new(Time::ZERO, 40, Point::ORIGIN)
                    .lifetime(seconds(2.0))
                    .speed(30.0)
                    .size(1.6),
            }),
            inherit: Inherit::Color,
            probability: 1.0,
        })
        .sub_emitter(SubEmitter {
            on: ParticleEventKind::Collision,
            emitter: Arc::new(Stage {
                parent: 1.0,
                stage: 2.0,
                burst: BurstEmitter::new(Time::ZERO, 3, Point::ORIGIN)
                    .lifetime(seconds(0.35))
                    .speed(45.0)
                    .size(0.8),
            }),
            inherit: Inherit::Color,
            probability: 0.4,
        })
        .style(trail(0.14, 2.6, 12))
        .build()
        .map_err(Into::into)
}

pub(super) fn murmuration(seed: u64, duration: Duration) -> motion::Result<ParticleSimulation> {
    let palette: Arc<[[f32; 4]]> = Arc::from([
        [0.1, 0.75, 0.9, 1.0],
        [0.3, 0.5, 1.0, 1.0],
        [0.75, 0.35, 1.0, 1.0],
        [0.2, 0.95, 0.65, 1.0],
    ]);
    ParticleSimulation::new(seed + 1)
        .duration(duration)
        .emitter(
            DiskEmitter::new(Point::new(WIDTH * 0.5, HEIGHT * 0.5), 140.0)
                .rate(190.0)
                .speed(95.0)
                .lifetime(seconds(3.0))
                .palette(palette),
        )
        .group_modifier(Boids {
            radius: 28.0,
            separation: 420.0,
            alignment: 2.4,
            cohesion: 0.6,
            max_particles: 2048,
        })
        .modifier(FlowField::curl(seed as u32, 0.0045, 110.0))
        .modifier(Leash { strength: 0.9 })
        .modifier(Cruise {
            speed: 130.0,
            rate: 2.5,
        })
        .modifier(IntensityOverLifetime::new(curve(&[
            (0.0, 0.0),
            (0.12, 0.9),
            (0.85, 0.9),
            (1.0, 0.0),
        ])))
        .limits(limits())
        .prewarm(seconds(1.4))
        .style(trail(0.42, 3.0, 16))
        .build()
        .map_err(Into::into)
}

/// The one-pixel-wide parts of the flower: a five-petal rosette with a warm core.
fn flower(size: u32) -> motion::Result<Arc<ImageAsset>> {
    let mut pixels = Vec::with_capacity((size * size * 4) as usize);
    for row in 0..size {
        for column in 0..size {
            let x = (f64::from(column) + 0.5) / f64::from(size) * 2.0 - 1.0;
            let y = (f64::from(row) + 0.5) / f64::from(size) * 2.0 - 1.0;
            let radius = math::hypot(x, y);
            let angle = math::atan2(y, x);
            let edge = 0.62 + 0.3 * math::cos(angle * 5.0 + 0.6);
            let coverage = ((edge - radius) * f64::from(size) * 0.5).clamp(0.0, 1.0);
            let warmth = (1.0 - radius / edge.max(0.1)).clamp(0.0, 1.0);
            pixels.extend_from_slice(&[
                255,
                (120.0 + 120.0 * warmth) as u8,
                (170.0 - 100.0 * warmth) as u8,
                (coverage * 255.0) as u8,
            ]);
        }
    }
    Ok(Arc::new(ImageAsset::from_rgba8(size, size, pixels)?))
}

pub(super) fn dissolve(
    seed: u64,
    duration: Duration,
    context: &SceneContext<'_>,
) -> motion::Result<ParticleSimulation> {
    let atlas = SpriteAtlas::new_padded(
        &[
            ProceduralSprite::SoftCircle.generate(40, seed)?,
            ProceduralSprite::Star.generate(40, seed)?,
            ProceduralSprite::Ring.generate(40, seed)?,
            ProceduralSprite::Star.generate(24, seed + 1)?,
        ],
        1 << 20,
    )?
    .random(seed);
    let draw = DrawContext::default().with_assets(context.assets());
    let text = Text::new("garden")
        .family(SANS)
        .size(168.0)
        .weight(FontWeight::BOLD)
        .letter_spacing(-2.0)
        .fill(Color::WHITE);
    let text_box = context.measure_or(&Node::from(text.clone()), Rect::new(0.0, 0.0, 480.0, 168.0));
    let offset = [
        (WIDTH * 0.5 - text_box.width() * 0.5 - text_box.left) as f32,
        (178.0 - text_box.height() * 0.5 - text_box.top) as f32,
    ];
    let aurora: Paint = LinearGradient::new(Point::ORIGIN, Point::new(1.0, 0.0))
        .stop(0.0, color(0x30e8c8))
        .stop(0.5, color(0xffd870))
        .stop(1.0, color(0xff5a8a))
        .into();
    let petals: Paint = LinearGradient::new(Point::ORIGIN, Point::new(0.0, 1.0))
        .stop(0.0, color(0xffffff))
        .stop(1.0, color(0xff8fb0))
        .into();
    let ribbon: Paint = LinearGradient::new(Point::ORIGIN, Point::new(1.0, 0.0))
        .stop(0.0, color(0x6a8cff))
        .stop(1.0, color(0x30e8c8))
        .into();
    let text_emitter = SurfaceEmitter::from_text(&text, &draw)?
        .rate_animation(Pulse {
            start: 0.0,
            end: 0.9,
            peak: 3200.0,
            fade: 0.35,
        })
        .speed(0.0)
        .lifetime(seconds(1.5))
        .size(0.9)
        .paint(aurora, text_box)?;
    let bloom_bounds = Rect::new(96.0, 300.0, 296.0, 500.0);
    let flower_emitter = ImageEmitter::new(
        flower(96)?,
        bloom_bounds,
        ImageDensity::Alpha,
        SimulationLimits::default(),
    )?
    .rate_animation(Pulse {
        start: 0.35,
        end: 1.5,
        peak: 1500.0,
        fade: 0.4,
    })
    .speed(0.0)
    .lifetime(seconds(1.5))
    .size(0.9)
    .paint(petals, bloom_bounds)?;
    let path = Arc::new(Path2D::parse_svg(
        "M 360 470 C 460 300 560 520 660 380 S 820 330 900 420",
    )?);
    let path_emitter = PathEmitter::new(path)
        .rate_animation(Pulse {
            start: 0.7,
            end: 1.9,
            peak: 900.0,
            fade: 0.5,
        })
        .speed(6.0)
        .lifetime(seconds(1.6))
        .size(0.9)
        .paint(ribbon, Rect::new(360.0, 300.0, 900.0, 520.0))?;
    ParticleSimulation::new(seed + 2)
        .duration(duration)
        .emitter(Placed {
            inner: Box::new(text_emitter),
            offset,
        })
        .emitter(flower_emitter)
        .emitter(path_emitter)
        .modifier(Dissolve {
            field: FlowField::curl(seed as u32 + 3, 0.012, 420.0),
        })
        .modifier(Wind::new(Vector2::new(0.0, -26.0)))
        .modifier(Drag::new(0.5))
        .modifier(SizeOverLifetime {
            start: 1.0,
            end: 0.25,
        })
        .modifier(IntensityOverLifetime::new(curve(&[
            (0.0, 1.5),
            (0.5, 1.0),
            (1.0, 0.0),
        ])))
        .style(atlas.style(13.0, false)?.blend(ParticleBlend::Plus))
        .build()
        .map_err(Into::into)
}

pub(super) fn embers(
    seed: u64,
    duration: Duration,
) -> motion::Result<(ParticleSimulation, ParticleSimulation)> {
    let bottom = Rect::new(0.0, HEIGHT - 6.0, WIDTH, HEIGHT);
    let dust = ParticleSimulation::new(seed + 4)
        .duration(duration)
        .emitter(
            RectEmitter::new(Rect::new(0.0, 0.0, WIDTH, HEIGHT))
                .rate(34.0)
                .speed(5.0)
                .lifetime(seconds(5.0))
                .depth_range(-2.0, 2.0)
                .color([0.55, 0.3, 0.1, 0.55]),
        )
        .modifier(CurlNoise {
            frequency: 0.004,
            strength: 6.0,
            seed: seed as u32 + 4,
            phase_speed: 1.0,
        })
        .prewarm(seconds(2.0))
        .style(
            ParticleStyle::Bokeh {
                radius: 4.5,
                aperture: 0.55,
                focus: 0.0,
                shape: BokehShape::Polygon(6),
                ring: 0.5,
                hardness: 0.6,
                highlight: 1.8,
            }
            .parallax([14.0, 5.0])
            .blend(ParticleBlend::Screen),
        )
        .build()?;

    let sparks = ParticleSimulation::new(seed + 5)
        .duration(duration)
        .emitter(
            RectEmitter::new(bottom)
                .rate(120.0)
                .speed(30.0)
                .velocity(Vector2::new(0.0, -85.0))
                .lifetime(seconds(4.2)),
        )
        .modifier(CurlNoise {
            frequency: 0.009,
            strength: 55.0,
            seed: seed as u32 + 5,
            phase_speed: 1.0,
        })
        .modifier(Wind::new(Vector2::new(8.0, -30.0)))
        .modifier(Temperature::new(curve(&[
            (0.0, 3900.0),
            (0.35, 2500.0),
            (1.0, 1100.0),
        ])))
        .modifier(IntensityOverLifetime::new(curve(&[
            (0.0, 2.6),
            (0.6, 1.2),
            (1.0, 0.0),
        ])))
        .prewarm(seconds(2.0))
        .style(ParticleStyle::GlowDots { radius: 2.2 }.blend(ParticleBlend::Plus))
        .build()?;
    Ok((dust, sparks))
}
