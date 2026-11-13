//! Declarative, versioned JSON documents that describe complete motion films.
//!
//! A [`Document`] is plain data: canvas size, frame rate, tracks of timed
//! [`SceneSpec`]s whose [`Element`] trees draw geometry, text, images and SVG,
//! transitions, audio and an optional film [`LookSpec`]. Any scalar, point or
//! color property may be [`Animated`] with tweens, keyframes, oscillators or
//! noise evaluated in scene-local seconds. Documents round-trip through JSON
//! ([`Document::from_json`], [`Document::to_json`]) and can be built in Rust with
//! the same types. [`Document::compile`] validates every property, reporting
//! [`ValidationError`](motion_core::ValidationError)s at JSON paths, and returns
//! an engine [`Timeline`](motion_core::Timeline) plus the look's display stage.
//! [`schema`] produces a JSON Schema for editor completion and validation.
//!
//! # Format
//!
//! ```json
//! {
//!   "version": 1,
//!   "size": [640, 360],
//!   "fps": 30,
//!   "background": "#0c1311",
//!   "tracks": [{
//!     "items": [
//!       {"scene": {"duration": 2, "content": {
//!         "circle": {"center": [320, 180], "radius": 40},
//!         "fill": "#14a082",
//!         "translate": {"tween": {"from": [-200, 0], "to": [200, 0], "duration": 2,
//!                                 "easing": "cubic_in_out"}}
//!       }}},
//!       {"transition": {"effect": "cross_fade", "duration": 0.5}},
//!       {"scene": {"duration": 2, "content": {"children": []}}}
//!     ]
//!   }]
//! }
//! ```
//!
//! Units are seconds, clockwise degrees, design units, `[x, y]` points and
//! `[left, top, right, bottom]` rectangles. Relative resource paths resolve
//! against [`CompileOptions::base_dir`]; [`load`] uses the document's directory.
//! Text, images and SVG require the `text`, `image` and `svg` features; without
//! them such documents still parse and serialize but fail to compile.
//!
//! ```
//! use motion_scene::Document;
//! let document = Document::from_json(
//!     r#"{"version": 1, "size": [64, 64], "fps": 24, "tracks": [{"items": [
//!         {"scene": {"duration": 1, "content": {
//!             "rect": {"bounds": [8, 8, 56, 56]},
//!             "fill": {"keyframes": {"keys": [
//!                 {"time": 0, "value": "black"},
//!                 {"time": 1, "value": "white"}]}}}}}]}]}"#,
//! )?;
//! let compiled = document.compile(&Default::default())?;
//! assert_eq!(compiled.timeline().frame_count(), 24);
//! # Ok::<(), motion_core::Error>(())
//! ```

#![forbid(unsafe_code)]

mod animation;
mod audio;
mod cache;
mod captions;
mod compile;
mod content;
mod definitions;
mod diagnostics;
mod diagram;
mod document;
mod effects;
mod element;
mod formula;
mod paint;
mod params;
mod particles;
mod value;

pub use animation::{ClockSpec, FollowPathSpec, MapSpec, PathComponent, SpringSpec};
pub use audio::{
    AudioSpec, BusSpec, DuckSpec, DuckTriggerSpec, EffectSpec, EqKindSpec, LimiterSpec,
    LoudnessSpec, MasterSpec, PanLawSpec, SpatialSpec,
};
pub use cache::CompileCache;
pub use captions::{CaptionsSpec, CueSpec, KeywordSpec};
pub use compile::{CompileOptions, CompiledDocument, load, load_with};
pub use content::{
    ArcSpec, ArrowSpec, CircleSpec, Content, EllipseSpec, ImageSpec, LineSpec, NumberTextSpec,
    PartsSpec, PathSpec, PointsSpec, RectSpec, RegularPolygonSpec, RingSpec, SampledPointsSpec,
    SplineSpec, StarSpec, SvgSourceSpec, SvgSpec, TextSpanSpec, TextSpec,
};
pub use diagram::{
    CalloutSpec, LineChartSpec, OrientationSpec, RulerSpec, ScaleKind, ScaleSpec, SeriesSpec,
};
pub use document::{
    Document, FrameRateSpec, LookSpec, SceneSpec, TrackItem, TrackSpec, TransitionEffect,
    TransitionSpec,
};
pub use effects::{
    ApertureSpec, BloomSpec, CdlSpec, ClipShapeSpec, ClipSpec, ColorAdjustSpec, FilterSpec,
    HeightSourceSpec, MaskSpec, OpticsSpec,
};
pub use element::Element;
pub use formula::{FormulaSpec, MorphSpec, SvgMorphSpec};
pub use paint::{
    GradientOptions, GradientSpec, GradientStop, LinearGradientSpec, PaintSpec, RadialGradientSpec,
    StrokeSpec, SweepGradientSpec, TrimSpec,
};
pub use particles::{
    BurstShape, DiskShape, EmitterShape, EmitterSpec, ModifierSpec, ParticleConfigurationSpec,
    ParticleCurveSpec, ParticleFalloffSpec, ParticleImageSourceSpec, ParticleImageSpec,
    ParticleMaterialSpec, ParticleOperatorSpec, ParticlePointsSpec, ParticlePresetSpec,
    ParticleShapeSpec, ParticleSheetSpec, ParticleSpriteSpec, ParticleStyleSpec,
    ParticleSubEmitterSpec, ParticleSurfaceSpec, ParticleTextSourceSpec, ParticleTrailSpec,
    ParticleVectorPathSpec, ParticlesSpec, TrajectoryModelSpec, TrajectorySpec,
};
pub use value::{
    Animated, EasingSpec, ExprSpec, KeyframeSpec, KeyframesSpec, MotionSpec, NoiseSpec,
    OscillateSpec, TweenSpec,
};

/// The document format version written by this crate and accepted on input.
pub const FORMAT_VERSION: u32 = 1;

/// Largest accepted document size in bytes.
pub const MAX_DOCUMENT_BYTES: usize = 16 * 1024 * 1024;

/// Returns the JSON Schema (draft 2020-12) describing [`Document`].
///
/// Reference it from a document's `$schema` key for editor completion. The schema
/// checks structure, types and selected static ranges; complete range and timing
/// validation remains in [`Document::compile`]. Value positions also accept `$calc`, `$param` and `use` references,
/// resolved by [`Document::from_json`] before type checking.
///
/// ```
/// let schema = motion_scene::schema();
/// assert_eq!(schema["title"], "Document");
/// assert!(schema["$defs"]["Element"].is_object());
/// ```
pub fn schema() -> serde_json::Value {
    let mut schema = serde_json::Value::from(schemars::schema_for!(Document));
    params::relax_schema(&mut schema);
    schema
}
