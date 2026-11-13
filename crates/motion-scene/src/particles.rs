//! Particle simulations and trajectories described as plain data.

use crate::paint::PaintSpec;
use crate::value::Animated;
use motion_content::Preset;
use motion_core::instances::InstanceBlend;
use motion_core::{Color, Point, Rect};
use schemars::{JsonSchema, Schema, SchemaGenerator};
use serde::{Deserialize, Serialize};
use std::borrow::Cow;

mod configuration;
mod materials;
mod operators;
mod sources;

pub use configuration::{ParticleConfigurationSpec, ParticleCurveSpec, ParticleTrailSpec};
pub use materials::{ParticleMaterialSpec, ParticleSheetSpec, ParticleSpriteSpec};
pub use operators::{ParticleFalloffSpec, ParticleOperatorSpec, ParticleSubEmitterSpec};
pub use sources::ParticlePointsSpec;
pub use sources::{
    ParticleImageSourceSpec, ParticleImageSpec, ParticleSurfaceSpec, ParticleTextSourceSpec,
};

/// A seeded, deterministic particle simulation drawn into the scene.
///
/// Emitters spawn particles, modifiers act on them in declaration order and the
/// style draws them, or a shared preset supplies the complete recipe. Simulation
/// time is scene-local unless `sample_time` overrides it. Presets require an
/// explicit `duration`; hand-authored simulations otherwise use the scene duration.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
#[schemars(extend("allOf" = [{
    "if": {"required": ["preset"], "properties": {
        "preset": {"type": "object", "required": ["kind"]}
    }},
    "then": {"required": ["duration"], "properties": {
        "duration": {"not": {"type": "null"}, "exclusiveMinimum": 0, "maximum": 3600}
    }}
}]))]
pub struct ParticlesSpec {
    /// Explicit deterministic seed.
    pub seed: u64,
    /// Shared recipe instead of explicit emitters and modifiers.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub preset: Option<ParticlePresetSpec>,
    /// Simulation sample time in `0..=1e6` seconds; scene-local time when omitted.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[schemars(extend("minimum" = 0.0, "maximum" = 1e6))]
    pub sample_time: Option<Animated<f64>>,
    /// Simulated seconds; required in `(0, 3600]` for presets, otherwise the scene duration by default.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[schemars(extend("exclusiveMinimum" = 0))]
    pub duration: Option<f64>,
    /// Fixed simulation step in seconds.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[schemars(extend("exclusiveMinimum" = 0))]
    pub step: Option<f64>,
    /// Particle count limit; the engine default (16384) when omitted.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[schemars(range(min = 1, max = 1_000_000))]
    pub max_particles: Option<u32>,
    /// Particle sources.
    #[serde(default)]
    pub emitters: Vec<EmitterSpec>,
    /// Forces and lifetime curves, applied in order.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub modifiers: Vec<ModifierSpec>,
    /// How particles are drawn; dots by default.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub style: Option<ParticleStyleSpec>,
    /// Compositing of the particle layer.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub blend: Option<InstanceBlend>,
    /// Optional time mapping, work allowance and batch presentation policy.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub configuration: Option<Box<ParticleConfigurationSpec>>,
    /// Ordered event-triggered local emitters.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub sub_emitters: Vec<ParticleSubEmitterSpec>,
}

impl ParticlesSpec {
    /// Creates a simulation with no emitters.
    pub fn new(seed: u64) -> Self {
        Self {
            seed,
            preset: None,
            sample_time: None,
            duration: None,
            step: None,
            max_particles: None,
            emitters: Vec::new(),
            modifiers: Vec::new(),
            style: None,
            blend: None,
            configuration: None,
            sub_emitters: Vec::new(),
        }
    }

    /// Adds an emitter.
    pub fn emitter(mut self, emitter: EmitterSpec) -> Self {
        self.emitters.push(emitter);
        self
    }

    /// Adds a modifier.
    pub fn modifier(mut self, modifier: ModifierSpec) -> Self {
        self.modifiers.push(modifier);
        self
    }

    /// Selects how particles are drawn.
    pub fn style(mut self, style: ParticleStyleSpec) -> Self {
        self.style = Some(style);
        self
    }
}

/// A shared particle recipe with explicit local bounds and optional adjustments.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ParticlePresetSpec {
    /// Recipe name.
    pub kind: Preset,
    /// Local design bounds `[left, top, right, bottom]`.
    pub bounds: [f64; 4],
    /// Density multiplier in `(0, 16]`; one by default.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[schemars(range(max = 16.0), extend("exclusiveMinimum" = 0))]
    pub density: Option<f64>,
    /// Motion multiplier in `(0, 16]`; one by default.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[schemars(range(max = 16.0), extend("exclusiveMinimum" = 0))]
    pub speed: Option<f64>,
    /// HDR intensity multiplier in `0..=64`; one by default.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[schemars(range(min = 0.0, max = 64.0))]
    pub intensity: Option<f32>,
    /// At most 256 fixed colored paths required by logo recipes, using the recipe bounds.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    #[schemars(length(max = 256))]
    pub logo: Vec<ParticleVectorPathSpec>,
}

/// A fixed painted path in a particle preset's colored vector source.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ParticleVectorPathSpec {
    /// SVG path data.
    pub data: String,
    /// Fixed solid color or gradient; animated paints are rejected.
    pub fill: PaintSpec,
}

/// Where particles are born.
#[derive(Clone, Debug, PartialEq)]
pub enum EmitterShape {
    /// A single point `[x, y]`.
    Point(Point),
    /// A disk.
    Disk {
        /// Center `[x, y]`.
        center: Point,
        /// Radius.
        radius: f64,
    },
    /// A rectangle `[left, top, right, bottom]`.
    Rect(Rect),
    /// Points along SVG path data.
    Path(String),
    /// Painted vector surface.
    Surface(ParticleSurfaceSpec),
    /// Image alpha or luminance distribution.
    Image(ParticleImageSourceSpec),
    /// Explicitly shaped glyph surface.
    Text(ParticleTextSourceSpec),
    /// One instantaneous burst.
    Burst {
        /// Scene-local birth time in seconds.
        time: f64,
        /// Number of particles.
        count: u32,
        /// Birth point `[x, y]`.
        position: Point,
    },
}

/// A particle source, written with one shape key (`point`, `disk`, `rect`, `path`,
/// `surface`, `image`, `text` or `burst`) next to optional birth settings. Colors are authored sRGB, premultiplied
/// internally. `rate` applies to continuous emitters, not bursts.
///
/// ```
/// use motion_scene::{EmitterShape, EmitterSpec};
/// let json = r##"{"disk": {"center": [40, 40], "radius": 6}, "rate": 80, "color": "#ffcc66"}"##;
/// let emitter: EmitterSpec = serde_json::from_str(json)?;
/// assert!(matches!(emitter.shape, EmitterShape::Disk { .. }));
/// assert_eq!(emitter.rate, Some(80.0));
/// # Ok::<(), serde_json::Error>(())
/// ```
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(try_from = "RawEmitter", into = "RawEmitter")]
pub struct EmitterSpec {
    /// Birth geometry.
    pub shape: EmitterShape,
    /// Particles per second; 100 by default.
    pub rate: Option<f64>,
    /// Seconds a particle lives; 2 by default.
    pub lifetime: Option<f64>,
    /// Launch speed in design units per second; 40 by default.
    pub speed: Option<f64>,
    /// Added velocity `[x, y]`.
    pub velocity: Option<[f64; 2]>,
    /// Birth color.
    pub color: Option<Color>,
    /// Birth size in `0..=4096`.
    pub size: Option<f64>,
    /// Depth range `[near, far]`.
    pub depth_range: Option<[f64; 2]>,
    /// Animated normalized arc-length progress for path sources.
    pub path_progress: Option<Animated<f64>>,
    /// Explicit premultiplied linear birth color, overriding authored sRGB color.
    pub premultiplied_color: Option<[f32; 4]>,
}

impl EmitterSpec {
    /// Creates an emitter with engine-default birth settings.
    pub fn new(shape: EmitterShape) -> Self {
        Self {
            shape,
            rate: None,
            lifetime: None,
            speed: None,
            velocity: None,
            color: None,
            size: None,
            depth_range: None,
            path_progress: None,
            premultiplied_color: None,
        }
    }

    /// Sets the emission rate.
    pub fn rate(mut self, rate: f64) -> Self {
        self.rate = Some(rate);
        self
    }

    /// Sets the particle lifetime in seconds.
    pub fn lifetime(mut self, seconds: f64) -> Self {
        self.lifetime = Some(seconds);
        self
    }

    /// Sets the launch speed.
    pub fn speed(mut self, speed: f64) -> Self {
        self.speed = Some(speed);
        self
    }

    /// Sets the birth color.
    pub fn color(mut self, color: impl Into<Color>) -> Self {
        self.color = Some(color.into());
        self
    }
}

impl JsonSchema for EmitterSpec {
    fn schema_name() -> Cow<'static, str> {
        "EmitterSpec".into()
    }

    fn json_schema(generator: &mut SchemaGenerator) -> Schema {
        RawEmitter::json_schema(generator)
    }
}

/// A disk emitter's geometry.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct DiskShape {
    /// Center `[x, y]`.
    pub center: Point,
    /// Radius.
    pub radius: f64,
}

/// A burst emitter's geometry.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct BurstShape {
    /// Scene-local birth time in seconds.
    pub time: f64,
    /// Number of particles.
    pub count: u32,
    /// Birth point `[x, y]`.
    pub position: Point,
}

#[derive(Default, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
struct RawEmitter {
    /// Point emitter `[x, y]`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    point: Option<Point>,
    /// Disk emitter.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    disk: Option<DiskShape>,
    /// Rectangle emitter `[left, top, right, bottom]`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    rect: Option<Rect>,
    /// Path emitter, as SVG path data.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    path: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    surface: Option<ParticleSurfaceSpec>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    image: Option<ParticleImageSourceSpec>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    text: Option<ParticleTextSourceSpec>,
    /// Burst emitter.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    burst: Option<BurstShape>,
    /// Particles per second.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    rate: Option<f64>,
    /// Seconds a particle lives.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    lifetime: Option<f64>,
    /// Launch speed.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    speed: Option<f64>,
    /// Added velocity `[x, y]`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    velocity: Option<[f64; 2]>,
    /// Birth color.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    color: Option<Color>,
    /// Birth size.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    size: Option<f64>,
    /// Depth range `[near, far]`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    depth_range: Option<[f64; 2]>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    path_progress: Option<Animated<f64>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    premultiplied_color: Option<[f32; 4]>,
}

impl TryFrom<RawEmitter> for EmitterSpec {
    type Error = String;

    fn try_from(raw: RawEmitter) -> Result<Self, String> {
        let shapes = [
            raw.point.map(|value| ("point", EmitterShape::Point(value))),
            raw.disk.map(|value| {
                (
                    "disk",
                    EmitterShape::Disk {
                        center: value.center,
                        radius: value.radius,
                    },
                )
            }),
            raw.rect.map(|value| ("rect", EmitterShape::Rect(value))),
            raw.path.map(|value| ("path", EmitterShape::Path(value))),
            raw.surface
                .map(|value| ("surface", EmitterShape::Surface(value))),
            raw.image.map(|value| ("image", EmitterShape::Image(value))),
            raw.text.map(|value| ("text", EmitterShape::Text(value))),
            raw.burst.map(|value| {
                (
                    "burst",
                    EmitterShape::Burst {
                        time: value.time,
                        count: value.count,
                        position: value.position,
                    },
                )
            }),
        ];
        let mut selected = shapes.into_iter().flatten();
        let shape = match (selected.next(), selected.next()) {
            (Some((_, shape)), None) => shape,
            (None, _) => {
                return Err("Emitter needs one shape: point, disk, rect, path, surface, image, text or burst".into());
            }
            (Some((first, _)), Some((second, _))) => {
                return Err(format!(
                    "Emitter has both `{first}` and `{second}`; choose one shape"
                ));
            }
        };
        Ok(Self {
            shape,
            rate: raw.rate,
            lifetime: raw.lifetime,
            speed: raw.speed,
            velocity: raw.velocity,
            color: raw.color,
            size: raw.size,
            depth_range: raw.depth_range,
            path_progress: raw.path_progress,
            premultiplied_color: raw.premultiplied_color,
        })
    }
}

impl From<EmitterSpec> for RawEmitter {
    fn from(spec: EmitterSpec) -> Self {
        let mut raw = Self {
            rate: spec.rate,
            lifetime: spec.lifetime,
            speed: spec.speed,
            velocity: spec.velocity,
            color: spec.color,
            size: spec.size,
            depth_range: spec.depth_range,
            path_progress: spec.path_progress,
            premultiplied_color: spec.premultiplied_color,
            ..Self::default()
        };
        match spec.shape {
            EmitterShape::Point(value) => raw.point = Some(value),
            EmitterShape::Disk { center, radius } => raw.disk = Some(DiskShape { center, radius }),
            EmitterShape::Rect(value) => raw.rect = Some(value),
            EmitterShape::Path(value) => raw.path = Some(value),
            EmitterShape::Surface(value) => raw.surface = Some(value),
            EmitterShape::Image(value) => raw.image = Some(value),
            EmitterShape::Text(value) => raw.text = Some(value),
            EmitterShape::Burst {
                time,
                count,
                position,
            } => {
                raw.burst = Some(BurstShape {
                    time,
                    count,
                    position,
                })
            }
        }
        raw
    }
}

/// A force or lifetime curve acting on every particle.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case", deny_unknown_fields)]
pub enum ModifierSpec {
    /// Constant acceleration `[x, y]`.
    Gravity([f64; 2]),
    /// Constant wind acceleration `[x, y]`.
    Wind([f64; 2]),
    /// Linear drag coefficient, nonnegative.
    Drag(f64),
    /// Swirl around a center.
    Vortex {
        /// Center `[x, y]`.
        center: Point,
        /// Signed strength.
        strength: f64,
    },
    /// Pull toward a center.
    Attractor {
        /// Center `[x, y]`.
        center: Point,
        /// Signed strength.
        strength: f64,
    },
    /// Divergence-free 2D noise flow.
    CurlNoise {
        /// Spatial frequency.
        frequency: f64,
        /// Acceleration scale.
        strength: f64,
        /// Noise seed in `0..=4294967295`; zero by default.
        #[serde(default)]
        seed: u64,
        /// Noise evolution speed; zero by default.
        #[serde(default)]
        phase_speed: f64,
    },
    /// Pull toward the nearest of several points.
    Seek {
        /// Target points `[[x, y], ...]`, 1 to 65536.
        targets: Vec<Point>,
        /// Nonnegative strength.
        strength: f64,
    },
    /// Reflect particles inside a box `[left, top, right, bottom]`.
    BounceBox(Rect),
    /// Color ramp over each particle's life, 1 to 256 stops.
    ColorOverLifetime(Vec<Color>),
    /// Size ramp over each particle's life.
    SizeOverLifetime {
        /// Size at birth.
        start: f64,
        /// Size at death.
        end: f64,
    },
    /// Nonnegative HDR energy over normalized age.
    IntensityOverLifetime(ParticleCurveSpec),
    /// Blackbody color over normalized age, in Kelvin.
    Temperature(ParticleCurveSpec),
    /// Population, collision or field operator.
    Operator(Box<ParticleOperatorSpec>),
}

/// How particles are drawn, written as `{"dots": {"radius": 2}}` and similar.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case", deny_unknown_fields)]
pub enum ParticleStyleSpec {
    /// Solid dots.
    Dots {
        /// Dot radius.
        radius: f64,
    },
    /// Soft glowing dots.
    GlowDots {
        /// Dot radius.
        radius: f64,
    },
    /// Motion trails.
    Trails {
        /// Trail length in seconds.
        length: f64,
        /// Trail width.
        width: f64,
    },
    /// Dots joined by lines when closer than `max_distance`.
    Plexus {
        /// Dot radius.
        radius: f64,
        /// Largest connected distance.
        max_distance: f64,
    },
    /// Procedural shapes.
    Shape {
        /// Shape radius.
        radius: f64,
        /// Shape kind.
        shape: ParticleShapeSpec,
    },
    /// Depth-aware aperture geometry.
    Bokeh {
        /// In-focus radius.
        radius: f64,
        /// Defocus scale.
        aperture: f32,
        /// Focus depth.
        focus: f32,
        /// Polygon blade count; a disc when omitted.
        blades: Option<u8>,
        /// Rim intensity.
        ring: f32,
        /// Edge hardness.
        hardness: f32,
        /// HDR energy multiplier.
        highlight: f32,
    },
    /// Budgeted immutable procedural atlas.
    Sprites {
        /// Explicit material seed.
        seed: u64,
        /// Generated frames, 1 to 64.
        frames: Vec<ParticleSpriteSpec>,
        /// Full local sprite side.
        size: f64,
        /// Pads differing frame sizes to uniform cells.
        padded: bool,
        /// Stateless frame selection.
        mode: ParticleSheetSpec,
    },
    /// Ordinary vector circles with an age-dependent radius.
    NodeCircles {
        /// Base radius in design units.
        radius: f64,
        /// Signed sinusoidal radius amplitude.
        amplitude: f64,
        /// Angular frequency in radians per second of particle age.
        frequency: f64,
        /// Fixed circle fill.
        fill: Color,
    },
}

/// Procedural particle shapes.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case", deny_unknown_fields)]
pub enum ParticleShapeSpec {
    /// Regular polygon with 3 to 32 sides.
    Polygon {
        /// Side count.
        sides: u8,
    },
    /// Ring with a thickness fraction in `(0, 1]`.
    Ring {
        /// Thickness fraction.
        thickness: f64,
    },
    /// Capsule with an aspect in `1..=64`.
    Capsule {
        /// Aspect ratio.
        aspect: f64,
    },
    /// Soft disc with a hardness in `0..=1`.
    Soft {
        /// Edge hardness.
        hardness: f64,
    },
    /// Quad with an aspect in `(0, 64]`.
    Quad {
        /// Aspect ratio.
        aspect: f64,
    },
}

/// A closed-form particle burst without a simulation step.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct TrajectorySpec {
    /// The motion model.
    pub model: TrajectoryModelSpec,
    /// Explicit sample time in seconds; scene-local time when omitted.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[schemars(extend("minimum" = 0.0, "maximum" = 1e6))]
    pub sample_time: Option<Animated<f64>>,
    /// How particles are drawn; dots by default.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub style: Option<ParticleStyleSpec>,
    /// Compositing of the particle layer.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub blend: Option<InstanceBlend>,
}

/// Analytic particle motion, written as `{"ballistic": {...}}` and similar.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case", deny_unknown_fields)]
pub enum TrajectoryModelSpec {
    /// Corresponding points easing from a source set to a target set.
    Converge {
        /// Source points, 1 to 65536.
        from: ParticlePointsSpec,
        /// Target points, with the same length as the source.
        to: ParticlePointsSpec,
        /// Travel duration in seconds.
        duration: f64,
    },
    /// Particles flung from a point under gravity and drag.
    Ballistic {
        /// Launch point `[x, y]`.
        center: Point,
        /// Number of particles.
        count: u32,
        /// Acceleration `[x, y]`.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        gravity: Option<[f64; 2]>,
        /// Drag coefficient.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        drag: Option<f64>,
        /// Speed range `[min, max]`.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        speed: Option<[f64; 2]>,
        /// Explicit seed; zero by default.
        #[serde(default)]
        seed: u64,
        /// Particle lifetime in seconds.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        lifetime: Option<f64>,
    },
    /// Particles circling a center.
    Orbit {
        /// Center `[x, y]`.
        center: Point,
        /// Orbit radius.
        radius: f64,
        /// Number of particles.
        count: u32,
        /// Radians per second.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        angular_velocity: Option<f64>,
        /// Explicit seed; zero by default.
        #[serde(default)]
        seed: u64,
        /// Particle lifetime in seconds.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        lifetime: Option<f64>,
    },
    /// Particles drifting through noise.
    NoiseDrift {
        /// Start point `[x, y]`.
        origin: Point,
        /// Number of particles.
        count: u32,
        /// Base velocity `[x, y]`.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        velocity: Option<[f64; 2]>,
        /// Noise `[amplitude, frequency]`.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        noise: Option<[f64; 2]>,
        /// Explicit seed; zero by default.
        #[serde(default)]
        seed: u64,
        /// Particle lifetime in seconds.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        lifetime: Option<f64>,
    },
}
