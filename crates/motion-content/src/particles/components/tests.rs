use super::*;
use motion_core::{Shape, TimeLiteral};

#[test]
fn emitters_modifiers_paths_and_surfaces_share_fixed_step_protocol() -> motion_core::Result<()> {
    let path = Arc::new(motion_core::Line::new(Point::ORIGIN, Point::new(10.0, 0.0))?.path()?);
    let emitters: Vec<Box<dyn ParticleEmitter>> = vec![
        Box::new(PointEmitter::new(Point::ORIGIN)),
        Box::new(DiskEmitter::new(Point::ORIGIN, 3.0)),
        Box::new(RectEmitter::new(Rect::new(0.0, 0.0, 2.0, 4.0))),
        Box::new(PathEmitter::new(path)),
    ];
    for emitter in emitters {
        let mut output = Vec::new();
        emitter.emit(0, Time::ZERO, &mut Random::from_key(42u64), &mut output)?;
        assert_eq!(output.len(), 1);
        output[0].validate()?;
    }
    let simulation = ParticleSimulation::new(1)
        .duration(100.milliseconds())
        .emitter(BurstEmitter::new(10.milliseconds().after_origin(), 2, Point::ORIGIN).speed(0.0))
        .modifier(Gravity::new(Vector2::new(0.0, 10.0)))
        .modifier(Drag::new(0.5))
        .modifier(SizeOverLifetime {
            start: 1.0,
            end: 0.0,
        })
        .build()?;
    assert!(simulation.sample(Time::ZERO)?.is_empty());
    assert_eq!(
        simulation.sample(50.milliseconds().after_origin())?.len(),
        2
    );
    assert!(simulation.sample(50.milliseconds().after_origin())?[0].position[1] > 0.0);
    let mut particle = Particle {
        position: [0.5, 0.5],
        velocity: [100.0, 0.0],
        ..Default::default()
    };
    BounceBox::new(Rect::new(0.0, 0.0, 1.0, 1.0)).apply(
        &mut particle,
        0.1,
        Time::ZERO,
        &mut Random::from_key(1u64),
    )?;
    particle.position[0] += particle.velocity[0] * 0.1;
    assert!((particle.position[0] - 0.5).abs() < 0.0001);
    Ok(())
}

#[test]
fn surface_animation_rates_and_all_modifier_callbacks_replay_without_hidden_state()
-> motion_core::Result<()> {
    let tree = Arc::new(VectorTree::new(
        Rect::new(0.0, 0.0, 4.0, 4.0),
        crate::VectorGroup {
            children: vec![crate::VectorItem::Path(crate::VectorPath::filled(
                Path2D::from_rect(Rect::new(0.0, 0.0, 4.0, 4.0))?,
                motion_core::Color::WHITE,
            ))],
            ..Default::default()
        },
    )?);
    let simulation = ParticleSimulation::new(7)
        .duration(200.milliseconds())
        .emitter(
            SurfaceEmitter::new(tree)
                .rate_animation(motion_core::Constant::new(100.0))
                .speed(0.0),
        )
        .modifier(Wind::new(Vector2::new(1.0, 0.0)))
        .modifier(Vortex {
            center: Point::ORIGIN,
            strength: 1.0,
        })
        .modifier(Attractor {
            center: Point::new(2.0, 2.0),
            strength: 1.0,
        })
        .modifier(CurlNoise {
            frequency: 0.1,
            strength: 1.0,
            seed: 0,
            phase_speed: 1.0,
        })
        .modifier(Seek {
            targets: Arc::from([Point::new(2.0, 2.0)]),
            strength: 0.5,
        })
        .modifier(ColorOverLifetime::new(Arc::from([[1.0; 4], [0.0; 4]])))
        .build()?;
    let expected = simulation.sample(150.milliseconds().after_origin())?;
    assert!(!expected.is_empty());
    simulation.sample(10.milliseconds().after_origin())?;
    assert_eq!(
        expected,
        simulation.sample(150.milliseconds().after_origin())?
    );
    assert!(
        expected
            .iter()
            .filter(|particle| particle.age > 0.0)
            .all(|particle| particle.color[3] < 1.0)
    );
    let emitter = PointEmitter::new(Point::ORIGIN).rate(100.0);
    let mut output = Vec::new();
    for step in 0..=100 {
        emitter.emit(
            step,
            (step as i64 * 10).milliseconds().after_origin(),
            &mut Random::from_key(step),
            &mut output,
        )?;
    }
    assert_eq!(output.len(), 101);
    Ok(())
}
