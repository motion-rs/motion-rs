//! Seeded builtin upload selection and native random-access trajectory parity.

use super::*;
use motion_core::instances::InstanceBatch;
use motion_core::{
    BlendSpace, ClipMode, DrawCommand, Filter, NodeModifiers, PixelSize, Point, RasterLimits,
    SceneCache, Size, TimeLiteral, ValidationLimits, Vector2,
};
use motion_render::RenderPrepare;

fn systems(count: u64) -> Result<Vec<ParticleTrajectory>, ValidationError> {
    Ok(vec![
        ParticleTrajectory::new(
            Ballistic::burst(Point::new(16.0, 12.0), count)
                .seed(42)
                .speed(2.0, 5.0)
                .gravity(Vector2::new(0.0, 4.0))
                .drag(0.0),
        ),
        ParticleTrajectory::new(
            Ballistic::burst(Point::new(16.0, 12.0), count)
                .seed(42)
                .speed(2.0, 5.0)
                .gravity(Vector2::new(0.0, 4.0))
                .drag(1e-9),
        ),
        ParticleTrajectory::new(
            Ballistic::burst(Point::new(16.0, 12.0), count)
                .seed(42)
                .speed(2.0, 5.0)
                .gravity(Vector2::new(0.0, 4.0))
                .drag(0.8),
        ),
        ParticleTrajectory::new(
            Converge::new(
                &vec![Point::new(4.0, 4.0); count as usize],
                &vec![Point::new(24.0, 18.0); count as usize],
                2.seconds(),
            )?
            .seed(42),
        ),
        ParticleTrajectory::new(Orbit::new(Point::new(16.0, 12.0), 5.0, count).seed(42)),
        ParticleTrajectory::new(
            NoiseDrift::new(Point::new(16.0, 12.0), count)
                .seed(42)
                .noise(3.0, 0.3),
        ),
    ])
}

#[test]
fn analytic_builtins_attach_seeded_initial_states_without_changing_cpu_samples()
-> motion_core::Result {
    let cache = SceneCache::new(0);
    for system in systems(7)? {
        let context = SceneContext::new(
            250.milliseconds().after_origin(),
            motion_core::fps!(30),
            Size::new(32.0, 24.0),
            &cache,
        );
        let list = system
            .clone()
            .style(ParticleStyle::Dots { radius: 1.25 })
            .try_render(&context)?
            .compile(
                context.size(),
                PixelSize::new(32, 24),
                ValidationLimits::default(),
            )?;
        let batch = list
            .commands
            .iter()
            .find_map(|command| {
                if let DrawCommand::ParticleInstances { batch, .. } = command {
                    Some(batch)
                } else {
                    None
                }
            })
            .unwrap();
        let upload = batch.analytic_upload().unwrap();
        assert_eq!(upload.len(), 7);
        assert_eq!(upload.words()[1], 0.25);
        assert_eq!(upload.words()[6].to_bits(), 42);
        let batch = batch.as_any().downcast_ref::<ParticleBatch>().unwrap();
        assert_eq!(
            batch
                .instances
                .slice()
                .iter()
                .map(|instance| instance.particle)
                .collect::<Vec<_>>(),
            system.sample(context.time())?.to_vec()
        );
        assert_eq!(batch.upload()?.into_words()[0], 7.0);
        assert_eq!(upload, batch.analytic_upload().unwrap());
        let trails = system
            .clone()
            .style(ParticleStyle::Trails {
                length: 100.milliseconds(),
                width: 1.0,
            })
            .try_render(&context)?
            .compile(
                context.size(),
                PixelSize::new(32, 24),
                ValidationLimits::default(),
            )?;
        for command in &trails.commands {
            if let DrawCommand::ParticleInstances { batch, .. } = command {
                assert!(batch.analytic_upload().is_none());
            }
        }
    }
    Ok(())
}

#[test]
fn analytic_batches_defer_cpu_sampling_and_bound_every_center() -> motion_core::Result {
    let cache = SceneCache::new(0);
    for system in systems(64)? {
        for milliseconds in [0, 250, 900, 1_900] {
            let context = SceneContext::new(
                milliseconds.milliseconds().after_origin(),
                motion_core::fps!(30),
                Size::new(32.0, 24.0),
                &cache,
            );
            let list = system.clone().try_render(&context)?.compile(
                context.size(),
                PixelSize::new(32, 24),
                ValidationLimits::default(),
            )?;
            let batch = list
                .commands
                .iter()
                .find_map(|command| {
                    if let DrawCommand::ParticleInstances { batch, .. } = command {
                        batch.as_any().downcast_ref::<ParticleBatch>()
                    } else {
                        None
                    }
                })
                .unwrap();
            assert!(batch.instances.is_deferred());
            assert!(!batch.instances.is_materialized());
            assert!(batch.upload()?.into_words()[0] == 64.0);
            assert!(!batch.instances.is_materialized());
            let bounds = batch.bounds();
            for particle in system.sample(context.time())?.iter() {
                let [x, y] = particle.position.map(f64::from);
                assert!(
                    bounds.left <= x && x <= bounds.right && bounds.top <= y && y <= bounds.bottom,
                    "{milliseconds} ms: {x},{y} outside {bounds:?}"
                );
            }
            batch.sample(
                Point::new(16.0, 12.0),
                1.0,
                [0.0; 4],
                BlendSpace::LinearRgb,
                |destination, source, _| {
                    *destination = source;
                    Ok(())
                },
            )?;
            assert!(batch.instances.is_materialized());
        }
    }
    Ok(())
}

#[test]
fn analytic_precision_fallback_preserves_cpu_and_exposes_a_reason() -> motion_core::Result {
    let system = ParticleTrajectory::new(
        Orbit::new(Point::new(16.0, 12.0), 5.0, 3).angular_velocity(20_000.0),
    );
    let cache = SceneCache::new(0);
    let context = SceneContext::new(
        1.seconds().after_origin(),
        motion_core::fps!(30),
        Size::new(32.0, 24.0),
        &cache,
    );
    let list = system.clone().try_render(&context)?.compile(
        context.size(),
        PixelSize::new(32, 24),
        ValidationLimits::default(),
    )?;
    let batch = list
        .commands
        .iter()
        .find_map(|command| {
            if let DrawCommand::ParticleInstances { batch, .. } = command {
                Some(batch)
            } else {
                None
            }
        })
        .unwrap();
    assert!(batch.analytic_upload().is_none());
    assert_eq!(
        batch.analytic_fallback().unwrap().kind,
        motion_core::ValidationErrorKind::Unsupported
    );
    let reference = batch.as_any().downcast_ref::<ParticleBatch>().unwrap();
    assert_eq!(
        reference
            .instances
            .slice()
            .iter()
            .map(|instance| instance.particle)
            .collect::<Vec<_>>(),
        system.sample(context.time())?.to_vec()
    );
    assert!(batch.upload()?.is_some());
    Ok(())
}

#[test]
#[ignore = "requires native GPU; analytic builtins, ordered tiles, clips, filters and replay"]
fn analytic_builtin_trajectories_cpu_gpu_equivalence_and_replay() -> motion_core::Result {
    let limits = RasterLimits::default();
    let cache = SceneCache::new(0);
    let mut gpu = motion_render::gpu::Gpu::new()?;
    let sprite = motion_core::Assets::default().preload(
        motion_core::Source::Bytes {
            key: "analytic-fire-core".into(),
            data: Arc::from([]),
        },
        ProceduralSprite::FireCore.generate(8, 42)?,
    )?;
    let format = motion_render::PixelFormat::SrgbRgba8 {
        premultiplied: false,
    };
    for count in [7, 257] {
        for (model, system) in systems(count)?.into_iter().enumerate() {
            for style in [
                ParticleStyle::Dots { radius: 1.25 },
                ParticleStyle::GlowDots { radius: 2.25 },
                ParticleStyle::Sprites {
                    image: sprite.clone(),
                    size: 3.0,
                    align_to_velocity: true,
                },
            ] {
                for milliseconds in [250, 0, 500, 250] {
                    let context = SceneContext::new(
                        milliseconds.milliseconds().after_origin(),
                        motion_core::fps!(30),
                        Size::new(32.0, 24.0),
                        &cache,
                    );
                    let node = system
                        .clone()
                        .style(style.clone())
                        .try_render(&context)?
                        .opacity(0.7)
                        .clip_with(Rect::new(1.0, 1.0, 31.0, 23.0), ClipMode::Geometric)
                        .translate(0.125, 0.25)
                        .filter(Filter::Blur { sigma: 0.1 });
                    for space in [BlendSpace::LinearRgb, BlendSpace::Srgb] {
                        let frame = node
                            .compile(
                                context.size(),
                                PixelSize::new(32, 24),
                                ValidationLimits::default(),
                            )?
                            .prepare(ValidationLimits::default(), limits)?
                            .blend_space(space);
                        let reference = frame.render_linear(limits)?.to_format(format, limits)?;
                        let actual = gpu
                            .render_linear(&frame, limits)?
                            .to_format(format, limits)?;
                        let replay = gpu
                            .render_linear(&frame, limits)?
                            .to_format(format, limits)?;
                        assert_eq!(actual, replay);
                        assert!(
                            actual
                                .bytes()
                                .as_chunks::<4>()
                                .0
                                .iter()
                                .any(|pixel| pixel[3] != 0)
                        );
                        for (a, b) in reference.bytes().iter().zip(actual.bytes()) {
                            assert!(
                                a.abs_diff(*b) <= 2,
                                "model={model}, count={count}, time={milliseconds}, space={space:?}: {a:?} != {b:?}"
                            );
                        }
                    }
                }
            }
        }
    }
    Ok(())
}
