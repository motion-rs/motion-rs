//! Built-in particle geometry, materials, compositing, lifetime curves and custom presentation.

use super::{Specimen, Study, VIEWPORT, base, text};
use crate::DemoParameters;
use motion::particles::{
    BokehShape, ColorField, ColorOverLifetime, CurlNoise, DiskEmitter, FieldCoordinate, Gravity,
    IntensityOverLifetime, Particle, ParticleBlend, ParticleCurve, ParticleModifier, ParticleShape,
    ParticleSimulationBuilder, ParticleStyle, PointEmitter, ProceduralSprite, RectEmitter,
    SizeOverLifetime, SpriteAtlas, SpriteSheetMode, Temperature, TrailOptions,
};
use motion::prelude::*;
use motion::render::RasterLimits;
use std::sync::Arc;

/// Maximum retained bytes admitted when packing each small specimen atlas.
const ATLAS_BYTES: u64 = 4 * 1024 * 1024;

pub(super) fn shapes(parameters: DemoParameters) -> motion::Result<Study> {
    let shapes = [
        ParticleShape::Polygon { sides: 5 },
        ParticleShape::Ring { thickness: 0.22 },
        ParticleShape::Capsule { aspect: 2.4 },
        ParticleShape::Soft { hardness: 0.4 },
        ParticleShape::Quad { aspect: 1.5 },
    ];
    let layers = shapes
        .into_iter()
        .enumerate()
        .map(|(index, shape)| {
            let simulation = base(parameters)
                .emitter(
                    DiskEmitter::new(Point::new(56.0 + index as f64 * 100.0, 152.0), 28.0)
                        .rate(f64::from(parameters.count) / 8.0)
                        .speed(8.0)
                        .lifetime(3.seconds())
                        .color([0.3, 0.75, 0.95, 1.0]),
                )
                .style(ParticleStyle::Shape { radius: 5.0, shape });
            (simulation.build(), Point::ORIGIN)
        })
        .collect();
    Ok(Study {
        source: Specimen::Layers(layers),
        guides: Node::empty(),
    })
}

fn streaming(parameters: DemoParameters) -> ParticleSimulationBuilder {
    base(parameters)
        .emitter(
            PointEmitter::new(Point::new(100.0, 208.0))
                .rate(f64::from(parameters.count) * 0.5)
                .speed(22.0)
                .velocity(Vector2::new(80.0, -26.0))
                .lifetime(3.2.seconds())
                .color([0.3, 0.85, 0.7, 1.0]),
        )
        .modifier(CurlNoise {
            frequency: 0.02,
            strength: 8.0,
            seed: parameters.seed as u32,
            phase_speed: 0.3,
        })
}

fn trail_style(ribbon: bool) -> ParticleStyle {
    ParticleStyle::Trails {
        length: 0.65.seconds(),
        width: 3.2,
    }
    .trails(TrailOptions {
        segments: 16,
        taper: Some(ParticleCurve::linear(0.0, 1.0)),
        fade: Some(ParticleCurve::linear(0.0, 1.0)),
        smooth: true,
        ribbon,
    })
    .blend(ParticleBlend::Plus)
}

pub(super) fn trails(parameters: DemoParameters) -> motion::Result<Study> {
    Ok(Study::new(streaming(parameters).style(trail_style(false))))
}

pub(super) fn ribbons(parameters: DemoParameters) -> motion::Result<Study> {
    Ok(Study::new(streaming(parameters).style(trail_style(true))))
}

pub(super) fn plexus(parameters: DemoParameters) -> motion::Result<Study> {
    Ok(Study::new(
        base(parameters)
            .emitter(
                RectEmitter::new(Rect::new(112.0, 64.0, 400.0, 240.0))
                    .rate(f64::from(parameters.count) * 0.5)
                    .speed(16.0)
                    .lifetime(3.5.seconds())
                    .color([0.25, 0.75, 0.85, 1.0]),
            )
            .style(ParticleStyle::Plexus {
                radius: 2.0,
                max_distance: 46.0,
            }),
    ))
}

pub(super) fn bokeh(parameters: DemoParameters) -> motion::Result<Study> {
    let layers = [BokehShape::Disc, BokehShape::Polygon(6)]
        .into_iter()
        .enumerate()
        .map(|(index, shape)| {
            (
                base(parameters)
                    .emitter(
                        RectEmitter::new(Rect::new(
                            80.0 + index as f64 * 220.0,
                            64.0,
                            210.0 + index as f64 * 220.0,
                            240.0,
                        ))
                        .rate(f64::from(parameters.count) * 0.25)
                        .speed(8.0)
                        .depth_range(-2.0, 2.0)
                        .lifetime(3.8.seconds())
                        .color([0.6, 0.45, 0.2, 0.75]),
                    )
                    .style(
                        ParticleStyle::Bokeh {
                            radius: 4.0,
                            aperture: 4.0,
                            focus: 0.0,
                            shape,
                            ring: 0.4,
                            hardness: 0.6,
                            highlight: 1.4,
                        }
                        .parallax([10.0, 0.0])
                        .blend(ParticleBlend::Screen),
                    )
                    .build(),
                Point::ORIGIN,
            )
        })
        .collect();
    Ok(Study {
        source: Specimen::Layers(layers),
        guides: Node::empty(),
    })
}

pub(super) fn blends(parameters: DemoParameters) -> motion::Result<Study> {
    let modes = [
        ParticleBlend::SourceOver,
        ParticleBlend::Plus,
        ParticleBlend::Screen,
        ParticleBlend::Multiply,
    ];
    let mut guides = vec![
        Rect::new(20.0, 64.0, 492.0, 228.0)
            .fill(Color::from_hex(0xc9d9cf))
            .into(),
    ];
    let layers = modes
        .into_iter()
        .enumerate()
        .map(|(index, blend)| {
            let center = Point::new(68.0 + index as f64 * 124.0, 146.0);
            guides.push(text(
                ["Over", "Plus", "Screen", "Multiply"][index],
                13.0,
                0xb4c8bd,
                center.x - 24.0,
                248.0,
            ));
            (
                base(parameters)
                    .emitter(
                        DiskEmitter::new(center, 28.0)
                            .rate(f64::from(parameters.count) / 8.0)
                            .speed(4.0)
                            .lifetime(3.8.seconds())
                            .color([0.05, 0.2, 0.35, 0.5]),
                    )
                    .style(ParticleStyle::Dots { radius: 10.0 }.blend(blend))
                    .build(),
                Point::ORIGIN,
            )
        })
        .collect();
    Ok(Study {
        source: Specimen::Layers(layers),
        guides: Node::group(guides),
    })
}

fn materials(seed: u64) -> Result<SpriteAtlas, ValidationError> {
    SpriteAtlas::new_padded(
        &[
            ProceduralSprite::SoftCircle.generate(40, seed)?,
            ProceduralSprite::Smoke {
                scale: 3.0,
                evolution: 0.0,
                fractal: motion::noise::Fractal::default(),
            }
            .generate(40, seed)?,
            ProceduralSprite::Star.generate(32, seed)?,
            ProceduralSprite::Ring.generate(40, seed)?,
            ProceduralSprite::FireCore.generate(32, seed)?,
        ],
        ATLAS_BYTES,
    )
}

fn sprites(parameters: DemoParameters, style: ParticleStyle) -> ParticleSimulationBuilder {
    base(parameters)
        .emitter(
            RectEmitter::new(Rect::new(96.0, 72.0, 416.0, 232.0))
                .rate(f64::from(parameters.count) / 3.0)
                .speed(10.0)
                .lifetime(3.8.seconds())
                .color([0.6, 0.85, 1.0, 1.0]),
        )
        .style(style)
}

pub(super) fn atlas(parameters: DemoParameters) -> motion::Result<Study> {
    let atlas = materials(parameters.seed)?.random(parameters.seed);
    Ok(Study::new(sprites(parameters, atlas.style(34.0, false)?)))
}

pub(super) fn sheets(parameters: DemoParameters) -> motion::Result<Study> {
    let source = materials(parameters.seed)?;
    let modes = [
        SpriteSheetMode::OverLifetime,
        SpriteSheetMode::Fps(2.0),
        SpriteSheetMode::Random(parameters.seed),
        SpriteSheetMode::Custom,
    ];
    let mut guides = Vec::new();
    let mut layers = Vec::new();
    for (index, mode) in modes.into_iter().enumerate() {
        let mut atlas = source.clone();
        atlas.sheet.mode = mode;
        let center = Point::new(68.0 + index as f64 * 124.0, 152.0);
        layers.push((
            base(parameters)
                .emitter(
                    DiskEmitter::new(center, 28.0)
                        .rate(f64::from(parameters.count) / 12.0)
                        .speed(5.0)
                        .lifetime(3.8.seconds()),
                )
                .modifier(AtlasPhase)
                .style(atlas.style(32.0, false)?)
                .build(),
            Point::ORIGIN,
        ));
        guides.push(text(
            ["Lifetime", "FPS", "Random", "Custom"][index],
            13.0,
            0xb4c8bd,
            center.x - 26.0,
            228.0,
        ));
    }
    Ok(Study {
        source: Specimen::Layers(layers),
        guides: Node::group(guides),
    })
}

pub(super) fn smoke(parameters: DemoParameters) -> motion::Result<Study> {
    let mut images = Vec::new();
    for phase in 0..8 {
        images.push(
            ProceduralSprite::Smoke {
                scale: 3.0,
                evolution: 1.0,
                fractal: motion::noise::Fractal::default(),
            }
            .generate_at(
                48,
                parameters.seed,
                (phase as f64 * 0.2).seconds().after_origin(),
                RasterLimits::default(),
            )?,
        );
    }
    let mut atlas = SpriteAtlas::new(&images, ATLAS_BYTES)?;
    atlas.sheet.mode = SpriteSheetMode::Fps(4.0);
    Ok(Study::new(sprites(parameters, atlas.style(54.0, false)?)))
}

pub(super) fn stretch(parameters: DemoParameters) -> motion::Result<Study> {
    let capsule = base(parameters)
        .emitter(
            RectEmitter::new(Rect::new(92.0, 24.0, 220.0, 26.0))
                .rate(f64::from(parameters.count) * 0.4)
                .speed(4.0)
                .velocity(Vector2::new(0.0, 70.0))
                .lifetime(3.2.seconds())
                .color([0.3, 0.7, 1.0, 1.0]),
        )
        .style(
            ParticleStyle::Shape {
                radius: 2.0,
                shape: ParticleShape::Capsule { aspect: 1.0 },
            }
            .stretch_by_speed(0.06, 8.0),
        );
    let quads = base(parameters)
        .emitter(
            RectEmitter::new(Rect::new(292.0, 24.0, 420.0, 26.0))
                .rate(f64::from(parameters.count) * 0.4)
                .speed(10.0)
                .velocity(Vector2::new(0.0, 30.0))
                .lifetime(3.2.seconds())
                .color([1.0, 0.55, 0.2, 1.0]),
        )
        .modifier(Gravity::new(Vector2::new(0.0, 18.0)))
        .modifier(AtlasPhase)
        .style(
            ParticleStyle::Shape {
                radius: 4.0,
                shape: ParticleShape::Quad { aspect: 1.6 },
            }
            .flip(),
        );
    Ok(Study {
        source: Specimen::Layers(vec![
            (capsule.build(), Point::ORIGIN),
            (quads.build(), Point::ORIGIN),
        ]),
        guides: Node::empty(),
    })
}

pub(super) fn lifetime(parameters: DemoParameters) -> motion::Result<Study> {
    Ok(Study::new(
        base(parameters)
            .emitter(
                PointEmitter::new(Point::new(80.0, 152.0))
                    .rate(f64::from(parameters.count) / 3.0)
                    .speed(8.0)
                    .velocity(Vector2::new(85.0, 0.0))
                    .lifetime(3.5.seconds()),
            )
            .modifier(SizeOverLifetime {
                start: 0.5,
                end: 2.5,
            })
            .modifier(
                ColorOverLifetime::new(Arc::from([[0.1, 0.8, 1.0, 1.0], [1.0, 0.3, 0.1, 1.0]]))
                    .curve(Easing::CubicInOut),
            )
            .modifier(IntensityOverLifetime::new(ParticleCurve::linear(1.5, 0.2)))
            .style(ParticleStyle::GlowDots { radius: 2.4 }),
    ))
}

pub(super) fn color(parameters: DemoParameters) -> motion::Result<Study> {
    let colors = ColorOverLifetime::new(Arc::from([[0.1, 0.8, 1.0, 1.0], [1.0, 0.3, 0.1, 1.0]]))
        .curve(Easing::Linear);
    let mut layers = Vec::new();
    for (index, coordinate) in [
        FieldCoordinate::X,
        FieldCoordinate::Y,
        FieldCoordinate::Speed,
    ]
    .into_iter()
    .enumerate()
    {
        layers.push((
            base(parameters)
                .emitter(
                    DiskEmitter::new(Point::new(68.0 + index as f64 * 124.0, 152.0), 36.0)
                        .rate(f64::from(parameters.count) / 12.0)
                        .speed(18.0)
                        .lifetime(3.5.seconds()),
                )
                .modifier(Gravity::new(Vector2::new(0.0, 8.0)))
                .modifier(ColorField {
                    coordinate,
                    range: if index == 0 {
                        [24.0, 112.0]
                    } else if index == 1 {
                        [100.0, 200.0]
                    } else {
                        [0.0, 40.0]
                    },
                    colors: colors.clone(),
                })
                .style(ParticleStyle::Dots { radius: 4.0 })
                .build(),
            Point::ORIGIN,
        ));
    }
    layers.push((
        base(parameters)
            .emitter(
                DiskEmitter::new(Point::new(440.0, 152.0), 36.0)
                    .rate(f64::from(parameters.count) / 12.0)
                    .speed(12.0)
                    .lifetime(3.5.seconds()),
            )
            .modifier(Temperature::new(ParticleCurve::linear(8500.0, 1200.0)))
            .style(ParticleStyle::GlowDots { radius: 3.2 })
            .build(),
        Point::ORIGIN,
    ));
    Ok(Study {
        source: Specimen::Layers(layers),
        guides: Node::empty(),
    })
}

pub(super) fn custom(parameters: DemoParameters) -> motion::Result<Study> {
    Ok(Study::new(
        base(parameters)
            .emitter(
                DiskEmitter::new(VIEWPORT.center(), 88.0)
                    .rate(f64::from(parameters.count) / 2.0)
                    .speed(12.0)
                    .lifetime(3.8.seconds()),
            )
            .style(ParticleStyle::Custom(Arc::new(|particles, _| {
                Ok(Node::group(
                    particles
                        .iter()
                        .map(|particle| {
                            Ok(Circle::new(
                                Point::new(
                                    f64::from(particle.position[0]),
                                    f64::from(particle.position[1]),
                                ),
                                3.0 + math::sin(f64::from(particle.age) * 4.0),
                            )?
                            .fill(Color::from_hex(0x67dabb))
                            .into())
                        })
                        .collect::<Result<Vec<Node>, ValidationError>>()?,
                ))
            }))),
    ))
}

struct AtlasPhase;

impl ParticleModifier for AtlasPhase {
    fn apply(
        &self,
        particle: &mut Particle,
        _: f32,
        _: Time,
        _: &mut Random,
    ) -> Result<(), ValidationError> {
        particle.custom[0] = (particle.index % 5) as f32;
        particle.custom[2] = particle.index as f32 * 0.7;
        Ok(())
    }
}
