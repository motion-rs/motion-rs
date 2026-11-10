use super::*;
use motion_core::{Color, Constant, Easing, Path2D, Point, Rect, TimeLiteral};

#[test]
fn shared_curl_flow_and_smoke_are_seeded_and_bounded() -> motion_core::Result {
    let original = Particle {
        position: [0.37, -1.23],
        ..Default::default()
    };
    let curl = CurlNoise {
        frequency: 0.5,
        strength: 2.0,
        seed: 42,
        phase_speed: 1.0,
    };
    let mut particle = original;
    curl.apply(
        &mut particle,
        0.125,
        1.seconds(),
        &mut Random::from_key(1_u64),
    )?;
    let expected = motion_core::noise::curl2(
        original.position.map(|value| f64::from(value) * 0.5 + 1.0),
        42,
    );
    for (velocity, expected) in particle.velocity.iter().zip(expected) {
        assert!((f64::from(*velocity) - expected * 0.25).abs() < 1e-7);
    }
    let mut replay = original;
    curl.apply(
        &mut replay,
        0.125,
        1.seconds(),
        &mut Random::from_key(9_u64),
    )?;
    assert_eq!(particle, replay);
    let mut flow_particle = original;
    FlowField::curl(42, 0.5, 2.0).apply(
        &mut flow_particle,
        0.125,
        1.seconds(),
        &mut Random::from_key(1_u64),
    )?;
    for axis in 0..2 {
        assert!((particle.velocity[axis] - flow_particle.velocity[axis]).abs() < 1e-7);
    }
    assert_eq!(curl.work(), 96);
    assert_eq!(FlowField::curl(42, 0.5, 2.0).work(), 96);
    assert!(
        CurlNoise {
            frequency: f64::INFINITY,
            ..curl
        }
        .apply(&mut replay, 0.125, Time::ZERO, &mut Random::from_key(1_u64))
        .is_err()
    );

    let smoke = ProceduralSprite::Smoke {
        scale: 3.0,
        evolution: 0.5,
        fractal: motion_core::noise::Fractal::default(),
    };
    let asset = smoke.generate_at(32, 42, 1.seconds(), motion_core::RasterLimits::default())?;
    assert_eq!(
        asset,
        smoke.generate_at(32, 42, 1.seconds(), motion_core::RasterLimits::default())?
    );
    assert_ne!(
        asset,
        smoke.generate_at(32, 43, 1.seconds(), motion_core::RasterLimits::default())?
    );
    assert_ne!(
        asset,
        smoke.generate_at(32, 42, 2.seconds(), motion_core::RasterLimits::default())?
    );
    assert_eq!(asset.levels()[0].pixel(0, 0), Some([0.0; 4]));
    assert!(asset.levels()[0].pixel(16, 16).unwrap()[3] > 0.3);
    for limits in [
        motion_core::RasterLimits {
            max_bytes: 1,
            ..Default::default()
        },
        motion_core::RasterLimits {
            max_work: 1,
            ..Default::default()
        },
    ] {
        assert!(smoke.generate_at(32, 42, Time::ZERO, limits).is_err());
    }
    assert_eq!(
        ProceduralSprite::Ring.generate(32, 42)?,
        ProceduralSprite::Ring.generate_at(
            32,
            42,
            7.seconds(),
            motion_core::RasterLimits::default()
        )?
    );
    Ok(())
}

#[test]
fn shared_noise_simulation_budget_parallel_seek_and_smoke_pixels() -> motion_core::Result {
    let system = ParticleSimulation::new(7)
        .duration(200.milliseconds())
        .emitter(BurstEmitter::new(Time::ZERO, 32, Point::new(4.0, 4.0)).speed(2.0))
        .modifier(CurlNoise {
            frequency: 0.13,
            strength: 2.0,
            seed: 42,
            phase_speed: 1.0,
        })
        .modifier(FlowField::curl(19, 0.17, 0.5))
        .build()?;
    let parallel = system.clone().to_builder().workers(4).build()?;
    for time in [
        150.milliseconds(),
        10.milliseconds(),
        100.milliseconds(),
        150.milliseconds(),
    ] {
        assert_eq!(system.sample(time)?, parallel.sample(time)?);
        assert_eq!(system.sample(time)?, system.sample(time)?);
    }
    assert!(
        system
            .to_builder()
            .limits(SimulationLimits {
                max_work: 100,
                ..Default::default()
            })
            .build()?
            .sample(100.milliseconds())
            .is_err()
    );
    let size = 8;
    let seed = 0xdead_beef_1234_5678;
    let fractal = motion_core::noise::Fractal::default();
    let folded = seed as u32 ^ (seed >> 32) as u32;
    let mut pixels = Vec::new();
    for row in 0..size {
        for column in 0..size {
            let horizontal = (f64::from(column) + 0.5) / f64::from(size) * 2.0 - 1.0;
            let vertical = (f64::from(row) + 0.5) / f64::from(size) * 2.0 - 1.0;
            let field = motion_core::noise::fbm(
                |point| motion_core::noise::perlin3([point[0], point[1], 0.0], folded),
                [horizontal * 3.0, vertical * 3.0],
                fractal,
            )?;
            let alpha = (1.0 - libm::hypot(horizontal, vertical)).max(0.0).powi(2)
                * (0.65 + 0.35 * field).clamp(0.0, 1.0);
            pixels.push([1.0, 1.0, 1.0, alpha]);
        }
    }
    let smoke = ProceduralSprite::Smoke {
        scale: 3.0,
        evolution: 0.0,
        fractal,
    };
    assert_eq!(
        *smoke.generate(size, seed)?,
        crate::ImageAsset::from_rgba_float(size, size, pixels)?
    );
    Ok(())
}

fn colored_tree() -> motion_core::Result<Arc<crate::VectorTree>> {
    Ok(Arc::new(crate::VectorTree::new(
        Rect::new(0.0, 0.0, 4.0, 4.0),
        crate::VectorGroup {
            children: vec![
                crate::VectorItem::Path(crate::VectorPath::filled(
                    Path2D::from_rect(Rect::new(0.0, 0.0, 4.0, 4.0))?,
                    Color::from_hex(0xff0000),
                )),
                crate::VectorItem::Path(crate::VectorPath::filled(
                    Path2D::from_rect(Rect::new(0.0, 0.0, 4.0, 4.0))?,
                    Color::rgba(0, 0, 255, 128),
                )),
            ],
            ..Default::default()
        },
    )?))
}

#[test]
fn colored_surface_preserves_geometry_seed_and_area_density() -> motion_core::Result {
    let tree = colored_tree()?;
    let plain = tree.sample_points(64, 7)?;
    let colored = tree.sample_colored_points(64, 7)?;
    assert_eq!(
        plain,
        colored.iter().map(|sample| sample.0).collect::<Vec<_>>()
    );
    for (_, color) in &colored {
        assert_eq!(color[3], 1.0);
        assert!(color[0] > 0.45 && color[2] > 0.45);
    }
    let emit = |inherit| -> Result<Particle, ValidationError> {
        let mut output = Vec::new();
        SurfaceEmitter::new(tree.clone())
            .speed(0.0)
            .inherit_color(inherit)
            .emit(0, Time::ZERO, &mut Random::from_key(3_u64), &mut output)?;
        Ok(output[0])
    };
    assert_eq!(emit(false)?.position, emit(true)?.position);
    assert_eq!(emit(false)?.color, [1.0; 4]);
    assert_ne!(emit(true)?.color, [1.0; 4]);
    Ok(())
}

#[test]
fn image_density_color_and_animated_path_are_deterministic() -> motion_core::Result {
    let image = Arc::new(crate::ImageAsset::from_rgba_float(
        2,
        1,
        vec![[1.0, 0.0, 0.0, 0.0], [0.0, 0.5, 1.0, 0.5]],
    )?);
    for density in [ImageDensity::Alpha, ImageDensity::Luminance] {
        let emitter = ImageEmitter::new(
            image.clone(),
            Rect::new(0.0, 0.0, 2.0, 1.0),
            density,
            SimulationLimits::default(),
        )?
        .speed(0.0);
        let mut output = Vec::new();
        emitter.emit(240, 1.seconds(), &mut Random::from_key(3_u64), &mut output)?;
        for particle in output {
            assert!(particle.position[0] >= 1.0);
            assert_eq!(particle.color, [0.0, 0.25, 0.5, 0.5]);
        }
    }
    assert!(
        ImageEmitter::new(
            image,
            Rect::new(0.0, 0.0, 2.0, 1.0),
            ImageDensity::Alpha,
            SimulationLimits {
                max_bytes: 1,
                ..Default::default()
            }
        )
        .is_err()
    );
    let mut path = Path2D::new();
    path.move_to(Point::ORIGIN).line_to(Point::new(10.0, 0.0));
    let emitter = PathEmitter::new(Arc::new(path))
        .along_path(Constant::new(0.25))
        .speed(2.0);
    let mut output = Vec::new();
    emitter.emit(0, Time::ZERO, &mut Random::from_key(3_u64), &mut output)?;
    assert_eq!(output[0].position, [2.5, 0.0]);
    assert_eq!(output[0].velocity, [2.0, 0.0]);
    Ok(())
}

#[test]
fn counter_workers_and_seek_replay_match_exactly() -> motion_core::Result {
    let original = ParticleSimulation::new(7)
        .duration(250.milliseconds())
        .emitter(BurstEmitter::new(Time::ZERO, 257, Point::ORIGIN))
        .modifier(CurlNoise {
            frequency: 0.1,
            strength: 2.0,
            seed: 0,
            phase_speed: 1.0,
        })
        .modifier(Drag::new(0.1))
        .build()?;
    for time in [
        200.milliseconds(),
        10.milliseconds(),
        100.milliseconds(),
        200.milliseconds(),
    ] {
        let expected = original.sample(time)?;
        for workers in [2, 4] {
            assert_eq!(
                &*expected,
                &*original
                    .clone()
                    .to_builder()
                    .workers(workers)
                    .build()?
                    .sample(time)?
            );
        }
    }
    let mut first = Random::for_particle(7, 42, 1);
    let mut same = Random::for_particle(7, 42, 1);
    let mut other = Random::for_particle(7, 42, 2);
    assert_eq!(first.next_float(), same.next_float());
    assert_ne!(first.next_float(), other.next_float());
    Ok(())
}

#[test]
fn events_are_sorted_and_birth_depth_count_limits_fail_explicitly() -> motion_core::Result {
    let system = ParticleSimulation::new(1)
        .duration(150.milliseconds())
        .emitter(BurstEmitter::new(Time::ZERO, 3, Point::ORIGIN).lifetime(50.milliseconds()))
        .sub_emitter(SubEmitter {
            on: ParticleEventKind::Death,
            emitter: Arc::new(
                BurstEmitter::new(Time::ZERO, 1, Point::ORIGIN).lifetime(1.seconds()),
            ),
            inherit: Inherit::All,
            probability: 1.0,
        })
        .build()?;
    let events = system.events(50.milliseconds())?;
    assert_eq!(
        events
            .iter()
            .map(|event| event.particle.index)
            .collect::<Vec<_>>(),
        vec![0, 1, 2, 3, 4, 5]
    );
    assert_eq!(system.sample(50.milliseconds())?.len(), 3);
    assert!(
        system
            .clone()
            .to_builder()
            .limits(SimulationLimits {
                max_births: 3,
                ..Default::default()
            })
            .build()?
            .sample(Time::ZERO)
            .is_err()
    );
    assert!(
        system
            .clone()
            .to_builder()
            .limits(SimulationLimits {
                max_events: 2,
                ..Default::default()
            })
            .build()?
            .sample(Time::ZERO)
            .is_err()
    );
    let recursive = ParticleSimulation::new(1)
        .duration(100.milliseconds())
        .emitter(BurstEmitter::new(Time::ZERO, 1, Point::ORIGIN))
        .sub_emitter(SubEmitter {
            on: ParticleEventKind::Birth,
            emitter: Arc::new(BurstEmitter::new(Time::ZERO, 1, Point::ORIGIN)),
            inherit: Inherit::Position,
            probability: 1.0,
        })
        .limits(SimulationLimits {
            max_sub_depth: 2,
            ..Default::default()
        })
        .build()?;
    assert!(recursive.sample(Time::ZERO).is_err());
    Ok(())
}

#[test]
fn prewarm_loop_and_explicit_reverse_are_random_access() -> motion_core::Result {
    let base = ParticleSimulation::new(2)
        .duration(1.seconds())
        .emitter(
            PointEmitter::new(Point::ORIGIN)
                .speed(1.0)
                .lifetime(1.seconds()),
        )
        .build()?;
    assert!(
        base.clone()
            .to_builder()
            .prewarm(250.milliseconds())
            .build()?
            .sample(Time::ZERO)?
            .len()
            > base.sample(Time::ZERO)?.len()
    );
    let looping = base
        .clone()
        .to_builder()
        .loop_period(500.milliseconds())
        .build()?;
    assert_eq!(
        &*looping.sample(100.milliseconds())?,
        &*looping.sample(600.milliseconds())?
    );
    assert!(
        base.clone()
            .to_builder()
            .time_remap(-1.0, 1.seconds(), false)
            .build()?
            .sample(Time::ZERO)
            .is_err()
    );
    let reverse = base
        .clone()
        .to_builder()
        .time_remap(-1.0, 900.milliseconds(), true)
        .build()?;
    assert_eq!(
        &*reverse.sample(100.milliseconds())?,
        &*base.sample(800.milliseconds())?
    );
    assert_eq!(
        &*reverse.sample(600.milliseconds())?,
        &*base.sample(300.milliseconds())?
    );
    Ok(())
}

#[test]
fn curves_temperature_hdr_and_field_have_bounded_absolute_values() -> motion_core::Result {
    let curve = ParticleCurve {
        stops: Arc::from([(0.0, 2.0), (0.2, 4.0), (1.0, 0.0)]),
        easing: Easing::Linear,
    };
    assert_eq!(curve.sample(0.2)?, 4.0);
    assert!(curve.sample(0.1)? > 1.0);
    let warm = Temperature::rgb(1200.0)?;
    let cold = Temperature::rgb(12000.0)?;
    assert!(cold[2] > warm[2]);
    let colors = ColorCurve {
        stops: Arc::from([(0.0, [0.0; 4]), (1.0, [4.0, 1.0, 0.0, 1.0])]),
        easing: Easing::Linear,
    };
    assert_eq!(colors.sample(0.5)?, [2.0, 0.5, 0.0, 0.5]);
    let mut particle = Particle {
        velocity: [5.0, 0.0],
        ..Default::default()
    };
    ColorField {
        coordinate: FieldCoordinate::Speed,
        range: [0.0, 10.0],
        colors,
    }
    .apply(
        &mut particle,
        0.01,
        Time::ZERO,
        &mut Random::from_key(1_u64),
    )?;
    assert_eq!(particle.color, [2.0, 0.5, 0.0, 0.5]);
    let system = ParticleSimulation::new(1)
        .duration(1.seconds())
        .emitter(BurstEmitter::new(Time::ZERO, 1, Point::ORIGIN))
        .modifier(IntensityOverLifetime::new(ParticleCurve::linear(4.0, 4.0)))
        .build()?;
    assert_eq!(
        system.sample(100.milliseconds())?[0].color,
        [4.0, 4.0, 4.0, 1.0]
    );
    assert_eq!(
        system.sample(200.milliseconds())?[0].color,
        [4.0, 4.0, 4.0, 1.0]
    );
    Ok(())
}

#[test]
fn sprites_sheets_and_atlas_preserve_cells_and_seeds() -> motion_core::Result {
    for generator in [
        ProceduralSprite::SoftCircle,
        ProceduralSprite::Smoke {
            scale: 3.0,
            evolution: 0.0,
            fractal: motion_core::noise::Fractal::default(),
        },
        ProceduralSprite::Star,
        ProceduralSprite::Ring,
        ProceduralSprite::FireCore,
    ] {
        let first = generator.generate(8, 7)?;
        let same = generator.generate(8, 7)?;
        for row in 0..8 {
            for column in 0..8 {
                assert_eq!(
                    first.levels()[0].pixel(column, row),
                    same.levels()[0].pixel(column, row)
                );
            }
        }
    }
    let first = ProceduralSprite::SoftCircle.generate(8, 1)?;
    let second = ProceduralSprite::Ring.generate(8, 1)?;
    let atlas = SpriteAtlas::new(&[first, second], 64 * 1024)?;
    assert_eq!(
        atlas.sheet.frame(&Particle {
            custom: [1.0, 0.0, 0.0, 0.0],
            ..Default::default()
        })?,
        1
    );
    assert_eq!(
        atlas.sheet.uv(&Particle {
            custom: [1.0, 0.0, 0.0, 0.0],
            ..Default::default()
        })?,
        [0.5, 0.0, 1.0, 1.0]
    );
    assert!(SpriteAtlas::new(&[atlas.image], 1).is_err());
    for mode in [
        SpriteSheetMode::OverLifetime,
        SpriteSheetMode::Fps(8.0),
        SpriteSheetMode::Random(7),
        SpriteSheetMode::Custom,
    ] {
        let sheet = SpriteSheet {
            columns: 2,
            rows: 2,
            frames: 4,
            mode,
        };
        let particle = Particle {
            age: 0.5,
            ..Default::default()
        };
        assert_eq!(sheet.frame(&particle)?, sheet.frame(&particle)?);
        assert!(sheet.frame(&particle)? < 4);
    }
    Ok(())
}

#[test]
fn spatial_hash_handles_twenty_thousand_and_preserves_pair_order() -> motion_core::Result {
    let sparse = (0..20_000)
        .map(|index| Particle {
            index,
            position: [(index % 200) as f32 * 2.0, (index / 200) as f32 * 2.0],
            ..Default::default()
        })
        .collect::<Vec<_>>();
    assert!(neighbors::pairs(&sparse, 0.5, 1_000_000)?.is_empty());
    assert!(neighbors::pairs(&sparse, 0.5, 1).is_err());
    let dense = &sparse[..10];
    let expected = (0..10)
        .flat_map(|first| {
            (first + 1..10)
                .filter(move |second| {
                    (dense[first].position[0] - dense[*second].position[0]).abs() <= 4.0
                })
                .map(move |second| (first, second))
        })
        .collect::<Vec<_>>();
    assert_eq!(neighbors::pairs(dense, 4.0, 1000)?, expected);
    Ok(())
}

#[test]
fn geometry_and_sdf_collisions_do_not_cross_thin_edges_or_corners() -> motion_core::Result {
    let response = CollisionResponse {
        restitution: 0.5,
        friction: 0.5,
        kill_on_hit: false,
    };
    let mut grazing = Particle {
        position: [-0.02001, 0.5],
        velocity: [0.0, 100.0],
        size: 0.02,
        ..Default::default()
    };
    assert!(
        !SegmentCollider {
            start: Point::new(0.0, 0.0),
            end: Point::new(0.0, 1.0),
            response
        }
        .apply_event(&mut grazing, 0.1, Time::ZERO, &mut Random::from_key(1_u64))?
    );
    let tree = Arc::new(crate::VectorTree::new(
        Rect::new(-1.0, -1.0, 1.0, 1.0),
        crate::VectorGroup {
            children: vec![crate::VectorItem::Path(crate::VectorPath::filled(
                Path2D::from_rect(Rect::new(0.0, 0.0, 0.01, 1.0))?,
                Color::WHITE,
            ))],
            ..Default::default()
        },
    )?);
    let grid = Arc::new(SignedDistanceGrid::new(
        tree.surface()?,
        Rect::new(-1.0, -1.0, 1.0, 1.0),
        [5, 5],
        SimulationLimits::default(),
    )?);
    let colliders: Vec<Box<dyn ParticleModifier>> = vec![
        Box::new(SegmentCollider {
            start: Point::new(0.0, 0.0),
            end: Point::new(0.0, 1.0),
            response,
        }),
        Box::new(PolygonCollider::new(
            Arc::from([
                Point::new(0.0, 0.0),
                Point::new(0.01, 0.0),
                Point::new(0.01, 1.0),
                Point::new(0.0, 1.0),
            ]),
            response,
        )?),
        Box::new(SdfCollider {
            grid: grid.clone(),
            response,
        }),
    ];
    for collider in colliders {
        for vertical in [0.0, 0.5, 1.0] {
            let mut particle = Particle {
                position: [-0.1, vertical],
                velocity: [2.0, 0.1],
                size: 0.02,
                ..Default::default()
            };
            assert!(collider.apply_event(
                &mut particle,
                0.1,
                Time::ZERO,
                &mut Random::from_key(1_u64)
            )?);
            particle.position[0] += particle.velocity[0] * 0.1;
            assert!(particle.position[0] < 0.0);
            assert!(particle.velocity[0] < 0.0);
        }
    }
    let mut particle = Particle {
        position: [-0.1, 0.5],
        velocity: [10.0, 0.0],
        size: 0.02,
        ..Default::default()
    };
    assert!(
        SdfCollider { grid, response }
            .apply(&mut particle, 0.1, Time::ZERO, &mut Random::from_key(1_u64))
            .is_err()
    );
    let cache = motion_core::SceneCache::new(1024 * 1024);
    let context = motion_core::SceneContext::new(
        Time::ZERO,
        motion_core::FrameRate::from(30),
        motion_core::Size::new(2.0, 2.0),
        &cache,
    );
    let cached = || {
        SignedDistanceGrid::cached(
            &context,
            "wall",
            tree.surface()?,
            Rect::new(-1.0, -1.0, 1.0, 1.0),
            [5, 5],
            SimulationLimits::default(),
        )
    };
    assert!(Arc::ptr_eq(&cached()?, &cached()?));
    Ok(())
}

#[test]
fn flow_radial_forces_and_age_events_follow_authored_controls() -> motion_core::Result {
    let mut particle = Particle {
        position: [1.0, 0.0],
        ..Default::default()
    };
    RadialForce {
        center: Point::ORIGIN,
        strength: 2.0,
        radius: 10.0,
        falloff: ForceFalloff::Constant,
    }
    .apply(&mut particle, 0.5, Time::ZERO, &mut Random::from_key(0_u64))?;
    assert_eq!(particle.velocity, [-1.0, 0.0]);

    let flow = FlowField {
        source: FlowSource::Curl {
            seed: 42,
            frequency: 0.3,
        },
        strength: 4.0,
        phase_speed: 0.5,
    };
    let mut first = Particle {
        position: [2.0, 3.0],
        ..Default::default()
    };
    let mut replay = first;
    flow.apply(&mut first, 0.25, 1.seconds(), &mut Random::from_key(1_u64))?;
    flow.apply(&mut replay, 0.25, 1.seconds(), &mut Random::from_key(2_u64))?;
    assert_eq!(first, replay);
    assert_ne!(first.velocity, [0.0; 2]);

    let image = Arc::new(crate::ImageAsset::from_rgba_float(
        1,
        1,
        vec![[1.0, 0.0, 0.0, 0.5]],
    )?);
    let mut mapped = Particle {
        position: [0.5, 0.5],
        ..Default::default()
    };
    FlowField {
        source: FlowSource::Image {
            image,
            bounds: Rect::new(0.0, 0.0, 1.0, 1.0),
        },
        strength: 2.0,
        phase_speed: 0.0,
    }
    .apply(&mut mapped, 0.5, Time::ZERO, &mut Random::from_key(0_u64))?;
    assert_eq!(mapped.velocity, [1.0, -1.0]);

    let system = ParticleSimulation::new(1)
        .duration(1.seconds())
        .emitter(BurstEmitter::new(Time::ZERO, 1, Point::ORIGIN).lifetime(1.seconds()))
        .sub_emitter(SubEmitter {
            on: ParticleEventKind::AgeReached(0.5),
            emitter: Arc::new(
                BurstEmitter::new(Time::ZERO, 1, Point::ORIGIN).lifetime(400.milliseconds()),
            ),
            inherit: Inherit::Position,
            probability: 1.0,
        })
        .build()?;
    let events = system.events(600.milliseconds())?;
    let mut age_events = 0;
    for boundary in 118..=123 {
        let events = system.events(((f64::from(boundary) + 0.1) / 240.0).seconds())?;
        age_events += events
            .iter()
            .filter(|event| event.kind == ParticleEventKind::AgeReached(0.5))
            .count();
    }
    assert_eq!(age_events, 1);
    assert_eq!(system.sample(600.milliseconds())?.len(), 2);
    assert_eq!(events, system.events(600.milliseconds())?);
    Ok(())
}

#[test]
fn population_forces_contacts_and_recipe_catalog_are_bounded_and_seekable() -> motion_core::Result {
    let original = [
        Particle {
            index: 1,
            position: [0.0, 0.0],
            velocity: [1.0, 0.0],
            ..Default::default()
        },
        Particle {
            index: 2,
            position: [0.5, 0.0],
            velocity: [-1.0, 0.0],
            ..Default::default()
        },
    ];
    let groups: Vec<Box<dyn ParticleGroupModifier>> = vec![
        Box::new(Boids::default()),
        Box::new(Spring {
            rest_length: 1.0,
            stiffness: 1.0,
            damping: 0.1,
            closed: false,
        }),
        Box::new(ParticleCollisions {
            response: CollisionResponse::default(),
        }),
    ];
    for group in groups {
        let mut particles = original;
        assert!(group.apply(&mut particles, 0.01, Time::ZERO, 1_000_000)? > 0);
        for particle in particles {
            particle.validate()?;
        }
        assert!(
            group
                .apply(&mut original.to_vec(), 0.01, Time::ZERO, 0)
                .is_err()
        );
    }
    for preset in presets::Preset::ALL {
        let mut options =
            presets::PresetOptions::new(7, 2.seconds(), Rect::new(0.0, 0.0, 64.0, 48.0));
        options.density = 0.1;
        options.logo = Some(colored_tree()?);
        let system = preset.simulation(options)?.build()?;
        let first = system.sample(500.milliseconds())?;
        let _later = system.sample(1500.milliseconds())?;
        assert_eq!(&*first, &*system.sample(500.milliseconds())?, "{preset:?}");
        assert!(!first.is_empty(), "{preset:?}");
    }
    Ok(())
}

#[test]
fn mixed_size_atlas_pads_cells_and_builds_a_sheet_style() -> motion_core::Result {
    let small = ProceduralSprite::SoftCircle.generate(4, 1)?;
    let large = ProceduralSprite::Ring.generate(8, 1)?;
    assert!(SpriteAtlas::new(&[small.clone(), large.clone()], 1 << 20).is_err());
    let atlas = SpriteAtlas::new_padded(&[small.clone(), large.clone(), small.clone()], 1 << 20)?;
    assert_eq!(
        (atlas.image.width(), atlas.image.height()),
        (16, 16),
        "2x2 cells of the largest image"
    );
    assert_eq!(atlas.sheet.frames, 3);
    let level = &atlas.image.levels()[0];
    assert_eq!(level.pixel(0, 0), Some([0.0; 4]), "padding is transparent");
    let centre = level.pixel(4, 4).unwrap();
    assert!(centre[3] > 0.0, "small sprite is centred in its cell");
    let randomized = atlas.clone().random(9);
    assert_eq!(randomized.sheet.mode, SpriteSheetMode::Random(9));
    assert!(SpriteAtlas::new_padded(&[], 1 << 20).is_err());
    let style = randomized.style(8.0, false)?;
    let system = ParticleSimulation::new(5)
        .duration(1.seconds())
        .emitter(BurstEmitter::new(Time::ZERO, 12, Point::new(16.0, 16.0)).speed(4.0))
        .style(style)
        .build()?;
    let cache = motion_core::SceneCache::default();
    let context = motion_core::SceneContext::new(
        100.milliseconds(),
        motion_core::FrameRate::from(30),
        motion_core::Size::new(32.0, 32.0),
        &cache,
    )
    .with_clip_duration(1.seconds());
    system.try_render(&context)?;
    Ok(())
}

#[test]
fn emitter_paint_colors_births_without_changing_positions() -> motion_core::Result {
    let bounds = Rect::new(0.0, 0.0, 100.0, 10.0);
    let gradient = motion_core::LinearGradient::new(Point::new(0.0, 0.0), Point::new(1.0, 0.0))
        .stop(0.0, Color::from_hex(0xff0000))
        .stop(1.0, Color::from_hex(0x0000ff));
    let plain = RectEmitter::new(bounds).rate(200.0);
    let painted = RectEmitter::new(bounds)
        .rate(200.0)
        .paint(motion_core::Paint::Linear(gradient.clone()), bounds)?;
    let mut first = Vec::new();
    let mut second = Vec::new();
    plain.emit(
        1,
        10.milliseconds(),
        &mut Random::from_key(3_u64),
        &mut first,
    )?;
    painted.emit(
        1,
        10.milliseconds(),
        &mut Random::from_key(3_u64),
        &mut second,
    )?;
    assert!(!first.is_empty() && first.len() == second.len());
    for (plain, painted) in first.iter().zip(&second) {
        assert_eq!(plain.position, painted.position);
        let expected = gradient.sample(
            Point::new(
                f64::from(painted.position[0]),
                f64::from(painted.position[1]),
            ),
            bounds,
        )?;
        for (actual, expected) in painted.color.iter().zip(expected) {
            assert!((actual - expected as f32).abs() < 1e-6);
        }
    }
    assert!(second.iter().any(|particle| particle.color[0] > 0.5));
    assert!(second.iter().any(|particle| particle.color[2] > 0.5));
    let invalid = bounds.union(Rect::new(f64::NAN, 0.0, 1.0, 1.0));
    assert!(
        RectEmitter::new(bounds)
            .paint(motion_core::Paint::Solid(Color::WHITE), invalid)
            .is_err()
    );
    Ok(())
}

#[test]
fn neighbor_index_queries_match_pairs_and_large_runs_are_stable() -> motion_core::Result {
    let particles = (0..5000)
        .map(|index| Particle {
            index,
            position: [(index % 100) as f32 * 1.5, (index / 100) as f32 * 1.5],
            ..Default::default()
        })
        .collect::<Vec<_>>();
    let index = neighbors::NeighborIndex::new(&particles, 2.0)?;
    let pairs = index.pairs(100_000_000)?;
    assert!(pairs.windows(2).all(|pair| pair[0] < pair[1]));
    let mut work = 0;
    for probe in [0, 1234, 4999] {
        let within = index.within(probe, false, &mut work, 100_000_000)?;
        assert!(!within.contains(&probe));
        let expected = pairs
            .iter()
            .filter_map(
                |(first, second)| match (*first == probe, *second == probe) {
                    (true, _) => Some(*second),
                    (_, true) => Some(*first),
                    _ => None,
                },
            )
            .collect::<std::collections::BTreeSet<_>>();
        assert_eq!(within, expected.into_iter().collect::<Vec<_>>());
    }
    // The allowance applies to the whole run no matter how chunks are scheduled.
    assert!(index.pairs(10_000).is_err());
    assert_eq!(pairs, neighbors::pairs(&particles, 2.0, 100_000_000)?);
    Ok(())
}

#[test]
fn group_modifiers_are_identical_for_any_worker_count() -> motion_core::Result {
    let system = ParticleSimulation::new(11)
        .duration(300.milliseconds())
        .emitter(
            RectEmitter::new(Rect::new(0.0, 0.0, 120.0, 120.0))
                .rate(3000.0)
                .speed(8.0)
                .lifetime(2.seconds()),
        )
        .modifier(Drag::new(0.2))
        .group_modifier(Boids {
            radius: 4.0,
            ..Boids::default()
        })
        .build()?;
    let expected = system.sample(250.milliseconds())?;
    assert!(expected.len() >= 128);
    for workers in [2, 3, 8] {
        assert_eq!(
            &*expected,
            &*system
                .clone()
                .to_builder()
                .workers(workers)
                .build()?
                .sample(250.milliseconds())?,
            "{workers} workers"
        );
    }
    Ok(())
}

#[test]
fn ribbon_trails_subdivide_bends_more_finely_than_round_trails() -> motion_core::Result {
    let count = |ribbon: bool| -> motion_core::Result<usize> {
        let system = ParticleSimulation::new(2)
            .duration(1.seconds())
            .emitter(BurstEmitter::new(Time::ZERO, 8, Point::new(20.0, 20.0)).speed(30.0))
            .modifier(Vortex {
                center: Point::new(20.0, 20.0),
                strength: 400.0,
            })
            .style(
                ParticleStyle::Trails {
                    length: 400.milliseconds(),
                    width: 1.0,
                }
                .trails(TrailOptions {
                    segments: 8,
                    smooth: true,
                    ribbon,
                    ..Default::default()
                }),
            )
            .build()?;
        let cache = motion_core::SceneCache::default();
        let context = motion_core::SceneContext::new(
            600.milliseconds(),
            motion_core::FrameRate::from(30),
            motion_core::Size::new(64.0, 64.0),
            &cache,
        )
        .with_clip_duration(1.seconds());
        let node = system.try_render(&context)?;
        let list = node.compile(
            context.size(),
            motion_core::PixelSize::new(64, 64),
            motion_core::ValidationLimits::default(),
        )?;
        Ok(list
            .commands
            .iter()
            .filter_map(|command| match command {
                motion_core::DrawCommand::ParticleInstances { batch, .. } => Some(batch.len()),
                _ => None,
            })
            .sum())
    };
    assert!(count(true)? >= count(false)?);
    Ok(())
}

#[test]
fn preset_catalogue_covers_the_roadmap_recipes() {
    let names = presets::Preset::ALL.map(presets::Preset::name);
    for required in [
        "Sparks",
        "Embers",
        "Dust",
        "Motes",
        "Snow",
        "Rain",
        "Smoke",
        "Fire",
        "Confetti",
        "Fireworks",
        "Fireflies",
        "Logo Dissolve",
        "Logo Assemble",
    ] {
        assert!(names.contains(&required), "{required}");
    }
}
