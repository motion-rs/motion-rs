use super::*;
use crate::asset::AssetsExt;
use motion_core::Shape;
use motion_core::{
    DrawCommand, NodeModifiers, PixelSize, RasterLimits, Size, TimeLiteral, ValidationLimits,
};
use motion_core::{Drawable, Point, Transform};
use motion_render::RenderPrepare;

fn original_coverage(batch: &ParticleBatch, point: Point, antialias: f64) -> f32 {
    let instance = &batch.instances[0];
    let particle = instance.particle;
    let radius = f64::from(batch.radius * particle.size);
    let relative = [
        point.x - f64::from(particle.position[0]),
        point.y - f64::from(particle.position[1]),
    ];
    if instance.line {
        let start = instance.previous.map(f64::from);
        let end = particle.position.map(f64::from);
        let delta = [end[0] - start[0], end[1] - start[1]];
        let square = delta[0] * delta[0] + delta[1] * delta[1];
        let progress = if square == 0.0 {
            0.0
        } else {
            ((point.x - start[0]) * delta[0] + (point.y - start[1]) * delta[1]) / square
        }
        .clamp(0.0, 1.0);
        let distance = libm::hypot(
            point.x - start[0] - delta[0] * progress,
            point.y - start[1] - delta[1] * progress,
        );
        ((f64::from(batch.width) * 0.5 - distance) / antialias + 0.5).clamp(0.0, 1.0) as f32
    } else if batch.kind == 1 {
        let square = (relative[0] * relative[0] + relative[1] * relative[1]) / (radius * radius);
        if square > 9.0 {
            0.0
        } else {
            libm::exp(-2.0 * square) as f32
        }
    } else {
        ((radius - libm::hypot(relative[0], relative[1])) / antialias + 0.5).clamp(0.0, 1.0) as f32
    }
}

#[test]
fn bokeh_focus_and_history_capacity_are_explicit() -> motion_core::Result {
    let cache = motion_core::SceneCache::new(0);
    let context = SceneContext::new(
        250.milliseconds().after_origin(),
        motion_core::fps!(30),
        Size::new(32.0, 24.0),
        &cache,
    );
    let focused = Particle {
        position: [10.0, 10.0],
        color: [8.0, 4.0, 2.0, 1.0],
        size: 2.0,
        index: 1,
        custom: [0.0, 2.0, 0.0, 0.0],
        ..Default::default()
    };
    let distant = Particle {
        index: 0,
        custom: [0.0, 4.0, 0.0, 0.0],
        ..focused
    };
    let states: Arc<[Particle]> = Arc::from([focused, distant]);
    let node = render(
        &ParticleStyle::Bokeh {
            radius: 1.0,
            shape: BokehShape::Disc,
            ring: 0.5,
            hardness: 0.2,
            focus: 2.0,
            aperture: 1.0,
            highlight: 1.0,
        },
        &context,
        states.clone(),
        |_| Ok(states.clone()),
    )?;
    let list = node.compile(
        Size::new(32.0, 24.0),
        PixelSize::new(32, 24),
        ValidationLimits::default(),
    )?;
    let upload = list
        .commands
        .iter()
        .find_map(|command| {
            if let DrawCommand::ParticleInstances { batch, .. } = command {
                Some(batch.upload().unwrap().unwrap())
            } else {
                None
            }
        })
        .unwrap();
    let words = upload.into_words();
    let distant = &words[4..20];
    let focused = &words[20..36];
    assert_eq!(distant[8], 6.0);
    assert_eq!(focused[8], 2.0);
    assert!((distant[4] - 8.0 / 9.0).abs() < 1e-6);
    assert_eq!(focused[4], 8.0);

    let instances: Arc<[Particle]> = (0..8_000)
        .map(|index| Particle {
            index,
            ..Default::default()
        })
        .collect::<Vec<_>>()
        .into();
    let samples = std::cell::Cell::new(0);
    assert!(
        render(
            &ParticleStyle::Trails {
                length: 100.milliseconds(),
                width: 1.0,
            }
            .trails(TrailOptions {
                segments: 64,
                ..Default::default()
            }),
            &context,
            instances.clone(),
            |_| {
                samples.set(samples.get() + 1);
                Ok(instances.clone())
            },
        )
        .is_err()
    );
    assert!(samples.get() < 65);
    Ok(())
}

#[test]
fn configured_eight_segment_trails_match_plain_trail_upload_bytes() -> motion_core::Result {
    let cache = motion_core::SceneCache::new(0);
    let context = SceneContext::new(
        500.milliseconds().after_origin(),
        motion_core::fps!(30),
        Size::new(32.0, 24.0),
        &cache,
    )
    .with_clip_duration(1.seconds());
    let system = ParticleSimulation::new(7)
        .duration(1.seconds())
        .emitter(BurstEmitter::new(Time::ZERO, 3, Point::new(16.0, 12.0)))
        .build()?;
    let plain = ParticleStyle::Trails {
        length: 100.milliseconds(),
        width: 1.0,
    };
    let first = system
        .clone()
        .to_builder()
        .style(plain.clone())
        .build()?
        .try_render(&context)?;
    let second = system
        .to_builder()
        .style(plain.trails(TrailOptions {
            segments: 8,
            ..Default::default()
        }))
        .build()?
        .try_render(&context)?;
    let first = first.compile(
        Size::new(32.0, 24.0),
        PixelSize::new(32, 24),
        ValidationLimits::default(),
    )?;
    let second = second.compile(
        Size::new(32.0, 24.0),
        PixelSize::new(32, 24),
        ValidationLimits::default(),
    )?;
    let uploads = |list: &motion_core::DisplayList| {
        list.commands
            .iter()
            .filter_map(|command| {
                if let DrawCommand::ParticleInstances { batch, .. } = command {
                    Some(batch.upload().unwrap().unwrap().into_words())
                } else {
                    None
                }
            })
            .collect::<Vec<_>>()
    };
    assert_eq!(uploads(&first), uploads(&second));
    Ok(())
}

#[test]
fn support_checks_preserve_original_samples_and_compositor_errors() -> motion_core::Result {
    fn capture(
        destination: &mut [f32; 4],
        source: [f32; 4],
        _: motion_core::BlendSpace,
    ) -> Result<(), ValidationError> {
        *destination = source;
        Ok(())
    }

    fn reject(
        _: &mut [f32; 4],
        _: [f32; 4],
        _: motion_core::BlendSpace,
    ) -> Result<(), ValidationError> {
        Err(invalid("test", "compositor still receives zero coverage"))
    }
    for kind in [0, 1, 3, 4] {
        for position in [[0.0, 0.0], [1e20, -1e20], [1e-20, -1e-20]] {
            for size in [1.0, 1e-20, 4096.0] {
                for width in [0.0, 0.75, 12.0] {
                    let particle = Particle {
                        position,
                        size,
                        color: [-0.4, 1.3, -0.0, 0.5],
                        ..Particle::default()
                    };
                    for previous in [position, [-6.0, 3.0], [3.0, 0.0]] {
                        let batch = ParticleBatch {
                            instances: vec![Instance::new(particle, previous, kind == 3)].into(),
                            kind,
                            radius: 2.0,
                            width,
                            align: false,
                            sprite: None,
                            bounds: Rect::default(),
                            index: Arc::from([]),
                        };
                        for antialias in
                            [1e-10, 0.125, 1.0, 8.0, 0.0, -1.0, f64::INFINITY, f64::NAN]
                        {
                            let reach = if kind == 3 {
                                f64::from(width) * 0.5
                            } else {
                                f64::from(2.0 * size)
                            };
                            for offset in [
                                -1e6,
                                -8.0,
                                -0.5,
                                0.0,
                                0.5,
                                8.0,
                                1e6,
                                reach,
                                reach + antialias * 0.5,
                            ] {
                                for horizontal in [false, true] {
                                    let point = Point::new(
                                        f64::from(position[0])
                                            + if horizontal { offset } else { 0.0 },
                                        f64::from(position[1])
                                            + if horizontal { 0.0 } else { offset },
                                    );
                                    let coverage = original_coverage(&batch, point, antialias);
                                    let expected = particle.color.map(|channel| channel * coverage);
                                    let actual = batch.sample(
                                        point,
                                        antialias,
                                        [0.0; 4],
                                        motion_core::BlendSpace::LinearRgb,
                                        capture,
                                    )?;
                                    assert_eq!(
                                        actual.map(f32::to_bits),
                                        expected.map(f32::to_bits)
                                    );
                                }
                            }
                        }
                        assert!(
                            batch
                                .sample(
                                    Point::new(1e30, 1e30),
                                    1.0,
                                    [0.0; 4],
                                    motion_core::BlendSpace::Srgb,
                                    reject
                                )
                                .is_err()
                        );
                    }
                }
            }
        }
    }
    Ok(())
}

#[test]
fn zero_identity_sampling_preserves_order_errors_and_signed_zero_destination() -> motion_core::Result
{
    fn composite(
        destination: &mut [f32; 4],
        source: [f32; 4],
        _: motion_core::BlendSpace,
    ) -> Result<(), ValidationError> {
        if source == [0.0; 4] {
            return Ok(());
        }
        if source.iter().any(|channel| !channel.is_finite()) {
            return Err(invalid("test", "nonfinite source"));
        }
        for channel in 0..4 {
            destination[channel] = source[channel] + destination[channel] * (1.0 - source[3]);
        }
        Ok(())
    }

    fn reject_nonzero(
        _: &mut [f32; 4],
        source: [f32; 4],
        _: motion_core::BlendSpace,
    ) -> Result<(), ValidationError> {
        if source == [0.0; 4] {
            Ok(())
        } else {
            Err(invalid("test", "nonzero source remains observable"))
        }
    }
    for kind in [0, 1, 3, 4] {
        let instances = (0..5)
            .map(|index| {
                let particle = Particle {
                    position: [index as f32 * 3.0, 0.0],
                    color: [index as f32 * 0.025, 0.125, -0.0, 0.25],
                    ..Particle::default()
                };
                Instance::new(particle, [-2.0, 1.0], kind == 3)
            })
            .collect::<Vec<_>>();
        let batch = ParticleBatch {
            instances: instances.into(),
            kind,
            radius: 2.0,
            width: 1.0,
            align: false,
            sprite: None,
            bounds: Rect::default(),
            index: Arc::from([]),
        };
        for space in [
            motion_core::BlendSpace::LinearRgb,
            motion_core::BlendSpace::Srgb,
        ] {
            for destination in [[-0.0; 4], [-0.25, 2.0, 0.125, 0.5]] {
                for row in -4..=4 {
                    for column in -4..=16 {
                        let point = Point::new(column as f64 * 0.5, row as f64 * 0.5);
                        let expected = batch.sample(point, 1.0, destination, space, composite)?;
                        let actual = batch.sample_with_zero_policy::<true>(
                            point,
                            1.0,
                            destination,
                            space,
                            composite,
                        )?;
                        assert_eq!(actual.map(f32::to_bits), expected.map(f32::to_bits));
                    }
                }
                let point = Point::new(1e6, 1e6);
                assert_eq!(
                    batch
                        .sample_with_zero_policy::<true>(point, 1.0, destination, space, composite)?
                        .map(f32::to_bits),
                    destination.map(f32::to_bits)
                );
            }
        }
        let batch: &dyn motion_core::instances::InstanceBatch = &batch;
        assert!(
            batch
                .sample_with_zero_identity(
                    Point::ORIGIN,
                    1.0,
                    [0.0; 4],
                    motion_core::BlendSpace::LinearRgb,
                    reject_nonzero
                )
                .is_err()
        );
        assert!(
            batch
                .sample_with_zero_identity(
                    Point::new(1e6, 1e6),
                    1.0,
                    [0.0; 4],
                    motion_core::BlendSpace::LinearRgb,
                    reject_nonzero
                )
                .is_ok()
        );
        assert_eq!(
            batch
                .sample_with_zero_identity(
                    Point::ORIGIN,
                    f64::NAN,
                    [0.0; 4],
                    motion_core::BlendSpace::LinearRgb,
                    composite
                )
                .map(|pixel| pixel.map(f32::to_bits)),
            batch
                .sample(
                    Point::ORIGIN,
                    f64::NAN,
                    [0.0; 4],
                    motion_core::BlendSpace::LinearRgb,
                    composite
                )
                .map(|pixel| pixel.map(f32::to_bits))
        );
    }
    Ok(())
}

#[test]
fn spatial_index_preserves_order_and_charges_retained_storage() -> motion_core::Result {
    fn composite(
        destination: &mut [f32; 4],
        source: [f32; 4],
        _: motion_core::BlendSpace,
    ) -> Result<(), ValidationError> {
        if source != [0.0; 4] {
            for channel in 0..4 {
                destination[channel] = source[channel] + destination[channel] * (1.0 - source[3]);
            }
        }
        Ok(())
    }
    for kind in [0, 1, 3, 4] {
        for scale in [1.0, 1e-20, 1e20] {
            let instances = (0..96)
                .map(|index| {
                    let particle = Particle {
                        position: [
                            (index % 12) as f32 * 4.0 * scale,
                            (index / 12) as f32 * 4.0 * scale,
                        ],
                        size: 1.0,
                        color: [index as f32 * 0.001, 0.125, -0.0, 0.25],
                        ..Particle::default()
                    };
                    Instance::new(
                        particle,
                        [
                            particle.position[0] - 3.0 * scale,
                            particle.position[1] + scale,
                        ],
                        kind == 3,
                    )
                })
                .collect::<Vec<_>>();
            let mut batch = ParticleBatch {
                instances: instances.into(),
                kind,
                radius: 2.0 * scale,
                width: scale,
                align: false,
                sprite: None,
                bounds: Rect::default(),
                index: Arc::from([]),
            };
            let base_bytes = batch.bytes();
            batch.index = batch.build_spatial_index();
            assert!(!batch.index.is_empty());
            assert_eq!(
                batch.bytes() - base_bytes,
                batch.index.len() as u64 * std::mem::size_of::<SpatialNode>() as u64
            );
            assert_eq!(
                batch.work(),
                batch.instances.len() as u64 * 64 + batch.index.len() as u64 * 16 + 1
            );
            for antialias in [0.125, 1.0, 8.0, 0.0, -1.0, f64::INFINITY, f64::NAN] {
                for row in -2..=10 {
                    for column in -2..=15 {
                        let point = Point::new(
                            column as f64 * 3.0 * f64::from(scale),
                            row as f64 * 3.0 * f64::from(scale),
                        );
                        let expected = batch.sample(
                            point,
                            antialias,
                            [-0.0; 4],
                            motion_core::BlendSpace::LinearRgb,
                            composite,
                        )?;
                        let actual = batch.sample_with_zero_policy::<true>(
                            point,
                            antialias,
                            [-0.0; 4],
                            motion_core::BlendSpace::LinearRgb,
                            composite,
                        )?;
                        assert_eq!(actual.map(f32::to_bits), expected.map(f32::to_bits));
                    }
                }
            }
        }
    }
    Ok(())
}

#[test]
fn cached_line_support_is_charged_without_changing_device_layout() {
    let particle = Particle {
        position: [8.0, -4.0],
        ..Particle::default()
    };
    let instance = Instance::new(particle, [-2.0, 6.0], true);
    assert_eq!(instance.line_bounds, [-2.0, -4.0, 8.0, 6.0]);
    assert_eq!(instance.line_guard, 8.0 * (128.0 * f64::EPSILON));
    let plain = Instance::new(particle, [-2.0, 6.0], false);
    assert_eq!(plain.line_bounds, [0.0; 4]);
    assert_eq!(plain.line_guard, 0.0);
    let batch = ParticleBatch {
        instances: vec![instance, plain].into(),
        kind: 4,
        radius: 2.0,
        width: 1.0,
        align: false,
        sprite: None,
        bounds: Rect::default(),
        index: Arc::from([]),
    };
    assert_eq!(
        batch.bytes(),
        2 * std::mem::size_of::<Instance>() as u64
            + std::mem::size_of::<Arc<[SpatialNode]>>() as u64
    );
    assert_eq!(batch.work(), 2 * 64 + 1);
    #[cfg(feature = "gpu")]
    {
        let upload = batch.upload().unwrap().into_words();
        assert_eq!(upload.len(), 4 + 2 * 12);
        assert_eq!(batch.upload_bytes(), upload.len() as u64 * 4);
        assert_eq!(&upload[4..6], &particle.position);
        assert_eq!(&upload[6..8], &[-2.0, 6.0]);
        assert_eq!(upload[15], 3.0);
        assert_eq!(upload[27], 0.0);
    }
}

fn prepared(node: Node) -> Result<motion_render::PreparedFrame, ValidationError> {
    node.compile(
        Size::new(32.0, 24.0),
        PixelSize::new(64, 48),
        ValidationLimits::default(),
    )?
    .prepare(ValidationLimits::default(), RasterLimits::default())
}

#[test]
fn instance_styles_are_not_paths_and_respect_budget_and_custom_callbacks() -> motion_core::Result<()>
{
    let cache = motion_core::SceneCache::new(0);
    let context = SceneContext::new(
        500.milliseconds().after_origin(),
        motion_core::fps!(30),
        Size::new(32.0, 24.0),
        &cache,
    )
    .with_clip_duration(2.seconds());
    let system =
        ParticleTrajectory::new(Ballistic::burst(Point::new(16.0, 12.0), 3).speed(2.0, 2.0));
    let node = system.try_render(&context)?;
    let list = node.compile(
        context.size(),
        PixelSize::new(64, 48),
        ValidationLimits::default(),
    )?;
    assert!(
        list.commands
            .iter()
            .all(|command| matches!(command, DrawCommand::ParticleInstances { .. }))
    );
    let collapsed = prepared(node.clone().scale(0.0))?;
    assert!(
        collapsed
            .render_linear(RasterLimits::default())?
            .linear_pixels()
            .all(|pixel| pixel == [0.0; 4])
    );

    struct Ignored(Arc<dyn motion_core::instances::InstanceBatch>);

    impl Drawable for Ignored {
        fn bounds(&self, _: &motion_core::DrawContext) -> Result<Rect, ValidationError> {
            Ok(self.0.bounds())
        }

        fn draw(
            &self,
            _: &motion_core::DrawContext,
            canvas: &mut motion_core::Canvas<'_>,
        ) -> Result<(), ValidationError> {
            let _ = canvas.instances(
                self.0.clone(),
                Transform::from_coefficients([f64::NAN, 0.0, 0.0, 1.0, 0.0, 0.0]),
            );
            Ok(())
        }
    }
    let DrawCommand::ParticleInstances { batch, .. } = &list.commands[0] else {
        unreachable!()
    };
    assert!(
        motion_core::DrawContext::default()
            .record(&Ignored(batch.clone()), ValidationLimits::default())
            .is_err()
    );
    let frame = prepared(node)?;
    assert_eq!(frame.strip_count(), 0);
    let pixels = frame.render_linear(RasterLimits::default())?;
    assert!(pixels.linear_pixels().any(|pixel| pixel[3] > 0.0));
    assert!(
        frame
            .render_linear(RasterLimits {
                max_work: 1,
                ..Default::default()
            })
            .is_err()
    );
    for style in [
        ParticleStyle::GlowDots { radius: 2.0 },
        ParticleStyle::Trails {
            length: 200.milliseconds(),
            width: 1.0,
        },
        ParticleStyle::Plexus {
            radius: 1.0,
            max_distance: 20.0,
        },
    ] {
        let frame = prepared(system.clone().style(style).try_render(&context)?)?;
        assert!(
            frame
                .render_linear(RasterLimits::default())?
                .linear_pixels()
                .any(|pixel| pixel[3] > 0.0)
        );
    }
    let assets = motion_core::Assets::new(".")?;
    let empty =
        ParticleTrajectory::new(Ballistic::burst(Point::ORIGIN, 0)).style(ParticleStyle::Sprites {
            image: assets.image("missing-unused-particle-sprite.png"),
            size: 3.0,
            align_to_velocity: false,
        });
    assert!(
        empty
            .try_render(&context)?
            .compile(
                context.size(),
                PixelSize::new(64, 48),
                ValidationLimits::default()
            )?
            .commands
            .is_empty()
    );
    let custom = system.style(ParticleStyle::Custom(Arc::new(|particles, _| {
        Ok(Node::from(
            Rect::new(0.0, 0.0, particles.len() as f64, 1.0).fill(motion_core::Color::WHITE),
        ))
    })));
    assert!(
        prepared(custom.try_render(&context)?)?
            .render_linear(RasterLimits::default())?
            .linear_pixel(0, 0)
            .unwrap()[3]
            > 0.0
    );
    Ok(())
}

#[cfg(feature = "gpu")]
mod gpu {
    use super::*;
    use motion_core::Shape;

    #[test]
    #[ignore = "requires actual GPU adapter; extended protocol coverage and blend parity only"]
    fn extended_shapes_sprites_trails_and_blends_match_cpu_gpu() -> motion_core::Result {
        let cache = motion_core::SceneCache::new(0);
        let limits = RasterLimits::default();
        let mut gpu = motion_render::gpu::Gpu::new()?;
        let context = SceneContext::new(
            250.milliseconds().after_origin(),
            motion_core::fps!(30),
            Size::new(32.0, 24.0),
            &cache,
        )
        .with_clip_duration(1.seconds());
        let system = ParticleSimulation::new(7)
            .duration(1.seconds())
            .emitter(
                BurstEmitter::new(Time::ZERO, 257, Point::new(16.0, 12.0))
                    .speed(2.0)
                    .depth_range(-1.0, 1.0)
                    .color([1.5, 0.2, 0.1, 0.2]),
            )
            .build()?;
        let atlas = SpriteAtlas::new(
            &[
                ProceduralSprite::SoftCircle.generate(4, 7)?,
                ProceduralSprite::Ring.generate(4, 7)?,
            ],
            65536,
        )?;
        let styles = vec![
            ParticleStyle::Shape {
                radius: 1.0,
                shape: ParticleShape::Polygon { sides: 6 },
            },
            ParticleStyle::Shape {
                radius: 1.0,
                shape: ParticleShape::Ring { thickness: 0.3 },
            },
            ParticleStyle::Shape {
                radius: 1.0,
                shape: ParticleShape::Capsule { aspect: 2.0 },
            },
            ParticleStyle::Shape {
                radius: 1.0,
                shape: ParticleShape::Soft { hardness: 0.5 },
            },
            ParticleStyle::Shape {
                radius: 1.0,
                shape: ParticleShape::Quad { aspect: 1.5 },
            }
            .flip(),
            ParticleStyle::Bokeh {
                radius: 1.0,
                aperture: 0.8,
                focus: 0.0,
                shape: BokehShape::Polygon(6),
                ring: 0.3,
                hardness: 0.5,
                highlight: 2.0,
            }
            .parallax([0.5, 0.0]),
            ParticleStyle::Sprites {
                image: motion_core::Assets::default().preload(
                    motion_core::Source::Bytes {
                        key: "particle-atlas".into(),
                        data: Arc::from([]),
                    },
                    atlas.image,
                )?,
                size: 3.0,
                align_to_velocity: true,
            }
            .sheet(atlas.sheet)
            .stretch_by_speed(0.5, 3.0),
            ParticleStyle::Trails {
                length: 100.milliseconds(),
                width: 1.0,
            }
            .trails(TrailOptions {
                segments: 16,
                taper: Some(ParticleCurve::linear(0.0, 1.0)),
                fade: Some(ParticleCurve::linear(0.0, 1.0)),
                smooth: true,
                ribbon: true,
            }),
            ParticleStyle::Sprites {
                image: motion_core::Assets::default().preload(
                    motion_core::Source::Bytes {
                        key: "particle-shared-smoke".into(),
                        data: Arc::from([]),
                    },
                    ProceduralSprite::Smoke {
                        scale: 3.0,
                        evolution: 0.5,
                        fractal: motion_core::noise::Fractal::default(),
                    }
                    .generate_at(16, 42, context.time(), limits)?,
                )?,
                size: 3.0,
                align_to_velocity: false,
            },
        ];
        for (style_index, style) in styles.into_iter().enumerate() {
            for blend in [
                ParticleBlend::SourceOver,
                ParticleBlend::Plus,
                ParticleBlend::Screen,
                ParticleBlend::Multiply,
            ] {
                let input = if matches!(blend, ParticleBlend::Screen | ParticleBlend::Multiply) {
                    system
                        .clone()
                        .to_builder()
                        .modifier(ColorOverLifetime::new(Arc::from([[0.08, 0.04, 0.02, 0.2]])))
                        .build()?
                } else {
                    system.clone()
                };
                let frame = prepared(
                    input
                        .clone()
                        .to_builder()
                        .style(style.clone().blend(blend))
                        .build()?
                        .try_render(&context)?,
                )?;
                let expected = frame.render_linear(limits)?;
                let actual = gpu.render_linear(&frame, limits)?;
                assert!(actual.linear_pixels().any(|pixel| pixel[3] > 0.0));
                assert_eq!(actual, gpu.render_linear(&frame, limits)?);
                let maximum = expected
                    .linear_pixels()
                    .zip(actual.linear_pixels())
                    .flat_map(|(first, second)| {
                        first
                            .into_iter()
                            .zip(second)
                            .map(|(first, second)| (first - second).abs())
                    })
                    .fold(0.0_f32, f32::max);
                assert!(
                    maximum < 0.03,
                    "style={style_index}, blend={blend:?}, maximum={maximum}"
                );
                let expected = expected.to_format(
                    motion_render::PixelFormat::SrgbRgba8 {
                        premultiplied: false,
                    },
                    limits,
                )?;
                let actual = actual.to_format(
                    motion_render::PixelFormat::SrgbRgba8 {
                        premultiplied: false,
                    },
                    limits,
                )?;
                assert!(
                    expected
                        .bytes()
                        .iter()
                        .zip(actual.bytes())
                        .all(|(first, second)| first.abs_diff(*second) <= 3)
                );
            }
        }
        Ok(())
    }

    #[test]
    fn gpu_upload_contains_evaluated_state_not_trajectory_inputs() {
        let initial = Particle {
            position: [8.0, -4.0],
            velocity: [4.0, 3.0],
            ..Particle::default()
        };
        let current = Particle {
            position: [18.0, 12.0],
            velocity: [0.0, -2.0],
            age: 0.5,
            ..initial
        };
        let batch = ParticleBatch {
            instances: vec![Instance::new(current, initial.position, false)].into(),
            kind: 0,
            radius: 2.0,
            width: 1.0,
            align: false,
            sprite: None,
            bounds: Rect::new(0.0, 0.0, 32.0, 24.0),
            index: Arc::from([]),
        };
        let upload = batch.upload().unwrap().into_words();
        assert_eq!(upload.len(), 4 + 12);
        assert_eq!(&upload[4..6], &current.position);
        assert_eq!(&upload[6..8], &initial.position);
        assert_eq!(upload[12], batch.radius * current.size);
        assert_eq!(upload[15], 0.0);
    }

    #[test]
    #[ignore = "requires native GPU; ordered tile overlap and dynamic budget rejection"]
    fn gpu_tiles_preserve_dense_order_and_retry_after_work_rejection() -> motion_core::Result {
        use crate::particles::batch::Instance;
        let instances = (0..257)
            .map(|index| {
                let particle = Particle {
                    position: [15.0 + (index % 3) as f32 * 0.125, 12.0],
                    color: if index % 2 == 0 {
                        [0.35, 0.0, 0.0, 0.5]
                    } else {
                        [0.0, 0.25, 0.0, 0.5]
                    },
                    index,
                    ..Particle::default()
                };
                Instance::new(particle, particle.position, false)
            })
            .collect::<Vec<_>>();
        let batch = ParticleBatch {
            instances: instances.into(),
            kind: 0,
            radius: 4.0,
            width: 1.0,
            align: false,
            sprite: None,
            bounds: Rect::new(10.0, 7.0, 21.0, 17.0),
            index: Arc::from([]),
        };
        let limits = RasterLimits::default();
        let mut gpu = motion_render::gpu::Gpu::new()?;
        for space in [
            motion_core::BlendSpace::LinearRgb,
            motion_core::BlendSpace::Srgb,
        ] {
            for reflected in [false, true] {
                let node = Node::from(batch.clone());
                let node = if reflected {
                    node.scale_xy(-0.9, 1.1).translate(1.0, -0.5)
                } else {
                    node
                };
                let frame = prepared(node)?.blend_space(space);
                let expected = frame.render_linear(limits)?.to_format(
                    motion_render::PixelFormat::SrgbRgba8 {
                        premultiplied: false,
                    },
                    limits,
                )?;
                let actual = gpu.render_linear(&frame, limits)?;
                let replay = gpu.render_linear(&frame, limits)?;
                assert_eq!(actual, replay);
                let packed = actual.to_format(
                    motion_render::PixelFormat::SrgbRgba8 {
                        premultiplied: false,
                    },
                    limits,
                )?;
                assert!(
                    expected
                        .bytes()
                        .iter()
                        .zip(packed.bytes())
                        .all(|(expected, actual)| expected.abs_diff(*actual) <= 2)
                );
                let limited = RasterLimits {
                    max_work: motion_render::gpu::Gpu::plan_stats(&frame, limits)?.work,
                    ..limits
                };
                let error = gpu.render_linear(&frame, limited).unwrap_err();
                assert_eq!(
                    error.downcast_ref::<ValidationError>().unwrap().field,
                    "max_work"
                );
                assert_eq!(gpu.render_linear(&frame, limits)?, actual);
            }
        }
        Ok(())
    }

    #[test]
    #[ignore = "requires native GPU; default-budget 1080p/10000-particle admission and replay"]
    fn ten_thousand_particles_at_1080p_use_default_authoring_limits() -> motion_core::Result {
        let size = PixelSize::new(1920, 1080);
        let design = Size::new(1920.0, 1080.0);
        let cache = motion_core::SceneCache::new(0);
        let context = SceneContext::new(
            500.milliseconds().after_origin(),
            motion_core::fps!(30),
            design,
            &cache,
        )
        .with_clip_duration(2.seconds());
        let particles =
            ParticleTrajectory::new(Orbit::new(Point::new(960.0, 540.0), 400.0, 10_000))
                .style(ParticleStyle::GlowDots { radius: 2.0 });
        let node = Node::group([
            Node::from(Rect::new(0.0, 0.0, 1920.0, 1080.0).fill(motion_core::Color::BLACK)),
            particles.try_render(&context)?,
        ]);
        let limits = RasterLimits::default();
        let frame = node
            .compile(design, size, ValidationLimits::default())?
            .prepare(ValidationLimits::default(), limits)?;
        let cpu = frame.render_linear(limits)?;
        assert!(
            cpu.linear_pixels()
                .any(|pixel| pixel[..3].iter().any(|channel| *channel > 0.01))
        );
        assert_eq!(cpu, frame.render_linear(limits)?);
        let restricted = frame.render_linear(RasterLimits::sandbox()).unwrap_err();
        assert_eq!(restricted.field, "max_work");
        let mut gpu = motion_render::gpu::Gpu::new()?;
        let actual = gpu.render_linear(&frame, limits)?;
        assert_eq!(actual, gpu.render_linear(&frame, limits)?);
        let cpu_bytes = cpu.to_format(
            motion_render::PixelFormat::SrgbRgba8 {
                premultiplied: false,
            },
            limits,
        )?;
        let gpu_bytes = actual.to_format(
            motion_render::PixelFormat::SrgbRgba8 {
                premultiplied: false,
            },
            limits,
        )?;
        assert!(
            cpu_bytes
                .bytes()
                .iter()
                .zip(gpu_bytes.bytes())
                .all(|(cpu, gpu)| cpu.abs_diff(*gpu) <= 2)
        );
        let rejected = gpu
            .render_linear(&frame, RasterLimits::sandbox())
            .unwrap_err();
        assert_eq!(
            rejected.downcast_ref::<ValidationError>().unwrap().field,
            "max_work"
        );
        assert_eq!(actual, gpu.render_linear(&frame, limits)?);
        println!(
            "Default 1080p/10000 particles admitted: CPU bytes {}, GPU bytes {}",
            cpu.byte_len(),
            actual.byte_len()
        );
        Ok(())
    }

    #[test]
    #[ignore = "requires native GPU; validates dispatch and admission above the former protocol ceiling"]
    fn evaluated_instances_above_old_ceiling_render_and_replay() -> motion_core::Result<()> {
        let count = 65_537;
        let particles = ParticleTrajectory::new(Orbit::new(Point::new(2.0, 2.0), 0.0, count))
            .style(ParticleStyle::Dots { radius: 0.5 });
        let cache = motion_core::SceneCache::new(0);
        let context = SceneContext::new(
            Time::ZERO,
            motion_core::fps!(30),
            Size::new(4.0, 4.0),
            &cache,
        );
        let node = particles.try_render(&context)?;
        let limits = RasterLimits::default();
        let frame = node
            .compile(
                Size::new(4.0, 4.0),
                PixelSize::new(4, 4),
                ValidationLimits::default(),
            )?
            .prepare(ValidationLimits::default(), limits)?;
        let cpu = frame.render_linear(limits)?.to_format(
            motion_render::PixelFormat::SrgbRgba8 {
                premultiplied: false,
            },
            limits,
        )?;
        let mut gpu = motion_render::gpu::Gpu::new()?;
        let actual = gpu.render_linear(&frame, limits)?;
        assert_eq!(actual, gpu.render_linear(&frame, limits)?);
        let bytes = actual.to_format(
            motion_render::PixelFormat::SrgbRgba8 {
                premultiplied: false,
            },
            limits,
        )?;
        assert!(
            cpu.bytes()
                .iter()
                .zip(bytes.bytes())
                .all(|(left, right)| left.abs_diff(*right) <= 1)
        );
        assert!(
            bytes
                .bytes()
                .as_chunks::<4>()
                .0
                .iter()
                .any(|pixel| pixel[3] > 0)
        );
        assert!(
            ParticleTrajectory::new(Orbit::new(
                Point::ORIGIN,
                0.0,
                MAX_INSTANCE_COUNT as u64 + 1
            ))
            .sample(Time::ZERO)
            .is_err()
        );
        Ok(())
    }

    #[test]
    #[ignore = "requires actual GPU adapter; no fonts, window, encoder or formula feature"]
    fn builtin_models_and_styles_match_cpu_gpu_and_replay() -> motion_core::Result<()> {
        let directory = tempfile::tempdir()?;
        image::RgbaImage::from_fn(2, 2, |column, row| {
            image::Rgba([
                if column == 0 { 255 } else { 0 },
                if row == 0 { 255 } else { 0 },
                255,
                192,
            ])
        })
        .save(directory.path().join("sprite.png"))
        .map_err(crate::image_error)?;
        let assets = motion_core::Assets::new(directory.path())?;
        let image = assets.image("sprite.png");
        let systems = [
            ParticleTrajectory::new(
                Ballistic::burst(Point::new(16.0, 12.0), 7)
                    .speed(2.0, 5.0)
                    .gravity(motion_core::Vector2::new(0.0, 4.0))
                    .drag(0.8),
            ),
            ParticleTrajectory::new(Converge::new(
                &[Point::new(4.0, 4.0), Point::new(12.0, 4.0)],
                &[Point::new(24.0, 18.0), Point::new(10.0, 20.0)],
                2.seconds(),
            )?),
            ParticleTrajectory::new(Orbit::new(Point::new(16.0, 12.0), 5.0, 7)),
            ParticleTrajectory::new(NoiseDrift::new(Point::new(16.0, 12.0), 7).noise(3.0, 0.3)),
            ParticleTrajectory::new(
                Ballistic::burst(Point::new(16.0, 12.0), 257)
                    .speed(2.0, 5.0)
                    .gravity(motion_core::Vector2::new(0.0, 4.0))
                    .drag(0.8),
            ),
            ParticleTrajectory::new(Converge::new(
                &vec![Point::new(4.0, 4.0); 257],
                &vec![Point::new(24.0, 18.0); 257],
                2.seconds(),
            )?),
            ParticleTrajectory::new(Orbit::new(Point::new(16.0, 12.0), 5.0, 257)),
            ParticleTrajectory::new(NoiseDrift::new(Point::new(16.0, 12.0), 257).noise(3.0, 0.3)),
        ];
        let styles = [
            ParticleStyle::Dots { radius: 1.25 },
            ParticleStyle::GlowDots { radius: 2.25 },
            ParticleStyle::Sprites {
                image,
                size: 3.0,
                align_to_velocity: true,
            },
            ParticleStyle::Trails {
                length: 200.milliseconds(),
                width: 0.75,
            },
            ParticleStyle::Plexus {
                radius: 1.0,
                max_distance: 5.0,
            },
            ParticleStyle::Custom(Arc::new(|particles, _| {
                Ok(Node::group(
                    particles
                        .iter()
                        .map(|particle| {
                            Ok(Node::from(
                                motion_core::Circle::new(
                                    Point::new(
                                        f64::from(particle.position[0]),
                                        f64::from(particle.position[1]),
                                    ),
                                    1.0,
                                )?
                                .fill(motion_core::Color::WHITE),
                            ))
                        })
                        .collect::<Result<Vec<_>, motion_core::ValidationError>>()?,
                ))
            })),
        ];
        let cache = motion_core::SceneCache::new(0);
        let limits = RasterLimits::default();
        let mut gpu = motion_render::gpu::Gpu::new()?;
        let mut cases = 0;
        let mut max_linear = 0.0f32;
        let mut max_byte = 0u8;
        for (system_index, system) in systems.into_iter().enumerate() {
            for style in &styles {
                if system_index >= 4
                    && !matches!(
                        style,
                        ParticleStyle::Dots { .. }
                            | ParticleStyle::GlowDots { .. }
                            | ParticleStyle::Sprites { .. }
                    )
                {
                    continue;
                }
                for milliseconds in [250, 0, 500, 250] {
                    let context = SceneContext::new(
                        milliseconds.milliseconds().after_origin(),
                        motion_core::fps!(30),
                        Size::new(32.0, 24.0),
                        &cache,
                    )
                    .with_clip_duration(2.seconds());
                    let node = system
                        .clone()
                        .style(style.clone())
                        .try_render(&context)?
                        .opacity(0.7)
                        .clip_with(
                            Rect::new(1.0, 1.0, 31.0, 23.0),
                            motion_core::ClipMode::Geometric,
                        )
                        .translate(0.125, 0.25)
                        .filter(motion_core::Filter::Blur { sigma: 0.1 });
                    for space in [
                        motion_core::BlendSpace::LinearRgb,
                        motion_core::BlendSpace::Srgb,
                    ] {
                        let frame = prepared(node.clone())?.blend_space(space);
                        let expected = frame.render_linear(limits)?;
                        let actual = gpu.render_linear(&frame, limits)?;
                        assert_eq!(actual, gpu.render_linear(&frame, limits)?);
                        for (reference, result) in
                            expected.linear_pixels().zip(actual.linear_pixels())
                        {
                            for channel in 0..4 {
                                max_linear =
                                    max_linear.max((reference[channel] - result[channel]).abs());
                            }
                        }
                        let expected = expected.to_format(
                            motion_render::PixelFormat::SrgbRgba8 {
                                premultiplied: false,
                            },
                            limits,
                        )?;
                        let actual = actual.to_format(
                            motion_render::PixelFormat::SrgbRgba8 {
                                premultiplied: false,
                            },
                            limits,
                        )?;
                        for (reference, result) in expected.bytes().iter().zip(actual.bytes()) {
                            max_byte = max_byte.max(reference.abs_diff(*result));
                        }
                        assert!(
                            max_byte <= 2,
                            "case={cases},max_byte={max_byte},max_linear={max_linear}"
                        );
                        cases += 1;
                    }
                }
            }
        }
        let simulation = ParticleSimulation::new(42)
            .duration(1.seconds())
            .emitter(BurstEmitter::new(Time::ZERO, 3, Point::new(16.0, 12.0)).speed(3.0))
            .modifier(Gravity::new(motion_core::Vector2::new(0.0, 4.0)))
            .build()?;
        for style in [styles[0].clone(), styles[3].clone()] {
            for space in [
                motion_core::BlendSpace::LinearRgb,
                motion_core::BlendSpace::Srgb,
            ] {
                let context = SceneContext::new(
                    500.milliseconds().after_origin(),
                    motion_core::fps!(30),
                    Size::new(32.0, 24.0),
                    &cache,
                )
                .with_clip_duration(1.seconds());
                let frame = prepared(
                    simulation
                        .clone()
                        .to_builder()
                        .style(style.clone())
                        .build()?
                        .try_render(&context)?,
                )?
                .blend_space(space);
                let expected = frame.render_linear(limits)?;
                let actual = gpu.render_linear(&frame, limits)?;
                assert_eq!(actual, gpu.render_linear(&frame, limits)?);
                for (reference, result) in expected.linear_pixels().zip(actual.linear_pixels()) {
                    for channel in 0..4 {
                        max_linear = max_linear.max((reference[channel] - result[channel]).abs());
                    }
                }
                let expected = expected.to_format(
                    motion_render::PixelFormat::SrgbRgba8 {
                        premultiplied: false,
                    },
                    limits,
                )?;
                let actual = actual.to_format(
                    motion_render::PixelFormat::SrgbRgba8 {
                        premultiplied: false,
                    },
                    limits,
                )?;
                for (reference, result) in expected.bytes().iter().zip(actual.bytes()) {
                    max_byte = max_byte.max(reference.abs_diff(*result));
                }
                assert!(
                    expected
                        .bytes()
                        .iter()
                        .zip(actual.bytes())
                        .all(|(reference, result)| reference.abs_diff(*result) <= 2)
                );
                cases += 1;
            }
        }
        assert_eq!(cases, 292);
        println!(
            "particles cases={cases} max_linear={max_linear} max_byte={max_byte} replay=exact"
        );
        Ok(())
    }
}
