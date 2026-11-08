//! Built-in and custom particle presentation styles.

use super::*;
use motion_core::{Node, SceneContext};

type CustomStyle = Arc<dyn Fn(&[Particle], &SceneContext<'_>) -> Node + Send + Sync>;

/// particle presentation. Built-ins share an instance protocol; custom
/// callbacks run on CPU and return ordinary content, never custom WGSL.
#[derive(Clone)]
pub enum ParticleStyle {
    /// Smooth analytic circles; radius multiplies particle.size.
    Dots {
        /// Base local radius.
        radius: f64,
    },
    /// Compact Gaussian halos with linear Plus compositing.
    GlowDots {
        /// Gaussian scale (support extends to three radii).
        radius: f64,
    },
    /// Bilinear sprite instances, tinted by premultiplied particle color.
    Sprites {
        /// Lazy immutable texture.
        image: motion_core::Handle<crate::ImageAsset>,
        /// Full local sprite side.
        size: f64,
        /// Adds velocity direction to particle.rotation.
        align_to_velocity: bool,
    },
    /// Eight actual historical segments per particle, not a velocity extrapolation.
    Trails {
        /// Historical interval.
        length: Duration,
        /// Local line width.
        width: f64,
    },
    /// Analytic dots and bounded pairwise neighbor connections.
    Plexus {
        /// Dot radius.
        radius: f64,
        /// Maximum local neighbor separation.
        max_distance: f64,
    },
    /// Texture-free analytic geometry.
    Shape {
        /// Base radius or half height.
        radius: f64,
        /// Coverage geometry.
        shape: ParticleShape,
    },
    /// Depth-aware, energy-compensated aperture. Depth is `particle.custom[1]`.
    Bokeh {
        /// Base in-focus radius.
        radius: f64,
        /// Circle-of-confusion scale.
        aperture: f32,
        /// In-focus depth.
        focus: f32,
        /// Aperture silhouette.
        shape: BokehShape,
        /// Rim intensity in 0..=1.
        ring: f32,
        /// Edge hardness in 0..=1.
        hardness: f32,
        /// Finite nonnegative HDR multiplier.
        highlight: f32,
    },
    /// A built-in or custom style with explicit batch options; see [`StyleOptions`].
    Configured {
        /// Underlying built-in or custom style.
        style: Box<ParticleStyle>,
        /// Batch-level presentation policy.
        options: StyleOptions,
    },
    /// CPU-only presentation callback.
    Custom(CustomStyle),
}

/// Explicit batch compositing, independent of primitive kind.
pub type ParticleBlend = motion_core::instances::InstanceBlend;

/// Built-in analytic particle silhouettes.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum ParticleShape {
    /// Regular polygon with 3..=32 sides.
    Polygon {
        /// Number of regular sides, in 3..=32.
        sides: u8,
    },
    /// Annulus with relative thickness in (0,1].
    Ring {
        /// Relative annulus width in (0,1].
        thickness: f32,
    },
    /// Rounded velocity-oriented capsule, aspect in 1..=64.
    Capsule {
        /// Length-to-radius ratio in 1..=64.
        aspect: f32,
    },
    /// Soft-edged circle, hardness in 0..=1.
    Soft {
        /// Edge hardness in 0..=1.
        hardness: f32,
    },
    /// Rotated rectangle, aspect in (0,64].
    Quad {
        /// Horizontal-to-vertical half-extent ratio in (0,64].
        aspect: f32,
    },
}

/// Aperture geometry for depth-aware particles.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum BokehShape {
    /// Circular aperture.
    Disc,
    /// Regular aperture with 3..=32 blades.
    Polygon(u8),
}

/// Deterministic velocity-aligned elongation.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct SpeedStretch {
    /// Additional aspect per unit speed.
    pub scale: f32,
    /// Aspect ceiling in 1..=64.
    pub maximum: f32,
}

/// Historical sampling shared by all particles at each requested boundary.
#[derive(Clone)]
pub struct TrailOptions {
    /// Number of historical intervals, in 1..=64; the default is eight.
    pub segments: u8,
    /// Width along the historical interval; None preserves constant width.
    pub taper: Option<ParticleCurve>,
    /// Premultiplied fade along the interval.
    pub fade: Option<ParticleCurve>,
    /// Catmull-Rom interpolation with bounded adaptive subdivision.
    pub smooth: bool,
    /// Flat-ended continuous strip instead of rounded segments; with `smooth`, bends
    /// are subdivided up to eight times per interval to close join gaps.
    pub ribbon: bool,
}

impl Default for TrailOptions {
    fn default() -> Self {
        Self {
            segments: 8,
            taper: None,
            fade: None,
            smooth: false,
            ribbon: false,
        }
    }
}

/// Optional policies grouped separately from each style's basic geometry.
#[derive(Clone, Default)]
pub struct StyleOptions {
    /// Batch compositing; None selects the style's natural rule: additive Plus for
    /// `GlowDots` and source-over otherwise.
    pub blend: Option<ParticleBlend>,
    /// Optional flipbook or atlas cell selection.
    pub sheet: Option<SpriteSheet>,
    /// Historical geometry policy.
    pub trails: TrailOptions,
    /// Optional velocity stretch.
    pub stretch: Option<SpeedStretch>,
    /// Local displacement per unit depth relative to focus.
    pub parallax: [f32; 2],
    /// Enables stable far-to-near sorting by (depth,index).
    pub sort_depth: bool,
    /// Animates Quad aspect and brightness using particle age and phase `custom[2]`.
    pub flip: bool,
}

impl ParticleStyle {
    fn configure(self, update: impl FnOnce(&mut StyleOptions)) -> Self {
        let (style, mut options) = match self {
            Self::Configured { style, options } => (style, options),
            style => (Box::new(style), StyleOptions::default()),
        };
        update(&mut options);
        Self::Configured { style, options }
    }

    /// Selects compositing for the entire batch, not individual GPU invocations.
    pub fn blend(self, blend: ParticleBlend) -> Self {
        self.configure(|options| options.blend = Some(blend))
    }

    /// Adds animation or atlas UVs to Sprites.
    pub fn sheet(self, sheet: SpriteSheet) -> Self {
        self.configure(|options| options.sheet = Some(sheet))
    }

    /// Configures segment count, endpoint width/color and optional smoothing/ribbon.
    pub fn trails(self, trails: TrailOptions) -> Self {
        self.configure(|options| options.trails = trails)
    }

    /// Aligns and stretches geometry along the velocity, with a finite ceiling.
    pub fn stretch_by_speed(self, scale: f32, maximum: f32) -> Self {
        self.configure(|options| options.stretch = Some(SpeedStretch { scale, maximum }))
    }

    /// Adds deterministic depth parallax and stable depth ordering.
    pub fn parallax(self, offset: [f32; 2]) -> Self {
        self.configure(|options| {
            options.parallax = offset;
            options.sort_depth = true;
        })
    }

    /// Enables the analytic single-axis confetti flip.
    pub fn flip(self) -> Self {
        self.configure(|options| options.flip = true)
    }
}

impl Default for ParticleStyle {
    fn default() -> Self {
        Self::Dots { radius: 2.0 }
    }
}
