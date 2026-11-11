//! Particle emitters and emission settings; modifiers live in `modifiers`.

use super::*;
use crate::VectorTree;
use motion_core::{Animation, Path2D, Point, Rect, Vector2};

mod image;
mod modifiers;

pub use image::ImageDensity;
use image::ImageDistribution;
pub use modifiers::{
    Attractor, BounceBox, ColorOverLifetime, Drag, Gravity, Seek, SizeOverLifetime, Vortex, Wind,
};

#[cfg(test)]
mod tests;

/// Default constant emission rate in particles per second.
const DEFAULT_RATE: f64 = 100.0;

/// Default particle lifetime in seconds.
const DEFAULT_LIFETIME_SECONDS: f64 = 2.0;

/// Default isotropic launch speed in design units per second.
const DEFAULT_SPEED: f64 = 40.0;

macro_rules! emitter {
    ($name:ident,$doc:literal) => {
        #[doc=$doc]
        #[derive(Clone)]
        pub struct $name(EmitterSettings);
        impl $name {
            /// Replaces particles/second; constant schedules quantize to step boundaries.
            pub fn rate(mut self, rate: f64) -> Self {
                self.0.rate = EmissionRate::Constant(rate);
                self
            }
            /// Uses seeded stochastic fractional births at each fixed step; its
            /// expected birth count follows the animation, without hidden cursors.
            pub fn rate_animation(mut self, rate: impl Animation<f64> + 'static) -> Self {
                self.0.rate = EmissionRate::Animated(Arc::new(rate));
                self
            }
            /// Replaces half-open particle lifetime.
            pub fn lifetime(mut self, lifetime: Duration) -> Self {
                self.0.lifetime = lifetime;
                self
            }
            /// Replaces isotropic launch speed (path sources use path tangents).
            pub fn speed(mut self, speed: f64) -> Self {
                self.0.speed = speed;
                self
            }
            /// Adds a fixed design-unit velocity to the sampled launch direction.
            pub fn velocity(mut self, velocity: Vector2) -> Self {
                self.0.velocity = velocity;
                self
            }
            /// Sets finite premultiplied birth color.
            pub fn color(mut self, color: [f32; 4]) -> Self {
                self.0.color = color;
                self
            }
            /// Sets birth size in 0..=4096.
            pub fn size(mut self, size: f32) -> Self {
                self.0.size = size;
                self
            }
            /// Samples depth into `custom[1]`; a constant range consumes no random draw.
            pub fn depth_range(mut self, minimum: f32, maximum: f32) -> Self {
                self.0.depth = [minimum, maximum];
                self
            }
            /// Multiplies the birth color by a solid or gradient paint evaluated at the
            /// sampled emission point; `bounds` is the paint's objectBoundingBox.
            /// Consumes no random draws, so positions and timing are unchanged.
            pub fn paint(
                mut self,
                paint: motion_core::Paint,
                bounds: Rect,
            ) -> Result<Self, ValidationError> {
                motion_core::drawing::validate_rect(bounds, "particles.emitter", "paint bounds")?;
                if !matches!(
                    paint,
                    motion_core::Paint::Solid(_)
                        | motion_core::Paint::Linear(_)
                        | motion_core::Paint::Radial(_)
                        | motion_core::Paint::Sweep(_)
                ) {
                    return Err(invalid("paint", "Emitter paint must be solid or gradient"));
                }
                self.0.paint = Some(Arc::new((paint, bounds)));
                Ok(self)
            }
            /// Selects a bounded deterministic premultiplied birth palette.
            pub fn palette(mut self, colors: Arc<[[f32; 4]]>) -> Self {
                self.0.palette = Some(colors);
                self
            }
        }
        impl ParticleEmitter for $name {
            fn emit(
                &self,
                step: u64,
                time: Time,
                random: &mut Random,
                output: &mut Vec<Particle>,
            ) -> Result<(), ValidationError> {
                self.0.emit(step, time, random, output)
            }
        }
    };
}

emitter!(
    PointEmitter,
    "A point source with stable fixed-boundary births."
);

emitter!(
    DiskEmitter,
    "A uniform area disk source, not a circumference distribution."
);

emitter!(
    RectEmitter,
    "A uniform rectangular source in local design coordinates."
);

emitter!(
    PathEmitter,
    "An arc-length path source inheriting its tangent direction."
);

emitter!(
    SurfaceEmitter,
    "An area-uniform shared VectorSurface source."
);

emitter!(
    ImageEmitter,
    "An image source weighted by alpha or linear luminance, inheriting pixel colors."
);

impl PointEmitter {
    /// Creates a point source.
    pub fn new(point: Point) -> Self {
        Self(EmitterSettings::new(EmitterSource::Point(point)))
    }
}

impl DiskEmitter {
    /// Creates a uniform disk source.
    pub fn new(center: Point, radius: f64) -> Self {
        Self(EmitterSettings::new(EmitterSource::Disk(center, radius)))
    }
}

impl RectEmitter {
    /// Creates a uniform rectangle source.
    pub fn new(rect: Rect) -> Self {
        Self(EmitterSettings::new(EmitterSource::Rect(rect)))
    }
}

impl PathEmitter {
    /// Captures immutable curves without flattening.
    pub fn new(path: Arc<Path2D>) -> Self {
        Self(EmitterSettings::new(EmitterSource::Path(path)))
    }

    /// Advances along an animated normalized arc length, retaining tangent launch velocity.
    pub fn along_path(mut self, progress: impl Animation<f64> + 'static) -> Self {
        self.0.path_progress = Some(Arc::new(progress));
        self
    }
}

impl SurfaceEmitter {
    /// Captures a shared tree; its default area triangulation is cached.
    pub fn new(tree: Arc<VectorTree>) -> Self {
        Self(EmitterSettings::new(EmitterSource::Surface(tree)))
    }

    /// Enables fill-color inheritance without changing area density or sampled positions.
    pub fn inherit_color(mut self, enabled: bool) -> Self {
        self.0.inherit_color = enabled;
        self
    }

    /// Captures already-shaped glyph outlines through the shared vector surface.
    #[cfg(feature = "text")]
    pub fn from_text(
        text: &crate::Text,
        context: &motion_core::DrawContext,
    ) -> Result<Self, ValidationError> {
        Ok(Self::new(text.vector_tree(context)?).inherit_color(true))
    }
}

/// A point burst emitted once at the first fixed boundary at/after its authored time.
#[derive(Clone)]
pub struct BurstEmitter {
    time: Time,
    count: usize,
    settings: EmitterSettings,
}

impl BurstEmitter {
    /// Creates a bounded timed point burst.
    pub fn new(time: Time, count: usize, position: Point) -> Self {
        Self {
            time,
            count,
            settings: EmitterSettings::new(EmitterSource::Point(position)),
        }
    }

    /// Replaces lifetime for every burst record.
    pub fn lifetime(mut self, lifetime: Duration) -> Self {
        self.settings.lifetime = lifetime;
        self
    }

    /// Replaces random radial speed.
    pub fn speed(mut self, speed: f64) -> Self {
        self.settings.speed = speed;
        self
    }

    /// Adds a fixed launch velocity to the radial burst.
    pub fn velocity(mut self, velocity: Vector2) -> Self {
        self.settings.velocity = velocity;
        self
    }

    /// Sets finite premultiplied birth color.
    pub fn color(mut self, color: [f32; 4]) -> Self {
        self.settings.color = color;
        self
    }

    /// Sets birth size.
    pub fn size(mut self, size: f32) -> Self {
        self.settings.size = size;
        self
    }

    /// Sets a finite ordered depth range.
    pub fn depth_range(mut self, minimum: f32, maximum: f32) -> Self {
        self.settings.depth = [minimum, maximum];
        self
    }

    /// Selects a deterministic birth palette.
    pub fn palette(mut self, colors: Arc<[[f32; 4]]>) -> Self {
        self.settings.palette = Some(colors);
        self
    }
}

impl ParticleEmitter for BurstEmitter {
    fn emit(
        &self,
        step: u64,
        time: Time,
        random: &mut Random,
        output: &mut Vec<Particle>,
    ) -> Result<(), ValidationError> {
        if self.time < Time::ZERO || self.count > 65536 {
            return Err(invalid("burst", "Invalid burst time/count"));
        }
        let previous = if step == 0 {
            Time::from_ticks(-1)
        } else {
            Time::from_ticks(time.ticks() - (time.ticks() / step as i64))
        };
        if self.time > previous && self.time <= time {
            for _ in 0..self.count {
                output.push(self.settings.particle(random, time)?);
            }
        }
        Ok(())
    }
}

#[derive(Clone)]
enum EmitterSource {
    Point(Point),
    Disk(Point, f64),
    Rect(Rect),
    Path(Arc<Path2D>),
    Surface(Arc<VectorTree>),
    Image(Arc<ImageDistribution>),
}

#[derive(Clone)]
enum EmissionRate {
    Constant(f64),
    Animated(Arc<dyn Animation<f64>>),
}

#[derive(Clone)]
struct EmitterSettings {
    source: EmitterSource,
    rate: EmissionRate,
    lifetime: Duration,
    speed: f64,
    velocity: Vector2,
    inherit_color: bool,
    color: [f32; 4],
    size: f32,
    depth: [f32; 2],
    palette: Option<Arc<[[f32; 4]]>>,
    path_progress: Option<Arc<dyn Animation<f64>>>,
    paint: Option<Arc<(motion_core::Paint, Rect)>>,
}

impl EmitterSettings {
    fn new(source: EmitterSource) -> Self {
        Self {
            source,
            rate: EmissionRate::Constant(DEFAULT_RATE),
            lifetime: Duration::from_seconds(DEFAULT_LIFETIME_SECONDS),
            speed: DEFAULT_SPEED,
            velocity: Vector2::ZERO,
            inherit_color: false,
            color: [1.0; 4],
            size: 1.0,
            depth: [0.0; 2],
            palette: None,
            path_progress: None,
            paint: None,
        }
    }

    fn paint_color(&self, point: Point) -> Result<Option<[f32; 4]>, ValidationError> {
        let Some(paint) = &self.paint else {
            return Ok(None);
        };
        let (paint, bounds) = (&paint.0, paint.1);
        let sample = match paint {
            motion_core::Paint::Solid(color) => color.to_linear_premultiplied(),
            motion_core::Paint::Linear(gradient) => gradient.sample(point, bounds)?,
            motion_core::Paint::Radial(gradient) => gradient.sample(point, bounds)?,
            motion_core::Paint::Sweep(gradient) => gradient.sample(point, bounds)?,
            _ => {
                return Err(invalid("paint", "Emitter paint must be solid or gradient"));
            }
        };
        Ok(Some(sample.map(|value| value as f32)))
    }

    fn particle(&self, random: &mut Random, time: Time) -> Result<Particle, ValidationError> {
        if self.lifetime <= Duration::ZERO
            || self.lifetime.as_seconds() > f64::from(MAX_LIFETIME_SECONDS)
            || !self.speed.is_finite()
            || self.speed < 0.0
        {
            return Err(invalid("emitter", "Invalid emitter lifetime/speed"));
        }
        let mut direction = random.unit_vector();
        let mut color = self.color;
        let point = match &self.source {
            EmitterSource::Point(point) => *point,
            EmitterSource::Disk(center, radius) => {
                if !radius.is_finite() || *radius < 0.0 {
                    return Err(invalid(
                        "radius",
                        "Disk radius must be finite and nonnegative",
                    ));
                }
                let distance = libm::sqrt(random.next_float()) * radius;
                Point::new(
                    center.x + direction.x * distance,
                    center.y + direction.y * distance,
                )
            }
            EmitterSource::Rect(rect) => {
                motion_core::drawing::validate_rect(*rect, "particles.emitter", "rect")?;
                Point::new(
                    rect.left + rect.width() * random.next_float(),
                    rect.top + rect.height() * random.next_float(),
                )
            }
            EmitterSource::Path(path) => {
                let progress = if let Some(animation) = &self.path_progress {
                    animation.sample(time.since_origin())
                } else {
                    random.next_float()
                };
                let (point, tangent) = path.try_point_at(progress)?;
                direction = tangent;
                point
            }
            EmitterSource::Surface(tree) => {
                let seed = (random.next_float() * u64::MAX as f64) as u64;
                if self.inherit_color {
                    let (point, sample) = tree.sample_colored_points(1, seed)?[0];
                    color = std::array::from_fn(|channel| color[channel] * sample[channel]);
                    point
                } else {
                    tree.sample_points(1, seed)?[0]
                }
            }
            EmitterSource::Image(distribution) => {
                let (point, sample) = distribution.sample(random)?;
                color = std::array::from_fn(|channel| color[channel] * sample[channel]);
                point
            }
        };
        if let Some(sample) = self.paint_color(point)? {
            color = std::array::from_fn(|channel| color[channel] * sample[channel]);
        }
        if let Some(palette) = &self.palette {
            if palette.is_empty() || palette.len() > 256 {
                return Err(invalid(
                    "palette",
                    "Require 1..=256 finite premultiplied colors",
                ));
            }
            for color in palette.iter() {
                Particle {
                    color: *color,
                    ..Particle::default()
                }
                .validate()?;
            }
            let index =
                ((random.next_float() * palette.len() as f64) as usize).min(palette.len() - 1);
            color = std::array::from_fn(|channel| color[channel] * palette[index][channel]);
        }
        if self.depth.iter().any(|value| !value.is_finite()) || self.depth[0] > self.depth[1] {
            return Err(invalid("depth", "Depth range must be finite and ordered"));
        }
        let depth = if self.depth[0] == self.depth[1] {
            self.depth[0]
        } else {
            self.depth[0] + (self.depth[1] - self.depth[0]) * random.next_float() as f32
        };
        let particle = Particle {
            position: [point.x as f32, point.y as f32],
            velocity: [
                (direction.x * self.speed + self.velocity.x) as f32,
                (direction.y * self.speed + self.velocity.y) as f32,
            ],
            lifetime: self.lifetime.as_seconds() as f32,
            color,
            size: self.size,
            custom: [0.0, depth, 0.0, 0.0],
            ..Default::default()
        };
        particle.validate()?;
        Ok(particle)
    }

    fn emit(
        &self,
        step: u64,
        time: Time,
        random: &mut Random,
        output: &mut Vec<Particle>,
    ) -> Result<(), ValidationError> {
        let rate = match &self.rate {
            EmissionRate::Constant(rate) => *rate,
            EmissionRate::Animated(animation) => animation.sample(time.since_origin()),
        };
        if !rate.is_finite() || !(0.0..=1_000_000.0).contains(&rate) {
            return Err(invalid(
                "rate",
                "Emitter rate must be finite in 0..=1000000",
            ));
        }
        let seconds = time.as_seconds();
        let delta = if step == 0 {
            1.0 / 240.0
        } else {
            Time::from_ticks(time.ticks() / step as i64).as_seconds()
        };
        let previous = if step == 0 {
            0.0
        } else {
            Time::from_ticks(time.ticks() - time.ticks() / step as i64).as_seconds()
        };
        let count = match self.rate {
            EmissionRate::Constant(_) if rate == 0.0 => 0,
            EmissionRate::Constant(_) => {
                if step == 0 {
                    1
                } else {
                    (libm::floor(seconds * rate) - libm::floor(previous * rate)).max(0.0) as usize
                }
            }
            EmissionRate::Animated(_) if step == 0 => 0,
            EmissionRate::Animated(_) => {
                let expected = rate * delta;
                libm::floor(expected) as usize
                    + usize::from(random.next_float() < expected - libm::floor(expected))
            }
        };
        if count > 65536 {
            return Err(limit("Emitter step exceeds 65536 births"));
        }
        for _ in 0..count {
            output.push(self.particle(random, time)?);
        }
        Ok(())
    }
}
