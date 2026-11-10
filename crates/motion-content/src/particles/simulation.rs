//! Deterministic fixed-step particle simulation with bounded snapshots and replay.

use super::*;

mod replay;

/// Logical snapshot/replay allowances, not a process-wide RSS sandbox.
#[derive(Clone, Copy, Debug)]
pub struct SimulationLimits {
    /// Maximum simultaneously living records.
    pub max_particles: usize,
    /// Snapshot payload plus two worst-case working buffers.
    pub max_bytes: u64,
    /// Maximum simulated fixed boundaries.
    pub max_steps: u64,
    /// Maximum preparation or per-query particle/operator work.
    pub max_work: u64,
    /// Maximum event-triggered descendant depth, at most 32.
    pub max_sub_depth: u8,
    /// Maximum nonrecycled births during preparation/replay.
    pub max_births: u64,
    /// Maximum events retained at one fixed boundary.
    pub max_events: usize,
}

impl Default for SimulationLimits {
    fn default() -> Self {
        Self {
            max_particles: 16384,
            max_bytes: 64 * 1024 * 1024,
            max_steps: 1_000_000,
            max_work: 100_000_000,
            max_sub_depth: 8,
            max_births: 1_000_000,
            max_events: 65536,
        }
    }
}

/// Actual immutable snapshot preparation facts.
#[derive(Clone, Copy, Debug)]
pub struct SimulationStats {
    /// Number of retained snapshots including step zero.
    pub snapshots: usize,
    /// Adaptive spacing in fixed steps.
    pub interval: u64,
    /// Conservative admitted snapshot/working payload bytes.
    pub admitted_bytes: u64,
    /// Actual preparation work units.
    pub work: u64,
}

/// Lazy fixed-step replay, created by [`ParticleSimulationBuilder::build`]. The first clip
/// duration fixes a shared preparation; reusing one cache instance with a different
/// duration is rejected explicitly. The configuration never changes after building.
#[derive(Clone)]
pub struct ParticleSimulation {
    config: Arc<Config>,
    pub(crate) style: Arc<ParticleStyle>,

    prepared: Arc<OnceLock<Result<Prepared, ValidationError>>>,
}

/// Collects a particle simulation's settings; [`build`](Self::build) validates them.
#[derive(Clone)]
pub struct ParticleSimulationBuilder {
    config: Config,
    pub(crate) style: Arc<ParticleStyle>,
}

impl ParticleSimulation {
    /// Starts a lazy fixed-step simulation with an explicit deterministic seed. Cache the
    /// built value with `SceneContext::try_cache` rather than building one afresh for every frame.
    #[allow(clippy::new_ret_no_self)]
    pub fn new(seed: u64) -> ParticleSimulationBuilder {
        ParticleSimulationBuilder::new(seed)
    }
}

impl ParticleSimulationBuilder {
    /// Starts a simulation with an explicit deterministic seed.
    pub fn new(seed: u64) -> Self {
        Self {
            config: Config {
                seed,
                duration: None,
                step: Time::from_ticks(motion_core::TICKS_PER_SECOND / 240),
                interval: 60,
                emitters: Vec::new(),
                modifiers: Vec::new(),
                groups: Vec::new(),
                sub_emitters: Vec::new(),
                prewarm: Time::ZERO,
                loop_period: None,
                time_scale: 1.0,
                time_offset: Time::ZERO,
                allow_reverse: false,
                workers: 1,
                limits: SimulationLimits::default(),
            },
            style: Arc::new(ParticleStyle::default()),
        }
    }

    fn change(mut self, update: impl FnOnce(&mut Config)) -> Self {
        update(&mut self.config);
        self
    }

    /// Sets finite preparation duration; otherwise render inherits clip duration.
    pub fn duration(self, duration: Duration) -> Self {
        self.change(|config| config.duration = Some(duration))
    }

    /// Sets an exact positive integer-tick step; no variable last integration step.
    pub fn step(self, step: Duration) -> Self {
        self.change(|config| config.step = step)
    }

    /// Requests snapshot spacing; memory admission may increase it automatically.
    pub fn snapshot_interval(self, steps: u64) -> Self {
        self.change(|config| config.interval = steps)
    }

    /// Replaces logical resource allowances.
    pub fn limits(self, limits: SimulationLimits) -> Self {
        self.change(|config| config.limits = limits)
    }

    /// Appends one fixed-boundary emitter, in declaration order.
    pub fn emitter(self, emitter: impl ParticleEmitter + 'static) -> Self {
        self.change(|config| config.emitters.push(Arc::new(emitter)))
    }

    /// Appends one deterministic operator; ordering is significant.
    pub fn modifier(self, modifier: impl ParticleModifier + 'static) -> Self {
        self.change(|config| config.modifiers.push(Arc::new(modifier)))
    }

    /// Adds a population operator before independent particle updates.
    pub fn group_modifier(self, modifier: impl ParticleGroupModifier + 'static) -> Self {
        self.change(|config| config.groups.push(Arc::new(modifier)))
    }

    /// Adds a stable-order, bounded event emitter rule.
    pub fn sub_emitter(self, emitter: SubEmitter) -> Self {
        self.change(|config| config.sub_emitters.push(emitter))
    }

    /// Prepares a steady-state lead-in without changing the visible clip duration.
    pub fn prewarm(self, duration: Duration) -> Self {
        self.change(|config| config.prewarm = duration)
    }

    /// Enables deterministic dual-phase seamless looping; two live populations share the limit.
    pub fn loop_period(self, period: Duration) -> Self {
        self.change(|config| config.loop_period = Some(period))
    }

    /// Remaps query time. Negative scales require allow_reverse, which explicitly revives past particles.
    pub fn time_remap(self, scale: f64, offset: Duration, allow_reverse: bool) -> Self {
        self.change(|config| {
            config.time_scale = scale;
            config.time_offset = offset;
            config.allow_reverse = allow_reverse;
        })
    }

    /// Requests 1..=64 CPU workers; independent updates retain stable IDs and event order.
    pub fn workers(self, workers: usize) -> Self {
        self.change(|config| config.workers = workers)
    }

    /// Checks everything that does not depend on the clip duration and freezes the
    /// simulation. Duration-dependent limits are checked when it is first sampled.
    pub fn build(self) -> Result<ParticleSimulation, ValidationError> {
        let config = &self.config;
        if config
            .duration
            .is_some_and(|duration| duration <= Time::ZERO)
            || config.step <= Time::ZERO
            || config.interval == 0
            || config.limits.max_particles == 0
            || config.limits.max_work == 0
            || config.emitters.len() > 32
            || config.modifiers.len() > 32
            || !config.time_scale.is_finite()
        {
            return Err(invalid(
                "configuration",
                "Invalid simulation duration/step/components/limits",
            ));
        }
        Ok(ParticleSimulation {
            config: Arc::new(self.config),
            style: self.style,
            prepared: Arc::new(OnceLock::new()),
        })
    }
}

impl ParticleSimulation {
    /// Reopens the settings for editing; the new simulation prepares afresh.
    pub fn to_builder(&self) -> ParticleSimulationBuilder {
        ParticleSimulationBuilder {
            config: self.config.as_ref().clone(),
            style: self.style.clone(),
        }
    }

    fn prepared(&self, duration: Option<Duration>) -> Result<&Prepared, ValidationError> {
        let duration = self.config.duration.or(duration).ok_or_else(|| {
            invalid(
                "duration",
                "Provide a simulation duration or a finite clip duration",
            )
        })?;
        let prepared = self
            .prepared
            .get_or_init(|| prepare(&self.config, duration))
            .as_ref()
            .map_err(Clone::clone)?;
        if prepared.duration != duration {
            return Err(invalid(
                "duration",
                "Cached simulation duration changed; use a distinct cache key/instance",
            ));
        }
        Ok(prepared)
    }

    /// Queries at the containing fixed boundary, never integrating a variable
    /// remainder. Outside [0,duration) is empty. Source initialization is shared.
    pub fn sample(&self, time: Time) -> Result<Arc<[Particle]>, ValidationError> {
        self.sample_in(time, None)
    }

    /// Queries using explicit finite clip metadata when no duration was configured.
    pub fn sample_in(
        &self,
        time: Time,
        clip_duration: Option<Duration>,
    ) -> Result<Arc<[Particle]>, ValidationError> {
        self.sample_with(&mut SampleCursor::default(), time, clip_duration)
    }

    /// Equals [`Self::sample_in`], but continues stepping from `cursor` when it already lies
    /// between the nearest retained snapshot and `time`, so ascending queries share work.
    pub(crate) fn sample_with(
        &self,
        cursor: &mut SampleCursor,
        time: Time,
        clip_duration: Option<Duration>,
    ) -> Result<Arc<[Particle]>, ValidationError> {
        let prepared = self.prepared(clip_duration)?;
        if time < Time::ZERO || time >= prepared.duration {
            return Ok(Arc::from([]));
        }
        if let Some(replay) = &prepared.replay {
            return replay.sample(&self.config, &mut cursor.replay, time);
        }
        let target = time.ticks() as u64 / self.config.step.ticks() as u64;
        let index = prepared
            .snapshots
            .partition_point(|snapshot| snapshot.step <= target)
            - 1;
        let checkpoint = &prepared.snapshots[index];
        if cursor
            .state
            .as_ref()
            .is_some_and(|state| state.step > target || state.step < checkpoint.step)
        {
            cursor.state = None;
        }
        let state = cursor.state.get_or_insert_with(|| checkpoint.clone());
        let mut work = 0;
        while state.step < target {
            if let Err(error) = advance(&self.config, state, &mut work) {
                // A partially advanced step must never seed a later query.
                cursor.state = None;
                return Err(error);
            }
        }
        Ok(state
            .live
            .iter()
            .map(|live| live.particle)
            .collect::<Vec<_>>()
            .into())
    }

    /// Returns preparation stats without evaluating a rendered frame.
    pub fn stats(&self) -> Result<SimulationStats, ValidationError> {
        Ok(self.prepared(None)?.stats)
    }

    /// Returns events at the containing fixed boundary, sorted by stable ID and trigger kind.
    pub fn events(&self, time: Time) -> Result<Vec<ParticleEvent>, ValidationError> {
        let prepared = self.prepared(None)?;
        if time < Time::ZERO || time >= prepared.duration {
            return Ok(Vec::new());
        }
        if let Some(replay) = &prepared.replay {
            replay.events(&self.config, time)
        } else {
            Ok(Vec::new())
        }
    }

    pub(super) fn natural_duration(&self) -> Option<Duration> {
        self.config.duration
    }
}

#[derive(Clone)]
struct Config {
    seed: u64,
    duration: Option<Duration>,
    step: Duration,
    interval: u64,
    emitters: Vec<Arc<dyn ParticleEmitter>>,
    modifiers: Vec<Arc<dyn ParticleModifier>>,

    groups: Vec<Arc<dyn ParticleGroupModifier>>,
    sub_emitters: Vec<SubEmitter>,
    prewarm: Duration,
    loop_period: Option<Duration>,
    time_scale: f64,
    time_offset: Duration,
    allow_reverse: bool,
    workers: usize,

    limits: SimulationLimits,
}

#[derive(Clone, Copy)]
struct Live {
    particle: Particle,
    birth: Time,
}

#[derive(Clone)]
struct State {
    step: u64,
    next: u64,
    live: Vec<Live>,
}

struct Prepared {
    duration: Duration,
    snapshots: Vec<State>,
    replay: Option<replay::PreparedReplay>,
    stats: SimulationStats,
}

/// Forward-only working state shared by the ascending queries of one render call.
///
/// It holds the admitted working buffers rather than extra retained snapshots, and is
/// dropped with the render, so it never carries state between frames.
#[derive(Default)]
pub(crate) struct SampleCursor {
    state: Option<State>,
    replay: replay::ReplayCursor,
}

fn charge(config: &Config, work: &mut u64, amount: usize) -> Result<(), ValidationError> {
    *work = work
        .checked_add(amount as u64)
        .ok_or_else(|| limit("Simulation work overflow"))?;
    if *work > config.limits.max_work {
        return Err(limit("Simulation work allowance exhausted"));
    }
    Ok(())
}

/// Counter-derived emitter stream, independent of iteration and worker count.
fn emitter_random(config: &Config, order: usize, step: u64) -> Random {
    Random::for_particle(config.seed ^ 0xe11e0000 ^ order as u64, step, 0)
}

/// Counter-derived modifier stream for one particle at one step.
fn modifier_random(config: &Config, order: usize, index: u64, step: u64) -> Random {
    Random::for_particle(
        config.seed ^ (order as u64).wrapping_mul(0xd1b54a32d192ed03),
        index,
        step,
    )
}

fn births(
    config: &Config,
    state: &mut State,
    time: Time,
    work: &mut u64,
) -> Result<(), ValidationError> {
    for (index, emitter) in config.emitters.iter().enumerate() {
        let mut output = Vec::new();
        let mut random = emitter_random(config, index, state.step);
        emitter.emit(state.step, time, &mut random, &mut output)?;
        charge(config, work, output.len() + 1)?;
        if output.len() > config.limits.max_particles.saturating_sub(state.live.len()) {
            return Err(limit("Live particle allowance exhausted"));
        }
        for mut particle in output {
            if state.next >= 1_000_000 {
                return Err(limit("Simulation exceeds one million total births"));
            }
            particle.index = state.next;
            particle.age = 0.0;
            particle.validate()?;
            state.next += 1;
            state.live.push(Live {
                particle,
                birth: time,
            });
        }
    }
    Ok(())
}

fn advance(config: &Config, state: &mut State, work: &mut u64) -> Result<(), ValidationError> {
    state.step += 1;
    let time = Time::from_ticks(state.step as i64 * config.step.ticks());
    let seconds = config.step.as_seconds() as f32;
    charge(
        config,
        work,
        state.live.len() * (config.modifiers.len() + 2),
    )?;
    for live in &mut state.live {
        let particle = &mut live.particle;
        particle.age = (time - live.birth).as_seconds() as f32;
        if particle.age >= particle.lifetime {
            continue;
        }
        let index = particle.index;
        let age = particle.age;
        for (order, modifier) in config.modifiers.iter().enumerate() {
            let mut random = modifier_random(config, order, index, state.step);
            modifier.apply(particle, seconds, time, &mut random)?;
            particle.index = index;
            particle.age = age;
            particle.validate()?;
        }
        for axis in 0..2 {
            particle.position[axis] += particle.velocity[axis] * seconds;
        }
        particle.validate()?;
    }
    state
        .live
        .retain(|live| live.particle.age < live.particle.lifetime);
    births(config, state, time, work)
}

fn prepare(config: &Config, duration: Duration) -> Result<Prepared, ValidationError> {
    if replay::enabled(config) {
        let replay = replay::prepare(config, duration)?;
        let stats = replay.stats;
        return Ok(Prepared {
            duration,
            snapshots: Vec::new(),
            replay: Some(replay),
            stats,
        });
    }
    let limits = config.limits;
    if duration <= Time::ZERO
        || duration.as_seconds() > 3600.0
        || config.step <= Time::ZERO
        || config.step > duration
        || config.interval == 0
        || limits.max_particles == 0
        || limits.max_particles > 65536
        || limits.max_work == 0
        || config.emitters.len() > 32
        || config.modifiers.len() > 32
    {
        return Err(invalid(
            "configuration",
            "Invalid simulation duration/step/components/limits",
        ));
    }
    let steps = duration.ticks() as u64 / config.step.ticks() as u64;
    if steps > limits.max_steps || steps > 1_000_000 {
        return Err(limit("Simulation fixed-step allowance exhausted"));
    }
    let slot = limits.max_particles as u64 * std::mem::size_of::<Live>() as u64
        + std::mem::size_of::<State>() as u64;
    let slots = limits.max_bytes / slot;
    if slots < 3 {
        return Err(limit(
            "Snapshot budget needs one snapshot and two bounded working buffers",
        ));
    }
    let capacity = slots - 2;
    let interval = if capacity == 1 {
        steps + 1
    } else {
        config.interval.max(steps.div_ceil(capacity - 1))
    };
    let mut state = State {
        step: 0,
        next: 0,
        live: Vec::new(),
    };
    let mut work = 0;
    births(config, &mut state, Time::ZERO, &mut work)?;
    let mut snapshots = vec![state.clone()];
    while state.step < steps {
        advance(config, &mut state, &mut work)?;
        if state.step.is_multiple_of(interval) {
            snapshots.push(state.clone());
        }
    }
    let stats = SimulationStats {
        snapshots: snapshots.len(),
        interval,
        admitted_bytes: (snapshots.len() as u64 + 2) * slot,
        work,
    };
    Ok(Prepared {
        duration,
        snapshots,
        replay: None,
        stats,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use motion_core::TimeLiteral;
    use std::sync::atomic::{AtomicUsize, Ordering};

    struct Birth(Arc<AtomicUsize>);

    impl ParticleEmitter for Birth {
        fn emit(
            &self,
            step: u64,
            _: Time,
            _: &mut Random,
            output: &mut Vec<Particle>,
        ) -> Result<(), ValidationError> {
            self.0.fetch_add(1, Ordering::SeqCst);
            if step == 0 {
                output.push(Particle {
                    velocity: [1.0, 0.0],
                    lifetime: 2.0,
                    ..Default::default()
                });
            }
            Ok(())
        }
    }

    struct Acceleration;

    impl ParticleModifier for Acceleration {
        fn apply(
            &self,
            particle: &mut Particle,
            step: f32,
            _: Time,
            _: &mut Random,
        ) -> Result<(), ValidationError> {
            particle.velocity[1] += 2.0 * step;
            Ok(())
        }
    }

    #[test]
    fn fixed_snapshot_replay_is_order_independent_once_initialized_and_bounded()
    -> motion_core::Result<()> {
        assert_eq!(std::mem::size_of::<Particle>(), 72);
        let calls = Arc::new(AtomicUsize::new(0));
        let simulation = ParticleSimulation::new(42)
            .duration(1.seconds())
            .step(10.milliseconds())
            .emitter(Birth(calls.clone()))
            .modifier(Acceleration)
            .limits(SimulationLimits {
                max_particles: 2,
                max_bytes: 2048,
                ..Default::default()
            })
            .snapshot_interval(1)
            .build()?;
        std::thread::scope(|scope| {
            for _ in 0..4 {
                let simulation = &simulation;
                scope.spawn(move || {
                    let point = simulation.sample(500.milliseconds()).unwrap()[0];
                    assert!((point.position[0] - 0.5).abs() < 1e-6);
                    assert!((point.position[1] - 0.255).abs() < 1e-6);
                });
            }
        });
        assert_eq!(calls.load(Ordering::SeqCst), 101 + 4 * 5);
        let stats = simulation.stats()?;
        assert!(stats.interval > 1 && stats.admitted_bytes <= 2048);
        for time in [990, 0, 500, 40, 500] {
            assert_eq!(
                simulation.sample(time.milliseconds())?,
                simulation.sample(time.milliseconds())?
            );
        }
        assert!(simulation.sample(1.seconds())?.is_empty());
        assert!(
            simulation
                .clone()
                .to_builder()
                .limits(SimulationLimits {
                    max_bytes: 1,
                    ..Default::default()
                })
                .build()?
                .sample(Time::ZERO)
                .is_err()
        );
        Ok(())
    }

    #[test]
    fn cursor_queries_match_independent_samples_for_every_replay_path() -> motion_core::Result {
        let plain = ParticleSimulation::new(9)
            .duration(2.seconds())
            .emitter(
                crate::particles::PointEmitter::new(motion_core::Point::ORIGIN)
                    .speed(3.0)
                    .lifetime(700.milliseconds()),
            )
            .modifier(Acceleration)
            .build()?;
        let simulations = [
            plain.clone(),
            plain.clone().to_builder().workers(2).build()?,
            plain.to_builder().loop_period(900.milliseconds()).build()?,
        ];
        // Ascending runs that cross snapshots, repeat a time, jump back and wrap the loop.
        let times = [
            0, 10, 240, 250, 251, 300, 300, 1200, 1490, 260, 270, 1999, 5, 2000, 1000,
        ];
        for simulation in &simulations {
            let mut cursor = SampleCursor::default();
            for time in times {
                let time = Time::from_ticks(motion_core::TICKS_PER_SECOND * time / 1000);
                assert_eq!(
                    &*simulation.sample_with(&mut cursor, time, None)?,
                    &*simulation.sample_in(time, None)?,
                    "{time:?}"
                );
            }
        }
        Ok(())
    }
}
