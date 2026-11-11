//! Random-access analytic particle trajectory models.

use super::*;
use motion_core::animation::value_noise;
use motion_core::{Point, Vector2};
use std::ops::Range;

/// A whole random-access particle trajectory, not separately composable forces.
pub trait TrajectoryModel: Send + Sync {
    /// Optional finite natural duration for timeline registration.
    fn duration(&self) -> Option<Duration> {
        None
    }

    /// Stable IDs born in [from,to).
    fn spawned(&self, from: Time, to: Time) -> Range<u64>;

    /// Exact integer-tick birth time of one stable ID.
    fn birth_time(&self, index: u64) -> Time;

    /// Complete state at this age; None means absent/dead.
    fn state(&self, index: u64, age_seconds: f32) -> Result<Option<Particle>, ValidationError>;
}

#[derive(Clone, Debug, PartialEq)]
struct TrajectoryDefinition {
    pub(crate) kind: u32,
    pub(crate) parameters: [f32; 8],
    pub(crate) count: u64,
    pub(crate) lifetime: f32,
    pub(crate) seed: u64,
    pub(crate) initial: Arc<[[f32; 4]]>,
}

impl TrajectoryDefinition {
    pub(crate) fn initial(&self, index: u64) -> Result<Particle, ValidationError> {
        if self.count > MAX_INSTANCE_COUNT as u64
            || !self.lifetime.is_finite()
            || self.lifetime <= 0.0
            || self.lifetime > 3600.0
            || self.parameters.iter().any(|value| !value.is_finite())
        {
            return Err(invalid(
                "trajectory",
                "Invalid builtin trajectory parameters/count/lifetime",
            ));
        }
        if (self.kind == 3 && self.parameters[2] < 0.0)
            || (self.kind == 4 && (self.parameters[4] < 0.0 || self.parameters[5] < 0.0))
        {
            return Err(invalid(
                "trajectory",
                "Radius, noise amplitude and frequency must be nonnegative",
            ));
        }
        let mut random = Random::from_key((self.seed, index));
        let mut particle = Particle {
            index,
            lifetime: self.lifetime,
            ..Default::default()
        };
        match self.kind {
            1 => {
                particle.position = [self.parameters[0], self.parameters[1]];
                let angle = random.next_float() * std::f64::consts::TAU;
                let speed = f64::from(self.parameters[6])
                    + (f64::from(self.parameters[7]) - f64::from(self.parameters[6]))
                        * random.next_float();
                particle.velocity = [
                    (libm::cos(angle) * speed) as f32,
                    (libm::sin(angle) * speed) as f32,
                ];
            }
            2 => {
                let points = self
                    .initial
                    .get(index as usize)
                    .ok_or_else(|| invalid("points", "Converge point index missing"))?;
                particle.position = [points[0], points[1]];
                particle.custom = [points[2], points[3], 0.0, 0.0];
            }
            3 => {
                particle.custom[0] = (random.next_float() * std::f64::consts::TAU) as f32;
            }
            4 => {
                particle.position = [self.parameters[0], self.parameters[1]];
                particle.velocity = [self.parameters[2], self.parameters[3]];
                particle.custom[0] = (random.next_float() * 32.0) as f32;
                particle.custom[1] = (random.next_float() * 32.0) as f32;
            }
            _ => return Err(invalid("model", "Unknown builtin trajectory")),
        }
        particle.validate()?;
        Ok(particle)
    }

    pub(crate) fn evaluate(
        &self,
        mut particle: Particle,
        age: f32,
    ) -> Result<Particle, ValidationError> {
        let time = f64::from(age);
        let values = self.parameters.map(f64::from);
        match self.kind {
            1 => {
                let drag = values[4];
                if drag < 0.0 || values[6] < 0.0 || values[7] < values[6] {
                    return Err(invalid(
                        "ballistic",
                        "Require nonnegative drag and ordered speeds",
                    ));
                }
                let product = drag * time;
                let (decay, advance, gravity) = if product.abs() < 0.01 {
                    let square = product * product;
                    let cube = square * product;
                    let fourth = cube * product;
                    (
                        libm::exp(-product),
                        time * (1.0 - product * 0.5 + square / 6.0 - cube / 24.0 + fourth / 120.0),
                        time * time
                            * (0.5 - product / 6.0 + square / 24.0 - cube / 120.0 + fourth / 720.0),
                    )
                } else {
                    let advance = -libm::expm1(-product) / drag;
                    (libm::exp(-product), advance, (time - advance) / drag)
                };
                for axis in 0..2 {
                    let velocity = f64::from(particle.velocity[axis]);
                    particle.position[axis] = (f64::from(particle.position[axis])
                        + velocity * advance
                        + values[2 + axis] * gravity)
                        as f32;
                    particle.velocity[axis] =
                        (velocity * decay + values[2 + axis] * advance) as f32;
                }
            }
            2 => {
                let progress = (time / f64::from(self.lifetime)).clamp(0.0, 1.0);
                let amount = progress * progress * (3.0 - 2.0 * progress);
                let derivative = 6.0 * progress * (1.0 - progress) / f64::from(self.lifetime);
                for axis in 0..2 {
                    let start = f64::from(particle.position[axis]);
                    let delta = f64::from(particle.custom[axis]) - start;
                    particle.position[axis] = (start + delta * amount) as f32;
                    particle.velocity[axis] = (delta * derivative) as f32;
                }
            }
            3 => {
                let angle = f64::from(particle.custom[0]) + values[3] * time;
                particle.position = [
                    (values[0] + values[2] * libm::cos(angle)) as f32,
                    (values[1] + values[2] * libm::sin(angle)) as f32,
                ];
                particle.velocity = [
                    (-values[2] * values[3] * libm::sin(angle)) as f32,
                    (values[2] * values[3] * libm::cos(angle)) as f32,
                ];
            }
            4 => {
                for axis in 0..2 {
                    let phase = f64::from(particle.custom[axis]);
                    let (value, derivative) = value_noise(
                        phase + time * values[5],
                        particle.index as u32 + axis as u32,
                    );
                    let (origin, _) = value_noise(phase, particle.index as u32 + axis as u32);
                    particle.position[axis] = (f64::from(particle.position[axis])
                        + f64::from(particle.velocity[axis]) * time
                        + values[4] * (value - origin))
                        as f32;
                    particle.velocity[axis] = (f64::from(particle.velocity[axis])
                        + values[4] * values[5] * derivative)
                        as f32;
                }
            }
            _ => unreachable!(),
        }
        particle.age = age;
        particle.validate()?;
        Ok(particle)
    }
}

/// Pure coupled-trajectory collection, without snapshots or previous-frame state.
#[derive(Clone)]
pub struct ParticleTrajectory {
    pub(crate) model: Arc<dyn TrajectoryModel>,
    pub(crate) style: Arc<ParticleStyle>,
}

impl ParticleTrajectory {
    /// Captures a pure coupled trajectory, without a simulation or snapshots.
    pub fn new(model: impl TrajectoryModel + 'static) -> Self {
        Self {
            model: Arc::new(model),
            style: Arc::new(ParticleStyle::default()),
        }
    }

    /// Returns stable-order live states. A malformed custom model fails explicitly.
    pub fn sample(&self, time: Time) -> Result<Arc<[Particle]>, ValidationError> {
        if time < Time::ZERO {
            return Ok(Arc::from([]));
        }
        let end = time
            .ticks()
            .checked_add(1)
            .map(Time::from_ticks)
            .ok_or_else(|| invalid("time", "Trajectory query overflow"))?;
        let range = self.model.spawned(Time::ZERO, end);
        if range.end < range.start || range.end - range.start > MAX_INSTANCE_COUNT as u64 {
            return Err(limit(
                "Trajectory birth range exceeds the instance allowance",
            ));
        }
        let mut particles = Vec::new();
        particles
            .try_reserve_exact((range.end - range.start) as usize)
            .map_err(|_| limit("Trajectory state allocation failed"))?;
        for index in range {
            let birth = self.model.birth_time(index);
            if birth < Time::ZERO || birth > time {
                return Err(invalid(
                    "birth",
                    "Custom model returned an invalid birth time",
                ));
            }
            if let Some(mut particle) = self
                .model
                .state(index, (time - birth).as_seconds() as f32)?
            {
                particle.index = index;
                particle.validate()?;
                if particle.age < particle.lifetime {
                    particles.push(particle);
                }
            }
        }
        Ok(particles.into())
    }
}

fn model(kind: u32, center: Point, count: u64) -> TrajectoryDefinition {
    TrajectoryDefinition {
        kind,
        parameters: [
            center.x as f32,
            center.y as f32,
            0.0,
            0.0,
            0.0,
            0.0,
            80.0,
            160.0,
        ],
        count,
        lifetime: 3.0,
        seed: 0,
        initial: Arc::from([]),
    }
}

macro_rules! common {
    ($name:ident) => {
        impl $name {
            /// Replaces deterministic initial-state seed.
            pub fn seed(mut self, seed: u64) -> Self {
                self.0.seed = seed;
                self
            }
            /// Replaces positive half-open lifetime in seconds.
            pub fn lifetime(mut self, lifetime: Duration) -> Self {
                self.0.lifetime = lifetime.as_seconds() as f32;
                self
            }
        }
        impl TrajectoryModel for $name {
            fn spawned(&self, from: Time, to: Time) -> Range<u64> {
                if from <= Time::ZERO && to > Time::ZERO {
                    0..self.0.count
                } else {
                    0..0
                }
            }
            fn birth_time(&self, _: u64) -> Time {
                Time::ZERO
            }
            fn state(&self, index: u64, age: f32) -> Result<Option<Particle>, ValidationError> {
                let initial = self.0.initial(index)?;
                if index >= self.0.count || !age.is_finite() {
                    return Err(invalid("age", "Invalid trajectory ID/age"));
                }
                if age < 0.0 || age >= self.0.lifetime {
                    return Ok(None);
                }
                Ok(Some(self.0.evaluate(initial, age)?))
            }
            fn duration(&self) -> Option<Duration> {
                if self.0.lifetime.is_finite() && self.0.lifetime > 0.0 && self.0.lifetime <= 3600.0
                {
                    Some(Duration::from_seconds(f64::from(self.0.lifetime)))
                } else {
                    None
                }
            }
        }
    };
}

/// Coupled constant gravity and linear drag, including the zero-drag limit.
#[derive(Clone, Debug)]
pub struct Ballistic(TrajectoryDefinition);

impl Ballistic {
    /// Creates a seeded radial burst with default speed 80..160 and life 3s.
    pub fn burst(center: Point, count: u64) -> Self {
        Self(model(1, center, count))
    }

    /// Replaces constant acceleration in units/squared second.
    pub fn gravity(mut self, gravity: Vector2) -> Self {
        self.0.parameters[2] = gravity.x as f32;
        self.0.parameters[3] = gravity.y as f32;
        self
    }

    /// Replaces nonnegative linear damping coefficient in reciprocal seconds.
    pub fn drag(mut self, drag: f64) -> Self {
        self.0.parameters[4] = drag as f32;
        self
    }

    /// Replaces deterministic radial speed range.
    pub fn speed(mut self, minimum: f64, maximum: f64) -> Self {
        self.0.parameters[6] = minimum as f32;
        self.0.parameters[7] = maximum as f32;
        self
    }
}
common!(Ballistic);

/// Smoothstep convergence between deterministic point sets, cycling the shorter.
#[derive(Clone, Debug)]
pub struct Converge(TrajectoryDefinition);

impl Converge {
    /// Prepares bounded source/target points; useful with VectorTree::sample_points.
    pub fn new(from: &[Point], to: &[Point], duration: Duration) -> Result<Self, ValidationError> {
        if from.is_empty() || to.is_empty() || from.len().max(to.len()) > MAX_INSTANCE_COUNT {
            return Err(invalid(
                "points",
                "Converge requires nonempty point sets within the instance allowance",
            ));
        }
        let mut definition = model(2, Point::ORIGIN, from.len().max(to.len()) as u64);
        definition.lifetime = duration.as_seconds() as f32;
        definition.initial = (0..definition.count as usize)
            .map(|index| {
                [
                    from[index % from.len()].x as f32,
                    from[index % from.len()].y as f32,
                    to[index % to.len()].x as f32,
                    to[index % to.len()].y as f32,
                ]
            })
            .collect::<Vec<_>>()
            .into();
        Ok(Self(definition))
    }
}
common!(Converge);

/// Exact circular orbits and tangent velocity, parameterized in radians/second.
#[derive(Clone, Debug)]
pub struct Orbit(TrajectoryDefinition);

impl Orbit {
    /// Creates stable seeded orbital phases.
    pub fn new(center: Point, radius: f64, count: u64) -> Self {
        let mut definition = model(3, center, count);
        definition.parameters[2] = radius as f32;
        definition.parameters[3] = 1.0;
        Self(definition)
    }

    /// Replaces angular velocity, including clockwise/counterclockwise motion.
    pub fn angular_velocity(mut self, velocity: f64) -> Self {
        self.0.parameters[3] = velocity as f32;
        self
    }
}
common!(Orbit);

/// Closed smooth value-noise displacement and its analytic velocity derivative.
#[derive(Clone, Debug)]
pub struct NoiseDrift(TrajectoryDefinition);

impl NoiseDrift {
    /// Creates seeded independent smooth noise phases, default amplitude 20/frequency 1.
    pub fn new(origin: Point, count: u64) -> Self {
        let mut definition = model(4, origin, count);
        definition.parameters[4] = 20.0;
        definition.parameters[5] = 1.0;
        Self(definition)
    }

    /// Replaces linear base velocity.
    pub fn velocity(mut self, velocity: Vector2) -> Self {
        self.0.parameters[2] = velocity.x as f32;
        self.0.parameters[3] = velocity.y as f32;
        self
    }

    /// Replaces nonnegative displacement amplitude and temporal frequency.
    pub fn noise(mut self, amplitude: f64, frequency: f64) -> Self {
        self.0.parameters[4] = amplitude as f32;
        self.0.parameters[5] = frequency as f32;
        self
    }
}
common!(NoiseDrift);

#[cfg(test)]
mod tests {
    use super::*;
    use motion_core::TimeLiteral;

    #[test]
    fn ballistic_solves_gravity_and_drag_together_with_stable_zero_limit() -> motion_core::Result<()>
    {
        for drag in [0.0, 1e-9, 0.8] {
            let model = Ballistic::burst(Point::new(2.0, 3.0), 1)
                .seed(42)
                .speed(5.0, 5.0)
                .gravity(Vector2::new(0.0, 10.0))
                .drag(drag);
            let start = model.state(0, 0.0)?.unwrap();
            let actual = model.state(0, 1.0)?.unwrap();
            let mut position = start.position.map(f64::from);
            let mut velocity = start.velocity.map(f64::from);
            let step = 1.0 / 20000.0;
            for _ in 0..20000 {
                for axis in 0..2 {
                    let force = if axis == 1 { 10.0 } else { 0.0 };
                    let acceleration = force - drag * velocity[axis];
                    position[axis] += velocity[axis] * step + 0.5 * acceleration * step * step;
                    velocity[axis] += acceleration * step;
                }
            }
            for axis in 0..2 {
                assert!((position[axis] - f64::from(actual.position[axis])).abs() < 0.001);
                assert!((velocity[axis] - f64::from(actual.velocity[axis])).abs() < 0.001);
            }
        }
        let converge = Converge::new(&[Point::ORIGIN], &[Point::new(10.0, 20.0)], 2.seconds())?;
        assert_eq!(converge.state(0, 1.0)?.unwrap().position, [5.0, 10.0]);
        let orbit = Orbit::new(Point::ORIGIN, 5.0, 1);
        let state = orbit.state(0, 1.0)?.unwrap();
        assert!(
            (state.position[0] * state.position[0] + state.position[1] * state.position[1] - 25.0)
                .abs()
                < 0.0001
        );
        let noise = NoiseDrift::new(Point::ORIGIN, 1).velocity(Vector2::new(1.0, 0.0));
        let before = noise.state(0, 0.999)?.unwrap();
        let after = noise.state(0, 1.001)?.unwrap();
        let exact = noise.state(0, 1.0)?.unwrap();
        for axis in 0..2 {
            assert!(
                ((after.position[axis] - before.position[axis]) / 0.002 - exact.velocity[axis])
                    .abs()
                    < 0.02
            );
        }
        let trajectory = ParticleTrajectory::new(orbit);
        assert_eq!(
            trajectory.sample(1.seconds().after_origin())?,
            trajectory.sample(1.seconds().after_origin())?
        );
        Ok(())
    }
}
