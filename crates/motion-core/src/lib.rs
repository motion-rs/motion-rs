//! Backend-independent motion engine primitives.
//!
//! This package owns time, geometry, animation, colors and diagnostics without
//! rendering devices, font engines, media codecs or window-system dependencies.
//!
//! ```
//! use motion_core::{FrameRate, Time, TimeLiteral};
//! let boundary = Time::from_frame(30_000, FrameRate::NTSC);
//! assert_eq!(boundary, 1001.seconds());
//! assert_eq!(boundary.to_frame(FrameRate::NTSC), 30_000);
//! ```

#![forbid(unsafe_code)]

pub mod animation;
pub mod asset;
pub mod audio;
pub mod authoring;
pub mod budget;
pub mod color;
pub mod deferred;
pub mod display_list;
pub mod drawing;
pub mod error;
pub mod filter;
pub mod geometry;
pub mod grading;
pub mod instances;
pub mod key;
pub mod math;
pub mod noise;
pub mod optics;
pub mod path2d;
pub mod pixels;
pub mod random;
pub mod shape;
pub mod time;
pub mod validation;

pub use animation::{Animation, Constant, Easing, Interpolate, Keyframes, Spring, Tween};
pub use animation::{
    Extrapolate, FollowPath, Noise, Oscillator, PathSample, Waveform, piecewise, remap,
};
pub use asset::environment::FontEnvironment;
pub use asset::environment::{FontIdentity, FontUsage};
pub use asset::{
    Asset, AssetStatus, Assets, Handle, LoadContext, RequestDiagnostics, ResourcePool, Source,
};
pub use audio::{
    AudioBus, AudioChapter, AudioClip, AudioClipParameters, AudioEffect, AudioMetadata,
    AudioSource, Duck, DuckTrigger, Ducking, Limiter, Loudness, Master, PanLaw,
};
pub use authoring::camera::camera;
pub use authoring::camera::{Camera, DepthOfField, DepthPlane, try_camera};
pub use authoring::context::{CacheLimits, CachePartition, RenderCache, SceneCache, SceneContext};
pub use authoring::elevation::{Elevation, ShadowLayer};
pub use authoring::sampling::{MotionBlur, ShutterCurve, ShutterSampling, sample_offsets};
pub use authoring::scene::IntoSceneOutput;
pub use authoring::scene::scene;
pub use authoring::skeleton::{Bone, BonePose, BoneTransform, Pose, Skeleton};
pub use authoring::timeline::{
    BoundaryAdjustment, BoundaryEdge, Clip, Diagnostics, ResolvedAudio, Soundtrack, Timeline,
    TimelineBuilder, TimelineVisual, TimelineWarning, Track, TrackBuilder,
};
pub use authoring::transition::Direction;
pub use authoring::transition::{
    CrossFade, Iris, IrisShape, Push, Slide, Transition, Wipe, ZoomBlur,
};
pub use authoring::{Alignment, Column, Layout, Node, NodeModifiers, Row, Scene, Side};
pub use budget::RasterLimits;
pub use color::Color;
pub use color::math::{
    linear_to_srgb, premultiply, rec709_luminance, srgb_to_linear, unpremultiply,
};
pub use display_list::{DisplayList, PixelSize, ValidatedDisplayList};
pub use drawing::{
    BlendMode, BlendSpace, Canvas, ClipMode, DrawCommand, DrawContext, Drawable, FillRule, LineCap,
    LineJoin, MAX_STROKE_VERBS, MaskMode, Paint, PathClip, RectClip, RenderQuality, Shape,
    ShapeClip, StrokeStyle, Styled,
};
pub use drawing::{
    GradientExtend, GradientSpace, GradientUnits, ImagePattern, LinearGradient, RadialGradient,
    SweepGradient,
};
pub use error::{BoxedError, Error, ErrorKind};
pub use filter::{Filter, FilterOperation};
pub use geometry::bounds::{Pivot, Rect, Size};
pub use geometry::{AngleLiteral, Point, Transform, TransformComponents, Vector2};
pub use grading::{Cdl, ColorWheels, Lut1d, Lut3d, LutInput};
pub use optics::{ApertureShape, HeightFrom, Optics};
pub use path2d::{MAX_PATH_VERBS, MAX_SVG_PATH_BYTES, MorphPath, Path2D, PathVerb};
pub use pixels::{ImagePixels, Sampling};
pub use random::Random;
pub use shape::{
    Arc as ArcShape, Arrow, ArrowHead, Circle, Ellipse, FunctionGraph, Line, Polygon, Polyline,
    RegularPolygon, Ring, RoundedRect, Spline, Star,
};
pub use time::{Duration, FrameRate, TICKS_PER_SECOND, Time, TimeLiteral};
pub use validation::{SceneStats, ValidationError, ValidationErrorKind, ValidationLimits};

/// A classified failure retaining its original source.
pub type Result<T = ()> = std::result::Result<T, Error>;

/// Common backend-independent authoring values.
pub mod prelude {
    pub use crate::ClipMode;
    pub use crate::{Alignment, Column, Layout, Node, NodeModifiers, Row, Scene, Side};
    pub use crate::{
        AngleLiteral, Color, Pivot, Point, Rect, Size, Transform, TransformComponents, Vector2,
    };
    pub use crate::{Animation, Constant, Easing, Interpolate, Keyframes, Spring, Tween};
    pub use crate::{
        ApertureShape, Cdl, ColorWheels, DepthOfField, DepthPlane, Elevation, HeightFrom, Lut1d,
        Lut3d, LutInput, Optics, ShutterCurve, ShutterSampling,
    };
    pub use crate::{
        ArcShape, Arrow, ArrowHead, Circle, Ellipse, FunctionGraph, Line, Polygon, Polyline,
        RegularPolygon, Ring, RoundedRect, Spline, Star,
    };
    pub use crate::{
        Asset, AssetStatus, Assets, AudioClip, AudioSource, Handle, LoadContext,
        RequestDiagnostics, Source,
    };
    pub use crate::{BlendMode, BlendSpace, Filter, FilterOperation, MaskMode, PixelSize};
    pub use crate::{
        Bone, BonePose, BoneTransform, Camera, Direction, Pose, SceneCache, SceneContext, Skeleton,
        camera, try_camera,
    };
    pub use crate::{
        BoundaryAdjustment, BoundaryEdge, Clip, Soundtrack, Timeline, TimelineBuilder,
        TimelineWarning, Track, TrackBuilder,
    };
    pub use crate::{
        Canvas, DrawCommand, DrawContext, Drawable, FillRule, LineCap, LineJoin, Paint, PathClip,
        RectClip, RenderQuality, Shape, ShapeClip, StrokeStyle, Styled,
    };
    pub use crate::{
        CrossFade, Iris, IrisShape, MotionBlur, Push, Slide, Transition, Wipe, ZoomBlur,
    };
    pub use crate::{Duration, FrameRate, Random, TICKS_PER_SECOND, Time, TimeLiteral};
    pub use crate::{
        Error, ErrorKind, SceneStats, ValidationError, ValidationErrorKind, ValidationLimits,
    };
    pub use crate::{
        Extrapolate, FollowPath, Noise, Oscillator, PathSample, Waveform, piecewise, remap,
    };
    pub use crate::{
        GradientExtend, GradientSpace, GradientUnits, ImagePattern, LinearGradient, RadialGradient,
        SweepGradient,
    };
    pub use crate::{MorphPath, Path2D, PathVerb};
    pub use crate::{group, scene};
    pub use crate::{linear_to_srgb, premultiply, rec709_luminance, srgb_to_linear, unpremultiply};
}
