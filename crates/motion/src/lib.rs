//! Programmable motion graphics and deterministic video rendering in Rust.
//!
//! A [`Scene`] maps signed-tick [`SceneContext`] inputs to an immutable [`Node`].
//! [`Signal`] samples values without playback state; [`Timeline`] schedules
//! tracks/clips and transitions. Public coordinates/colors use [`Point`],
//! [`Rect`], [`Size`] and [`Color`] in logical authoring units.
//! [`Project`], [`render_still`] and [`export()`] share resource/backend/output policy.
//! Feature-selected `Image`, `Svg` and `Text` record into the same validated drawing pipeline.
//!
//! The rendering core has no desktop windows, widgets, or implicit playback clock.
//! [`Row`] and [`Column`] provide deterministic layout inside the scene tree.
//! The optional `studio` feature adds an egui desktop preview and export workbench.
//! Import [`prelude`] for canonical authoring APIs.
//!
//! # Model
//!
//! 1. Build a [`Scene`] from exact local/global ticks and a rational [`FrameRate`].
//! 2. Return [`Node`] content, compiled to a [`DisplayList`] and [`PreparedFrame`].
//! 3. Render a transparent [`FrameBuffer`], external texture or native encoder output.
//!
//! [`RenderOptions`] configures reusable [`RenderSession`] state; each
//! [`FrameRequest`] addresses a frame by index or exact time and supplies duration, destination, background and format.
//! [`FrameRenderer`] adds Composition's frame-index clock. [`ValidatedDisplayList`]
//! freezes checked commands without claiming device availability or raster admission.
//!
//! # Content and Composition
//!
//! [`Path2D`] and [`StrokeStyle`] provide vector commands; `ink::Ink` adds deterministic
//! directional dry gaps, soft spreading, rough edges, paper imprint and pigment
//! pooling through `ink::BrushStroke` (configured with `ink::BrushStrokeBuilder`) and
//! `ink::InkFill`; `ink::Paper` selects procedural or texture paper. Pressure and ink-load profiles
//! are sampled in full arc length.
//! [`Signal`] and [`Keyframes`] are analytic, clock-free samplers, not playback
//! tasks. [`FrameBuffer`] is the single stored image: `Composition::render_frame` yields it opaque over white, `render_still` preserves alpha.
//! [`AudioClip`] supplies explicit timeline PCM, mixed into export or Studio Play.
//! [`Skeleton`] solves 2D forward kinematics from timeline-compatible [`Pose`]
//! values; [`BoneTransform`] positions artwork without changing either backend.
//! [`NodeModifiers`] supplies isolated opacity, blending, geometric clips, content
//! masks and ordered [`Filter`] effects. [`Transition`] combines scenes in
//! an explicit viewport with independent local clocks.
//!
//! # Example
//!
//! ```no_run
//! use motion::prelude::*;
//! # fn main() -> motion::Result {
//! let timeline = Timeline::new(Size::new(640.0, 360.0), 30).track(|track| {
//!     track.push(
//!         scene(|context| {
//!             Node::from(
//!                 Rect::new(32.0, 64.0, 32.0 + context.seconds() * 80.0, 88.0)
//!                     .fill(Color::rgb(20, 150, 125)),
//!             )
//!         })
//!         .with_duration(3.seconds()),
//!     )
//! }).build()?;
//! render_still(&timeline, 1500.milliseconds().after_origin(), &RenderOptions::default())?
//!     .save_png("frame.png")?;
//! # Ok(())
//! # }
//! ```
//!
//! # Where things live
//!
//! The prelude covers authoring values and the everyday entry points (`render_still`,
//! `export`, [`RenderOptions`], [`ExportOptions`]). Everything else sits in an explicit
//! namespace, some behind a Cargo feature:
//!
//! | Need | Namespace | Feature |
//! | --- | --- | --- |
//! | Particles | [`particles`] | `particles` |
//! | Ink and brush strokes | `ink` | `ink` |
//! | Charts and diagrams | `diagram` | `diagram` |
//! | Captions and SRT | `captions` | `captions` |
//! | Typst math | `formula` | `formula` |
//! | JSON documents and schema | `declarative` | `declarative` |
//! | Vector trees and morphs | `vector` | `vector` |
//! | Extension protocols (`Drawable`, `DeferredContent`, `Transition`, `RenderPrepare`) | `extend` | always |
//! | Compiled commands, caches, backends | `render` | always |
//! | Encoders, progress sinks, export reports | `encode` | always |
//! | Desktop preview | `studio` | `studio` |
//!
//! # Features and Resources
//!
//! Default Cargo features include `gpu`, content, native FFmpeg and Studio;
//! library rendering selects CPU unless requested explicitly; the CLI defaults to Automatic.
//! Native GPU composition uses Metal, Vulkan or DX12. `BackendPreference::gpu()` selects an
//! available native backend, including Metal on macOS. `studio` adds an egui
//! workbench but does not open a window on import. Geometry-only rendering does
//! not discover fonts; text requires an explicit font environment. Native video export
//! and Studio's streamed file-audio mixing require feature `ffmpeg` and native FFmpeg
//! with `libx264`, plus AAC for encoded audio.
//! Building that feature requires development headers/libraries, pkg-config and
//! libclang for bindgen. Studio enables it automatically; no executable is invoked.
//!
//! Optional `typst` compiles trusted markup in-process using bundled fonts,
//! without a CLI or external resource loading. Its single-page SVG can be retained
//! or imported through `TypstDocument::node` before frame evaluation. Feature
//! `formula` provides math vectors and operations through FormulaCompiler.
//!
//! # Validation and Ownership
//!
//! [`Composition::preflight`] checks configuration; [`Node::compile`] validates
//! every content branch against [`ValidationLimits`].
//! [`Composition::preflight_frame`] additionally builds one frame and attaches its
//! index to scene diagnostics. Engine preflight does not discover fonts or initialize
//! devices, though explicit text bounds may require layout. It cannot establish live
//! backend availability or predict every raster allocation; custom callbacks are trusted.
//! [`ValidationError`] exposes the failing property, structural path and optional
//! frame index. Scene diagnostics returned through [`Result`] can be recovered with
//! `error.downcast_ref::<ValidationError>()`.
//! [`Result`] uses [`Error`]: inspect [`Error::kind`] and [`Error::frame`] without
//! parsing display text. Original causes remain in the standard source chain.
//! Errors are `Send + Sync`, so workers retain typed diagnostics until UI display.
//!
//! [`Assets`] owns both lazy typed sources and explicit preloads within an injected
//! [`ResourcePool`]. [`RenderContext`] and scene compilation select the same owner;
//! clones share resolved entries; mutable access isolates future registrations,
//! including previously requested but unresolved handles. Relative file lookup
//! requires an explicit project root. Separate projects need not share a pool.
//! Sessions keep their own caches/targets while font snapshots and injected GPUs
//! may be shared. [`RequestDiagnostics`] supports explicit cross-thread resource
//! failures. Compiled [`RenderCapabilities`] are not a live-device probe.
//!
//! [`SceneDescriptor`] combines a stable scene ID, composition and callback with optional
//! [`VisualCheckPolicy`] metadata. Descriptor preflight does not execute the callback;
//! it is separate from validating the scene that a callback produces.
//!
//! Rendering and I/O entry points return [`Result`]. Validation timing is API-specific:
//! some builders defer checks while convenience animation/value methods may panic;
//! their checked variants return diagnostics for dynamic input. Core callbacks run
//! synchronously and their panics propagate; Studio workers catch scene panics.
//!
//! # Import Boundaries
//!
//! Renderer internals belong to [`render`], encoders and progress sinks to [`encode`], and
//! implementation protocols to [`extend`].
//!
//! ```compile_fail
//! use motion::prelude::*;
//! fn renderer_internal(_: Composition) {}
//! ```
//!
//! ```compile_fail
//! use motion::prelude::*;
//! fn command_internal(_: DisplayList) {}
//! ```
//!
//! ```compile_fail
//! use motion::prelude::*;
//! fn extension_protocol(_: &dyn Drawable) {}
//! ```
//!
//! ```compile_fail
//! use motion::prelude::*;
//! fn particle_protocol(_: &dyn Emitter) {}
//! ```
//!
//! ```compile_fail
//! use motion::prelude::*;
//! fn encoder_protocol(_: &dyn Encoder) {}
//! ```
//!
//! ```compile_fail
//! use motion::{Composition, DisplayList, Drawable};
//! ```
//!
//! [`DisplayList`]: render::DisplayList
//! [`PreparedFrame`]: render::PreparedFrame
//! [`FrameBuffer`]: render::FrameBuffer
//! [`RenderSession`]: render::RenderSession
//! [`FrameRequest`]: render::FrameRequest
//! [`FrameRenderer`]: render::FrameRenderer
//! [`ValidatedDisplayList`]: render::ValidatedDisplayList
//! [`Composition::preflight`]: render::Composition::preflight
//! [`Composition::preflight_frame`]: render::Composition::preflight_frame
//! [`ValidationLimits`]: render::ValidationLimits
//! [`ResourcePool`]: render::ResourcePool
//! [`RenderContext`]: render::RenderContext
//! [`RequestDiagnostics`]: render::RequestDiagnostics
//! [`RenderCapabilities`]: render::RenderCapabilities
//! [`Transition`]: extend::Transition
//!
#![forbid(unsafe_code)]

mod catalog;
mod project;

#[cfg(feature = "ink")]
pub use motion_content::vector::ink;

pub use motion_core::{
    ApertureShape, Cdl, ColorWheels, DepthOfField, DepthPlane, Elevation, HeightFrom, Lut1d, Lut3d,
    LutInput, Optics, ShutterCurve, ShutterSampling,
};
pub use motion_encode::BitDepth;
pub use motion_render::look;
pub use motion_render::look::{GateWeave, Look, SceneEffects};
pub use motion_render::{DisplayTransform, FilmGrain, OutputTransfer, ToneMap};

pub use motion_core::ClipMode;
pub use motion_core::IntoSceneOutput;
pub use motion_core::{
    AudioBus, AudioChapter, AudioEffect, AudioMetadata, Limiter, Loudness, Master, PanLaw,
};
pub use motion_core::{
    Extrapolate, FollowPath, Noise, NoiseBuilder, Oscillator, OscillatorBuilder, PathSample,
    SpringBuilder, Waveform, piecewise, remap, try_piecewise, try_remap,
};
pub use motion_core::{camera, group, scene};
pub use motion_core::{key, math};

pub use motion_core::{
    Alignment, AngleLiteral, ArcShape, Arrow, ArrowHead, Assets, AudioClip, BlendMode, BlendSpace,
    Bone, BonePose, BoneTransform, Camera, Circle, Clip, Color, Column, Constant, CrossFade,
    Direction, Duration, Easing, Ellipse, Error, ErrorKind, FillRule, Filter, FiniteAnimation,
    FontEnvironment, FrameRate, FunctionGraph, GradientExtend, GradientSpace, GradientUnits,
    ImagePattern, Interpolate, Interval, Iris, IrisShape, Keyframes, Layout, Line, LineCap,
    LineJoin, LinearGradient, MaskMode, MotionBlur, Node, NodeModifiers, Paint, Path2D,
    PathBuilder, Pivot, PixelSize, Point, Polygon, Polyline, Pose, Push, RadialGradient, Random,
    Rect, RegularPolygon, RenderQuality, Ring, RoundedRect, Row, Sampling, Scene, SceneContext,
    Shape, Side, Signal, Size, Skeleton, Slide, Soundtrack, Source, Spline, Spring, Star,
    StrokeStyle, Styled, SweepGradient, Time, TimeLiteral, Timeline, TimelineBuilder,
    TimelineWarning, Track, TrackBuilder, Transform, TransformComponents, Tween, ValidationError,
    Vector2, Wipe, ZoomBlur, animation, authoring, color, geometry, noise, path2d, sample_offsets,
    shape, time, try_camera,
};
pub use motion_render::{BackendPreference, RenderOptions, render_still};

pub use motion_encode::audio;
pub use motion_encode::{Codec, EncoderPreset, ExportOptions, export};

#[cfg(feature = "particles")]
pub use motion_content::particles;
#[cfg(feature = "image")]
pub use motion_content::{AnimatedImage, ImageLoopMode};
#[cfg(feature = "vector")]
pub use motion_content::{AssetsExt, Fit, Image, ImageAsset};
#[cfg(feature = "text")]
pub use motion_content::{
    FontFace, FontLibrary, FontWeight, ParagraphStyle, PartStyle, Text, TextAlignment, TextPart,
    TextSpan, TextSplit,
};
#[cfg(feature = "svg")]
pub use motion_content::{Svg, svg};
#[cfg(feature = "text")]
pub use motion_content::{asset, font, text};
#[cfg(feature = "vector")]
pub use motion_content::{atmosphere, image, load_lut, load_shaper, vector};

#[cfg(feature = "formula")]
pub use motion_formula as formula;
#[cfg(feature = "formula")]
pub use motion_formula::{Formula, FormulaOptions};
#[cfg(feature = "typst")]
pub use motion_formula::{TypstCompiler, TypstDocument};
#[cfg(feature = "declarative")]
pub use motion_scene as declarative;
#[cfg(feature = "studio")]
pub use motion_studio as studio;

pub use crate::catalog::{SceneDescriptor, VisualCheckPolicy, VisualRegion};
pub use crate::project::{
    Backend, ExportArgs, OutputArgs, ParameterArgs, Project, ProjectBuilder, ProjectCommand,
    ProjectOutcome, ProjectTimelineInfo, RenderArgs, StillArgs, StudioArgs, ValidateArgs,
};

#[cfg(feature = "captions")]
pub use motion_content::captions;
#[cfg(feature = "diagram")]
pub use motion_content::diagram;

/// Shared result with an optional caller-selected error type.
pub type Result<T = (), E = Error> = std::result::Result<T, E>;

/// Encoding implementations and low-level audio/export contracts.
pub mod encode {
    #[cfg(feature = "audio")]
    pub use motion_encode::{AudioAnalysis, AudioSignal};
    pub use motion_encode::{
        Codec, Encoder, EncoderCapabilities, EncoderFactory, EncoderPreset, ExportOptions,
        ExportPhase, ExportReport, FfmpegEncoder, GifEncoder, MixOptions, MixSource, MixedAudio,
        NoProgress, PcmBuffer, PngSequenceEncoder, ProgressSink, export,
    };
}

/// Protocols for custom drawing, assets, particles, transitions and encoders.
pub mod extend {
    #[cfg(feature = "particles")]
    pub use motion_content::particles::ParticleGroupModifier;
    #[cfg(feature = "particles")]
    pub use motion_content::{ParticleEmitter, ParticleModifier, TrajectoryModel};
    pub use motion_core::instances::{
        ExtendedInstanceRecord, InstanceBatch, InstanceBlend, InstanceComposite, InstanceKind,
        InstanceRecord, InstanceUpload, MAX_INSTANCE_COUNT,
    };
    pub use motion_core::{
        Asset, AudioSource, Canvas, DeferredContent, DrawContext, Drawable, Evaluation,
        ImagePixels, LoadContext, Transition, ValidationError, ValidationLimits,
    };
    pub use motion_encode::{Encoder, EncoderCapabilities, ProgressSink};
    pub use motion_render::RenderPrepare;
}

/// Explicit rendering, compiled commands and backend-level contracts.
pub mod render {
    pub use motion_core::{
        AssetStatus, AudioClipParameters, BoundaryAdjustment, BoundaryEdge, BoxedError,
        CacheLimits, DisplayList, DrawCommand, FilterOperation, FontIdentity, Handle,
        IntoShapeClip, MAX_PATH_VERBS, MAX_STROKE_VERBS, MAX_SVG_PATH_BYTES, MorphPath, PathClip,
        PathVerb, RasterLimits, RectClip, RequestDiagnostics, ResourcePool, SceneCache,
        SceneLimits, SceneStats, ShapeClip, TICKS_PER_SECOND, ValidatedDisplayList,
        ValidationErrorKind, ValidationLimits,
    };
    pub use motion_core::{
        budget, drawing, instances, linear_to_srgb, premultiply, rec709_luminance, srgb_to_linear,
        unpremultiply, validation,
    };
    pub use motion_render::*;
}

/// Immediately evaluated content operations for explicit preparation boundaries.
pub mod eager {
    #[cfg(feature = "vector")]
    pub use motion_content::{CompileCache, Morph, VectorSelection, VectorTree, WriteOptions};
    #[cfg(feature = "text")]
    pub use motion_content::{ParagraphLayout, TextGlyph};
    pub use motion_core::MorphPath;
    #[cfg(feature = "formula")]
    pub use motion_formula::{FormulaAsset, FormulaCompiler, FormulaSelection};
}

/// Authoring APIs. Extension protocols and renderer internals use explicit namespaces, and
/// particles, ink, diagrams, captions and formulas are imported from their own modules
/// (see the table in the crate documentation). Transition is in scope for its methods only.
pub mod prelude {
    pub use crate::{
        Alignment, AngleLiteral, ArcShape, Arrow, ArrowHead, Assets, AudioClip, BackendPreference,
        BlendMode, BlendSpace, Bone, BonePose, BoneTransform, Camera, Circle, Clip, ClipMode,
        Color, Column, Constant, CrossFade, Direction, Duration, Easing, Ellipse, Error, ErrorKind,
        Extrapolate, FillRule, Filter, FiniteAnimation, FollowPath, FontEnvironment, FrameRate,
        FunctionGraph, GradientExtend, GradientSpace, GradientUnits, ImagePattern, Interpolate,
        Interval, Iris, IrisShape, Keyframes, Layout, Line, LineCap, LineJoin, LinearGradient,
        MaskMode, MotionBlur, Node, NodeModifiers, Noise, Oscillator, Paint, Path2D, PathSample,
        Pivot, PixelSize, Point, Polygon, Polyline, Pose, Project, ProjectBuilder, ProjectOutcome,
        ProjectTimelineInfo, Push, RadialGradient, Random, Rect, RegularPolygon, RenderOptions,
        RenderQuality, Ring, RoundedRect, Row, Sampling, Scene, SceneContext, Shape, Side, Signal,
        Size, Skeleton, Slide, Source, Spline, Spring, Star, StrokeStyle, Styled, SweepGradient,
        Time, TimeLiteral, Timeline, TimelineBuilder, TimelineWarning, Track, TrackBuilder,
        Transform, TransformComponents, Tween, ValidationError, Vector2, Waveform, Wipe, ZoomBlur,
        camera, export, group, key, math, piecewise, remap, render_still, scene,
    };
    pub use crate::{
        ApertureShape, BitDepth, Cdl, ColorWheels, DepthOfField, DepthPlane, DisplayTransform,
        Elevation, FilmGrain, GateWeave, HeightFrom, Look, Lut1d, Lut3d, LutInput, Optics,
        OutputTransfer, SceneEffects, ShutterCurve, ShutterSampling, ToneMap,
    };
    pub use crate::{
        AudioBus, AudioChapter, AudioEffect, AudioMetadata, Limiter, Loudness, Master, PanLaw,
    };
    #[cfg(feature = "svg")]
    pub use motion_content::Svg;
    #[cfg(feature = "image")]
    pub use motion_content::{AnimatedImage, ImageLoopMode};
    #[cfg(feature = "vector")]
    pub use motion_content::{AssetsExt, Fit, Image, ImageAsset};
    #[cfg(feature = "text")]
    pub use motion_content::{
        FontFace, FontLibrary, FontWeight, ParagraphStyle, PartStyle, Text, TextAlignment,
        TextPart, TextSpan, TextSplit,
    };
    pub use motion_core::Transition as _;
    pub use motion_encode::{Codec, EncoderPreset, ExportOptions};
    #[cfg(feature = "formula")]
    pub use motion_formula::{Formula, FormulaOptions};
    #[cfg(feature = "typst")]
    pub use motion_formula::{TypstCompiler, TypstDocument};
}

/// Compiles the Rust blocks in the tutorial and reference pages so renamed APIs break the build.
#[cfg(doctest)]
mod tutorial {
    #[doc = include_str!("../../../docs/design/rendering.md")]
    struct DesignRendering;

    #[cfg(feature = "text")]
    #[doc = include_str!("../../../docs/design/resources.md")]
    struct DesignResources;

    #[doc = include_str!("../../../docs/cli.md")]
    struct CliReference;

    #[cfg(feature = "particles")]
    #[doc = include_str!("../../../docs/tutorial/00-overview.md")]
    struct Chapter00;

    #[doc = include_str!("../../../docs/tutorial/01-quick-start.md")]
    struct Chapter01;

    #[doc = include_str!("../../../docs/tutorial/02-foundations.md")]
    struct Chapter02;

    #[doc = include_str!("../../../docs/tutorial/03-animation.md")]
    struct Chapter03;

    #[doc = include_str!("../../../docs/tutorial/04-scenes-and-timeline.md")]
    struct Chapter04;

    #[cfg(feature = "ink")]
    #[doc = include_str!("../../../docs/tutorial/05-drawing.md")]
    struct Chapter05;

    #[cfg(feature = "text")]
    #[doc = include_str!("../../../docs/tutorial/06-content.md")]
    struct Chapter06;

    #[cfg(feature = "particles")]
    #[doc = include_str!("../../../docs/tutorial/07-particles.md")]
    struct Chapter07;

    #[cfg(feature = "formula")]
    #[doc = include_str!("../../../docs/tutorial/08-formula-typst.md")]
    struct Chapter08;

    #[doc = include_str!("../../../docs/tutorial/09-rendering.md")]
    struct Chapter09;

    #[cfg(feature = "audio")]
    #[doc = include_str!("../../../docs/tutorial/10-audio.md")]
    struct Chapter10;

    #[cfg(feature = "ffmpeg")]
    #[doc = include_str!("../../../docs/tutorial/11-export.md")]
    struct Chapter11;

    #[cfg(feature = "studio")]
    #[doc = include_str!("../../../docs/tutorial/12-studio.md")]
    struct Chapter12;

    #[doc = include_str!("../../../docs/tutorial/13-performance-and-reliability.md")]
    struct Chapter13;

    #[doc = include_str!("../../../docs/tutorial/14-architecture-and-extending.md")]
    struct Chapter14;

    #[cfg(all(feature = "particles", feature = "ffmpeg"))]
    #[doc = include_str!("../../../docs/tutorial/15-cinematic.md")]
    struct Chapter15;

    #[cfg(feature = "declarative")]
    #[doc = include_str!("../../../docs/tutorial/16-declarative.md")]
    struct Chapter16;

    #[doc = include_str!("../../../docs/tutorial/appendix-gallery.md")]
    struct AppendixGallery;
}
