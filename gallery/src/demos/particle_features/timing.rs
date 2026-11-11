//! Explicit prewarm, seamless phases, time mapping and worker-count capability studies.

use super::{SOURCE_SECONDS, Specimen, Study, VIEWPORT, base, text};
use crate::DemoParameters;
use motion::particles::{
    BounceBox, BurstEmitter, Gravity, ParticleSimulationBuilder, ParticleStyle, PointEmitter,
};
use motion::prelude::*;

fn stream(parameters: DemoParameters) -> ParticleSimulationBuilder {
    base(parameters).emitter(
        PointEmitter::new(Point::new(84.0, 152.0))
            .rate(f64::from(parameters.count))
            .speed(8.0)
            .velocity(Vector2::new(82.0, 0.0))
            .lifetime(3.5.seconds())
            .color([0.3, 0.85, 0.65, 1.0]),
    )
}

pub(super) fn prewarm(parameters: DemoParameters) -> motion::Result<Study> {
    Ok(Study::new(stream(parameters).prewarm(1.5.seconds())))
}

pub(super) fn looped(parameters: DemoParameters) -> motion::Result<Study> {
    Ok(Study::new(stream(parameters).loop_period(1.8.seconds())))
}

pub(super) fn slow(parameters: DemoParameters) -> motion::Result<Study> {
    Ok(Study::new(stream(parameters).time_remap(
        0.35,
        Duration::ZERO,
        false,
    )))
}

pub(super) fn reverse(parameters: DemoParameters) -> motion::Result<Study> {
    Ok(Study::new(stream(parameters).time_remap(
        -0.75,
        3.seconds(),
        true,
    )))
}

pub(super) fn workers(parameters: DemoParameters) -> motion::Result<Study> {
    let simulation = base(parameters)
        .emitter(
            BurstEmitter::new(
                Time::ZERO,
                parameters.count.max(64) as usize * 2,
                VIEWPORT.center(),
            )
            .speed(24.0)
            .lifetime(SOURCE_SECONDS.seconds())
            .color([0.4, 0.7, 1.0, 1.0]),
        )
        .modifier(Gravity::new(Vector2::new(0.0, 8.0)))
        .modifier(BounceBox::new(Rect::new(180.0, 56.0, 332.0, 248.0)))
        .style(ParticleStyle::Dots { radius: 1.8 });
    Ok(Study {
        source: Specimen::Layers(vec![
            (
                simulation.clone().workers(1).build(),
                Point::new(-116.0, 0.0),
            ),
            (simulation.workers(4).build(), Point::new(116.0, 0.0)),
        ]),
        guides: Node::group([
            text("1 worker", 14.0, 0xb4c8bd, 108.0, 268.0),
            text("4 workers", 14.0, 0xb4c8bd, 340.0, 268.0),
        ]),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn workers_preserve_exact_states_across_seek_order() -> motion::Result {
        let study = workers(DemoParameters::default())?;
        let Specimen::Layers(layers) = study.source else {
            panic!("expected worker comparison");
        };
        for moment in [2.8, 0.25, 1.1, 2.8] {
            assert_eq!(
                layers[0]
                    .0
                    .as_ref()
                    .unwrap()
                    .sample(moment.seconds().after_origin())?,
                layers[1]
                    .0
                    .as_ref()
                    .unwrap()
                    .sample(moment.seconds().after_origin())?
            );
        }
        Ok(())
    }

    #[test]
    fn prewarm_and_reverse_use_explicit_source_time() -> motion::Result {
        let source = stream(DemoParameters::default());
        let warm = source.clone().prewarm(1.5.seconds()).build()?;
        assert!(!warm.sample(Time::ZERO)?.is_empty());
        let reversed = source
            .clone()
            .time_remap(-0.75, 3.seconds(), true)
            .build()?;
        let slowed = source
            .clone()
            .time_remap(0.35, Duration::ZERO, false)
            .build()?;
        let source = source.build()?;
        let reversed_state = reversed.sample(1.seconds().after_origin())?;
        let forward_state = source.sample(2.25.seconds().after_origin())?;
        assert!(
            reversed_state == forward_state,
            "reverse mapping changed the fixed-boundary state"
        );
        let source_time = Time::from_ticks((2.seconds().ticks() as f64 * 0.35) as i64);
        let slowed_state = slowed.sample(2.seconds().after_origin())?;
        let forward_state = source.sample(source_time)?;
        assert!(
            slowed_state == forward_state,
            "slow mapping changed the fixed-boundary state"
        );
        Ok(())
    }
}
