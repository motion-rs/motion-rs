//! Bounded event-aware snapshots, deterministic parallel updates and time policies.

use super::*;
use std::collections::BTreeMap;

#[derive(Clone, Copy)]
struct Origin {
    depth: u8,
    color: [f32; 4],
}

#[derive(Clone)]
struct Snapshot {
    state: State,
    origins: BTreeMap<u64, Origin>,
    events: Vec<ParticleEvent>,
}

pub(super) struct PreparedReplay {
    snapshots: Vec<Snapshot>,
    horizon: Duration,
    pub(super) stats: SimulationStats,
}

/// Forward-only working snapshots for the primary and loop-crossfade lanes of one render.
///
/// These are the two working buffers admitted by [`prepare`]; they live only as long as
/// the caller keeps the cursor, never inside the prepared replay.
#[derive(Default)]
pub(super) struct ReplayCursor {
    lanes: [Option<Snapshot>; 2],
}

pub(super) fn enabled(config: &Config) -> bool {
    config.prewarm != Duration::ZERO
        || config.loop_period.is_some()
        || config.time_scale != 1.0
        || config.time_offset != Duration::ZERO
        || config.allow_reverse
        || config.workers != 1
        || !config.groups.is_empty()
        || !config.sub_emitters.is_empty()
        || config.limits.max_births != SimulationLimits::default().max_births
        || config.limits.max_events != SimulationLimits::default().max_events
        || config.limits.max_sub_depth != SimulationLimits::default().max_sub_depth
        || config
            .modifiers
            .iter()
            .any(|modifier| modifier.resets_color() || modifier.work() > 1)
}

fn push_event(
    config: &Config,
    events: &mut Vec<ParticleEvent>,
    event: ParticleEvent,
) -> Result<(), ValidationError> {
    if events.len() >= config.limits.max_events {
        return Err(limit("Particle event allowance exhausted"));
    }
    events
        .try_reserve(1)
        .map_err(|_| limit("Particle event allocation failed"))?;
    events.push(event);
    Ok(())
}

fn charge_work(config: &Config, work: &mut u64, amount: u64) -> Result<(), ValidationError> {
    *work = work
        .checked_add(amount)
        .ok_or_else(|| limit("Replay work overflow"))?;
    if *work > config.limits.max_work {
        return Err(limit("Replay work allowance exhausted"));
    }
    Ok(())
}

fn tag(kind: ParticleEventKind) -> (u8, u32) {
    match kind {
        ParticleEventKind::Birth => (0, 0),
        ParticleEventKind::Collision => (1, 0),
        ParticleEventKind::AgeReached(age) => (2, age.to_bits()),
        ParticleEventKind::Death => (3, 0),
    }
}

fn admit(
    config: &Config,
    snapshot: &mut Snapshot,
    mut particle: Particle,
    time: Time,
    depth: u8,
) -> Result<ParticleEvent, ValidationError> {
    let capacity = if config.loop_period.is_some() {
        config.limits.max_particles / 2
    } else {
        config.limits.max_particles
    };
    if snapshot.state.live.len() >= capacity || snapshot.state.next >= config.limits.max_births {
        return Err(limit("Particle/sub-emitter birth allowance exhausted"));
    }
    particle.index = snapshot.state.next;
    particle.age = 0.0;
    particle.validate()?;
    snapshot.state.next += 1;
    snapshot.origins.insert(
        particle.index,
        Origin {
            depth,
            color: particle.color,
        },
    );
    snapshot
        .state
        .live
        .try_reserve(1)
        .map_err(|_| limit("Replay particle allocation failed"))?;
    snapshot.state.live.push(Live {
        particle,
        birth: time,
    });
    Ok(ParticleEvent {
        time: time - config.prewarm,
        particle,
        kind: ParticleEventKind::Birth,
    })
}

fn root_births(
    config: &Config,
    snapshot: &mut Snapshot,
    time: Time,
    work: &mut u64,
) -> Result<Vec<ParticleEvent>, ValidationError> {
    let mut events = Vec::new();
    for (order, emitter) in config.emitters.iter().enumerate() {
        let mut output = Vec::new();
        let mut random = emitter_random(config, order, snapshot.state.step);
        emitter.emit(snapshot.state.step, time, &mut random, &mut output)?;
        charge_work(config, work, output.len() as u64 + 1)?;
        if output.len() > config.limits.max_particles
            || events.len() + output.len() > config.limits.max_events
        {
            return Err(limit("Emitter output/event allowance exhausted"));
        }
        for particle in output {
            events.push(admit(config, snapshot, particle, time, 0)?);
        }
    }
    Ok(events)
}

fn triggers(
    config: &Config,
    snapshot: &mut Snapshot,
    initial: Vec<ParticleEvent>,
    time: Time,
    work: &mut u64,
) -> Result<(), ValidationError> {
    if initial.len() > config.limits.max_events {
        return Err(limit("Particle event allowance exhausted"));
    }
    let mut pending = BTreeMap::new();
    let mut recorded = BTreeMap::new();
    for event in initial {
        let (kind, age) = tag(event.kind);
        pending.insert((event.particle.index, kind, age), event);
    }
    while let Some((key, event)) = pending.pop_first() {
        if recorded.len() >= config.limits.max_events {
            return Err(limit("Particle event allowance exhausted"));
        }
        recorded.insert(key, event);
        charge_work(config, work, config.sub_emitters.len() as u64 + 1)?;
        for (order, rule) in config.sub_emitters.iter().enumerate() {
            if rule.on != event.kind || !rule.emitter.accepts_event(&event) {
                continue;
            }
            let depth = snapshot
                .origins
                .get(&event.particle.index)
                .map_or(0, |origin| origin.depth);
            let mut random = Random::for_particle(
                config.seed ^ (order as u64).wrapping_mul(0x9e3779b97f4a7c15),
                event.particle.index,
                snapshot.state.step ^ ((key.1 as u64) << 32) ^ u64::from(key.2),
            );
            if random.next_float() >= f64::from(rule.probability) {
                continue;
            }
            let mut output = Vec::new();
            rule.emitter.emit(0, Time::ZERO, &mut random, &mut output)?;
            charge_work(config, work, output.len() as u64 + 1)?;
            if !output.is_empty() && depth >= config.limits.max_sub_depth {
                return Err(limit("Sub-emitter recursion depth exhausted"));
            }
            if output.len() > config.limits.max_particles
                || output.len() + pending.len() + recorded.len() > config.limits.max_events
            {
                return Err(limit("Sub-emitter event/output allowance exhausted"));
            }
            for mut particle in output {
                if rule.inherit != Inherit::None {
                    for axis in 0..2 {
                        particle.position[axis] += event.particle.position[axis];
                    }
                }
                if matches!(rule.inherit, Inherit::Velocity | Inherit::All) {
                    for axis in 0..2 {
                        particle.velocity[axis] += event.particle.velocity[axis];
                    }
                }
                if matches!(rule.inherit, Inherit::Color | Inherit::All) {
                    particle.color = std::array::from_fn(|channel| {
                        particle.color[channel] * event.particle.color[channel]
                    });
                }
                let birth = admit(config, snapshot, particle, time, depth + 1)?;
                pending.insert((birth.particle.index, 0, 0), birth);
            }
        }
    }
    snapshot.events = recorded.into_values().collect();
    charge_work(
        config,
        work,
        snapshot.state.live.len() as u64 * 16 + snapshot.origins.len() as u64,
    )?;
    let live_ids = snapshot
        .state
        .live
        .iter()
        .map(|live| live.particle.index)
        .collect::<std::collections::BTreeSet<_>>();
    snapshot.origins.retain(|index, _| live_ids.contains(index));
    Ok(())
}

fn update(
    config: &Config,
    live: &mut Live,
    origin: Origin,
    step: u64,
    time: Time,
) -> Result<bool, ValidationError> {
    let particle = &mut live.particle;
    let seconds = config.step.as_seconds() as f32;
    let index = particle.index;
    let age = particle.age;
    if age >= particle.lifetime {
        return Ok(false);
    }
    if config
        .modifiers
        .iter()
        .any(|modifier| modifier.resets_color())
    {
        particle.color = origin.color;
    }
    let mut collision = false;
    for (order, modifier) in config.modifiers.iter().enumerate() {
        let mut random = modifier_random(config, order, index, step);
        collision |= modifier.apply_event(particle, seconds, time, &mut random)?;
        particle.index = index;
        particle.age = age;
        particle.validate()?;
        if particle.age >= particle.lifetime {
            break;
        }
    }
    if particle.age < particle.lifetime {
        for axis in 0..2 {
            particle.position[axis] += particle.velocity[axis] * seconds;
        }
    }
    particle.validate()?;
    Ok(collision)
}

fn advance(
    config: &Config,
    snapshot: &mut Snapshot,
    work: &mut u64,
) -> Result<(), ValidationError> {
    snapshot.state.step += 1;
    let time = Time::from_ticks(snapshot.state.step as i64 * config.step.ticks() as i64);
    let mut events = Vec::new();
    for live in &mut snapshot.state.live {
        let old_age = live.particle.age;
        live.particle.age = (time - live.birth).as_seconds() as f32;
        for rule in &config.sub_emitters {
            if let ParticleEventKind::AgeReached(age) = rule.on
                && old_age < age
                && live.particle.age >= age
            {
                push_event(
                    config,
                    &mut events,
                    ParticleEvent {
                        time: time - config.prewarm,
                        particle: live.particle,
                        kind: rule.on,
                    },
                )?;
            }
        }
        if live.particle.age >= live.particle.lifetime {
            push_event(
                config,
                &mut events,
                ParticleEvent {
                    time: time - config.prewarm,
                    particle: live.particle,
                    kind: ParticleEventKind::Death,
                },
            )?;
        }
    }
    snapshot
        .state
        .live
        .retain(|live| live.particle.age < live.particle.lifetime);
    for group in &config.groups {
        let mut particles = snapshot
            .state
            .live
            .iter()
            .map(|live| live.particle)
            .collect::<Vec<_>>();
        let used = group.apply(
            &mut particles,
            config.step.as_seconds() as f32,
            time,
            config.limits.max_work.saturating_sub(*work),
        )?;
        charge_work(config, work, used)?;
        for (live, mut particle) in snapshot.state.live.iter_mut().zip(particles) {
            particle.index = live.particle.index;
            particle.age = live.particle.age;
            if group.collisions()
                && (particle.position != live.particle.position
                    || particle.velocity != live.particle.velocity
                    || particle.lifetime != live.particle.lifetime)
            {
                push_event(
                    config,
                    &mut events,
                    ParticleEvent {
                        time: time - config.prewarm,
                        particle,
                        kind: ParticleEventKind::Collision,
                    },
                )?;
            }
            live.particle = particle;
            live.particle.validate()?;
        }
    }
    let unit = config.modifiers.iter().try_fold(2u64, |total, modifier| {
        total
            .checked_add(modifier.work())
            .ok_or_else(|| limit("Modifier work overflow"))
    })?;
    charge_work(
        config,
        work,
        unit.checked_mul(snapshot.state.live.len() as u64)
            .ok_or_else(|| limit("Population work overflow"))?,
    )?;
    let count = snapshot.state.live.len();
    let workers = if count < 128 {
        1
    } else {
        config.workers.min(count)
    };
    let collisions = if workers == 1 {
        snapshot
            .state
            .live
            .iter_mut()
            .map(|live| {
                let origin = *snapshot
                    .origins
                    .get(&live.particle.index)
                    .ok_or_else(|| invalid("snapshot", "Missing birth metadata"))?;
                update(config, live, origin, snapshot.state.step, time)
            })
            .collect::<Result<Vec<_>, _>>()?
    } else {
        std::thread::scope(|scope| {
            let mut handles = Vec::new();
            let origins = &snapshot.origins;
            let step = snapshot.state.step;
            for chunk in snapshot.state.live.chunks_mut(count.div_ceil(workers)) {
                handles.push(scope.spawn(move || {
                    chunk
                        .iter_mut()
                        .map(|live| {
                            let origin = *origins
                                .get(&live.particle.index)
                                .ok_or_else(|| invalid("snapshot", "Missing birth metadata"))?;
                            update(config, live, origin, step, time)
                        })
                        .collect::<Result<Vec<_>, ValidationError>>()
                }));
            }
            let mut result = Vec::new();
            for handle in handles {
                result.extend(
                    handle
                        .join()
                        .unwrap_or_else(|panic| std::panic::resume_unwind(panic))?,
                );
            }
            Ok::<_, ValidationError>(result)
        })?
    };
    for (live, collision) in snapshot.state.live.iter().zip(collisions) {
        if collision {
            push_event(
                config,
                &mut events,
                ParticleEvent {
                    time: time - config.prewarm,
                    particle: live.particle,
                    kind: ParticleEventKind::Collision,
                },
            )?;
        }
        if live.particle.age >= live.particle.lifetime {
            push_event(
                config,
                &mut events,
                ParticleEvent {
                    time: time - config.prewarm,
                    particle: live.particle,
                    kind: ParticleEventKind::Death,
                },
            )?;
        }
    }
    snapshot
        .state
        .live
        .retain(|live| live.particle.age < live.particle.lifetime);
    events.extend(root_births(config, snapshot, time, work)?);
    if events.len() > config.limits.max_events {
        return Err(limit("Particle event allowance exhausted"));
    }
    triggers(config, snapshot, events, time, work)
}

pub(super) fn prepare(
    config: &Config,
    duration: Duration,
) -> Result<PreparedReplay, ValidationError> {
    let limits = config.limits;
    if duration <= Duration::ZERO
        || duration.as_seconds() > 3600.0
        || config.prewarm.as_seconds() > 3600.0
        || config.step <= Duration::ZERO
        || config.step > duration
        || config.interval == 0
        || limits.max_particles == 0
        || limits.max_particles > 65536
        || limits.max_sub_depth > 32
        || limits.max_events == 0
        || limits.max_events > 1_000_000
        || limits.max_births == 0
        || limits.max_births > 16_000_000
        || limits.max_work == 0
        || config.workers == 0
        || config.workers > 64
        || config.emitters.len() > 32
        || config.modifiers.len() > 32
        || config.groups.len() > 16
        || config.sub_emitters.len() > 32
        || !config.time_scale.is_finite()
        || config.time_scale.abs() > 1000.0
        || (config.time_scale < 0.0 && !config.allow_reverse)
    {
        return Err(invalid(
            "configuration",
            "Invalid enhanced simulation duration/components/time/limits",
        ));
    }
    for rule in &config.sub_emitters {
        rule.validate()?;
    }
    let interval_duration = if let Some(period) = config.loop_period {
        if period < config.step * 2 || period.as_seconds() > 3600.0 || limits.max_particles < 2 {
            return Err(invalid(
                "loop",
                "Loop period must cover two fixed steps and a two-population limit",
            ));
        }
        period
    } else {
        duration
    };
    let horizon = interval_duration + config.prewarm;
    if horizon.as_seconds() > 3600.0 {
        return Err(limit("Preparation including prewarm exceeds one hour"));
    }
    let steps = horizon.ticks() / config.step.ticks();
    if steps > limits.max_steps || steps > 1_000_000 {
        return Err(limit("Enhanced fixed-step allowance exhausted"));
    }
    let event_capacity = limits.max_events.min(
        limits
            .max_particles
            .saturating_mul(config.sub_emitters.len() + 3),
    );
    let slot = limits.max_particles as u64 * (std::mem::size_of::<Live>() as u64 + 96)
        + event_capacity as u64 * std::mem::size_of::<ParticleEvent>() as u64
        + std::mem::size_of::<Snapshot>() as u64;
    let slots = limits.max_bytes / slot;
    if slots < 3 {
        return Err(limit(
            "Enhanced snapshots require event metadata and two working buffers",
        ));
    }
    let capacity = slots - 2;
    let interval = if capacity == 1 {
        steps + 1
    } else {
        config.interval.max(steps.div_ceil(capacity - 1))
    };
    let mut snapshot = Snapshot {
        state: State {
            step: 0,
            next: 0,
            live: Vec::new(),
        },
        origins: BTreeMap::new(),
        events: Vec::new(),
    };
    let mut work = 0;
    let events = root_births(config, &mut snapshot, Time::ZERO, &mut work)?;
    triggers(config, &mut snapshot, events, Time::ZERO, &mut work)?;
    let mut snapshots = vec![snapshot.clone()];
    while snapshot.state.step < steps {
        advance(config, &mut snapshot, &mut work)?;
        if snapshot.state.step.is_multiple_of(interval) {
            snapshots.push(snapshot.clone());
        }
    }
    let stats = SimulationStats {
        snapshots: snapshots.len(),
        interval,
        admitted_bytes: (snapshots.len() as u64 + 2) * slot,
        work,
    };
    Ok(PreparedReplay {
        snapshots,
        horizon,
        stats,
    })
}

impl PreparedReplay {
    fn mapped(&self, config: &Config, time: Time) -> Result<Option<Time>, ValidationError> {
        let ticks = time.ticks() as f64 * config.time_scale + config.time_offset.ticks() as f64;
        if !ticks.is_finite() || ticks.abs() > i64::MAX as f64 * 0.5 {
            return Err(invalid("time", "Time remapping exceeds integer ticks"));
        }
        let mut source = Time::from_ticks(ticks as i64);
        if let Some(period) = config.loop_period {
            source = Time::from_ticks(source.ticks().rem_euclid(period.ticks() as i64));
        }
        if source < Time::ZERO {
            return Ok(None);
        }
        source += config.prewarm;
        if source >= self.horizon.after_origin() {
            return Ok(None);
        }
        Ok(Some(source))
    }

    /// Advances `cursor` to the fixed boundary containing `time`. The cursor restarts from the
    /// nearest retained snapshot unless it already lies between that snapshot and the target;
    /// stepping is deterministic, so both routes produce the same snapshot.
    fn advance_to<'cursor>(
        &self,
        config: &Config,
        cursor: &'cursor mut Option<Snapshot>,
        time: Time,
        work: &mut u64,
    ) -> Result<&'cursor Snapshot, ValidationError> {
        let target = time.ticks() as u64 / config.step.ticks();
        let index = self
            .snapshots
            .partition_point(|snapshot| snapshot.state.step <= target)
            .saturating_sub(1);
        let checkpoint = &self.snapshots[index];
        if cursor.as_ref().is_some_and(|snapshot| {
            snapshot.state.step > target || snapshot.state.step < checkpoint.state.step
        }) {
            *cursor = None;
        }
        let snapshot = cursor.get_or_insert_with(|| checkpoint.clone());
        while snapshot.state.step < target {
            if let Err(error) = advance(config, snapshot, work) {
                // A partially advanced step must never seed a later query.
                *cursor = None;
                return Err(error);
            }
        }
        Ok(cursor.get_or_insert_with(|| checkpoint.clone()))
    }

    pub(super) fn sample(
        &self,
        config: &Config,
        cursor: &mut ReplayCursor,
        time: Time,
    ) -> Result<Arc<[Particle]>, ValidationError> {
        let Some(source) = self.mapped(config, time)? else {
            return Ok(Arc::from([]));
        };
        let mut work = 0;
        if let Some(period) = config.loop_period {
            let phase = source - config.prewarm;
            let other = Time::from_ticks(
                (phase.ticks() + period.ticks() as i64 / 2).rem_euclid(period.ticks() as i64),
            ) + config.prewarm;
            let amount = ((1.0
                - libm::cos(std::f64::consts::TAU * phase.as_seconds() / period.as_seconds()))
                * 0.5) as f32;
            let mut particles = Vec::new();
            for (time, weight, lane) in [(source, amount, 0), (other, 1.0 - amount, 1)] {
                if weight == 0.0 {
                    continue;
                }
                let snapshot = self.advance_to(config, &mut cursor.lanes[lane], time, &mut work)?;
                for live in &snapshot.state.live {
                    let mut particle = live.particle;
                    particle.index = particle.index * 2 + lane as u64;
                    particle.color = particle.color.map(|value| value * weight);
                    particles.push(particle);
                }
            }
            if particles.len() > config.limits.max_particles {
                return Err(limit("Loop population allowance exhausted"));
            }
            particles.sort_by_key(|particle| particle.index);
            return Ok(particles.into());
        }
        Ok(self
            .advance_to(config, &mut cursor.lanes[0], source, &mut work)?
            .state
            .live
            .iter()
            .map(|live| live.particle)
            .collect::<Vec<_>>()
            .into())
    }

    pub(super) fn events(
        &self,
        config: &Config,
        time: Time,
    ) -> Result<Vec<ParticleEvent>, ValidationError> {
        let Some(source) = self.mapped(config, time)? else {
            return Ok(Vec::new());
        };
        Ok(self
            .advance_to(config, &mut None, source, &mut 0)?
            .events
            .clone())
    }
}
