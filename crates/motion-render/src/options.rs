//! Shared renderer selection and still, export and session configuration.
use crate::*;
use motion_core::Scene;
use motion_core::{Color, PixelSize, Result, Time, Timeline, ValidationError};

/// Shared render-session configuration for stills, exports and direct compositions.
#[derive(Clone)]
pub struct RenderOptions {
    /// Explicit approximation policy; production and export default to Final.
    pub quality: motion_core::RenderQuality,
    /// Defaults to ceil(design dimensions).
    pub size: Option<PixelSize>,
    /// Background used by render_still and export; None preserves transparency.
    /// A direct FrameRequest supplies its own background. Composition's opaque
    /// Snapshot background is not replaced by render_options.
    pub background: Option<Color>,
    /// Optional whole-frame shutter.
    pub motion_blur: Option<motion_core::MotionBlur>,
    /// Opt-in jitter, temporal weights and adaptive whole-frame sample selection.
    pub shutter_sampling: motion_core::ShutterSampling,
    /// Compositing color space; storage remains linear-premultiplied.
    pub blend_space: motion_core::BlendSpace,
    /// Opt-in display transform, applied after shutter accumulation.
    pub display: crate::DisplayTransform,
    /// Allows diagnostic resource placeholders instead of failing a frame.
    pub allow_missing_assets: bool,

    /// Explicit backend; CPU is the default. GPU requests carry their own software adapter policy.
    pub backend: BackendPreference,
    /// Optional shared native device (for example Studio's eframe context).
    /// Does not select a backend.
    #[cfg(feature = "gpu")]
    pub gpu_context: Option<crate::GpuContext>,
    /// Immutable lazy project resources.
    pub resources: Option<motion_core::Assets>,
    /// Project objects and structural policy.
    /// Separate from lazy resources; neither performs implicit font discovery.
    pub context: crate::RenderContext,

    /// Partitioned pure caches shared by clones, sessions and export workers.
    pub cache: crate::RenderCache,
    /// Per-frame logical raster bounds.
    pub limits: RasterLimits,

    /// Explicit source observations shared across this rendering operation.
    pub font_usage: motion_core::FontUsage,
}

impl Default for RenderOptions {
    fn default() -> Self {
        Self {
            quality: motion_core::RenderQuality::Final,
            size: None,
            background: None,
            motion_blur: None,
            shutter_sampling: motion_core::ShutterSampling::default(),
            blend_space: motion_core::BlendSpace::LinearRgb,
            display: crate::DisplayTransform::default(),
            allow_missing_assets: false,
            backend: BackendPreference::Cpu,
            #[cfg(feature = "gpu")]
            gpu_context: None,
            resources: None,
            context: crate::RenderContext::new(),
            cache: crate::RenderCache::default(),
            limits: RasterLimits::default(),
            font_usage: motion_core::FontUsage::default(),
        }
    }
}

impl RenderOptions {
    /// Sets the approximation policy.
    pub fn quality(mut self, quality: motion_core::RenderQuality) -> Self {
        self.quality = quality;
        self
    }

    /// Sets explicit output dimensions instead of ceil(design dimensions).
    pub fn size(mut self, size: PixelSize) -> Self {
        self.size = Some(size);
        self
    }

    /// Composites over an opaque background.
    pub fn background(mut self, color: Color) -> Self {
        self.background = Some(color);
        self
    }

    /// Preserves alpha instead of compositing over a background.
    pub fn transparent(mut self) -> Self {
        self.background = None;
        self
    }

    /// Enables whole-frame motion blur.
    pub fn motion_blur(mut self, blur: motion_core::MotionBlur) -> Self {
        self.motion_blur = Some(blur);
        self
    }

    /// Selects the deterministic temporal sampling policy.
    pub fn shutter_sampling(mut self, policy: motion_core::ShutterSampling) -> Self {
        self.shutter_sampling = policy;
        self
    }

    /// Selects the compositing color space.
    pub fn blend_space(mut self, space: motion_core::BlendSpace) -> Self {
        self.blend_space = space;
        self
    }

    /// Sets output grading, tone mapping, transfer and deterministic grain.
    pub fn display(mut self, display: crate::DisplayTransform) -> Self {
        self.display = display;
        self
    }

    /// Allows diagnostic placeholders for missing resources.
    pub fn allow_missing_assets(mut self, allow: bool) -> Self {
        self.allow_missing_assets = allow;
        self
    }

    /// Selects the rendering backend.
    pub fn backend(mut self, backend: BackendPreference) -> Self {
        self.backend = backend;
        self
    }

    /// Shares a native device, for example Studio's eframe context. Does not select a backend.
    #[cfg(feature = "gpu")]
    pub fn gpu_context(mut self, context: crate::GpuContext) -> Self {
        self.gpu_context = Some(context);
        self
    }

    /// Supplies lazy project resources.
    pub fn resources(mut self, resources: motion_core::Assets) -> Self {
        self.resources = Some(resources);
        self
    }

    /// Supplies project objects and structural policy.
    pub fn context(mut self, context: crate::RenderContext) -> Self {
        self.context = context;
        self
    }

    /// Shares a render cache across sessions and workers.
    pub fn cache(mut self, cache: crate::RenderCache) -> Self {
        self.cache = cache;
        self
    }

    /// Sets the per-frame logical raster bounds.
    pub fn limits(mut self, limits: RasterLimits) -> Self {
        self.limits = limits;
        self
    }

    /// Validates explicit size, positive raster allowances and shutter parameters.
    /// Does not evaluate scenes, decode resources or probe devices. An inferred size
    /// is checked later against the actual timeline/session design dimensions;
    /// passing this check does not guarantee per-frame work/storage admission.
    pub fn validate(&self) -> Result<()> {
        self.display.validate()?;
        if let Some(size) = self.size
            && (!(1..=16_384).contains(&size.width) || !(1..=16_384).contains(&size.height))
        {
            return Err(ValidationError::invalid(
                "render_options",
                "size",
                "Require 1..=16384 output pixels per dimension",
            )
            .into());
        }
        self.limits
            .validate_size(self.size.unwrap_or(PixelSize::new(1, 1)))?;
        if let Some(shutter) = self.motion_blur {
            shutter.validate()?;
        }
        Ok(())
    }
}

/// Resolves and validates the requested output dimensions for a timeline.
pub fn size(timeline: &Timeline, options: &RenderOptions) -> Result<PixelSize> {
    let size = options.size.unwrap_or(PixelSize::new(
        timeline.size().width.ceil() as u32,
        timeline.size().height.ceil() as u32,
    ));
    options.limits.validate_size(size)?;
    Ok(size)
}

/// Creates a checked session template using the timeline's exact rate and frame count.
/// Carries resources, structural/raster limits, backend policy and optional shutter settings.
pub fn composition(timeline: &Timeline, options: &RenderOptions) -> Result<crate::Composition> {
    crate::Composition::for_timeline(timeline, options)
}

/// Renders an arbitrary in-range tick to straight sRGB RGBA with preserved alpha.
pub fn render_still(timeline: &Timeline, time: Time, options: &RenderOptions) -> Result<Bitmap> {
    let mut renderer = composition(timeline, options)?.frame_renderer(timeline.size())?;
    Bitmap::from_frame(renderer.render_output_at(
        time,
        options.background,
        PixelFormat::SrgbRgba8 {
            premultiplied: false,
        },
        |context| timeline.render(context),
    )?)
    .map_err(Into::into)
}

#[cfg(test)]
mod tests {
    use super::*;
    use motion_core::{Circle, FrameRate, Node, Point, SceneContext, Shape, Size, TimeLiteral};

    #[test]
    fn draft_disables_global_shutter_and_final_remains_exact() -> Result<()> {
        let options = RenderOptions::default();
        assert_eq!(options.quality, motion_core::RenderQuality::Final);
        let base = Composition::new(16, 16, crate::FrameClock::new(30, 150)).with_options(
            crate::RenderOptions::default().motion_blur(motion_core::MotionBlur {
                samples: 4,
                shutter_angle: 180.0,
            }),
        )?;
        let build = |context: &SceneContext<'_>| {
            Node::from(
                Circle::new(Point::new(8.0 + context.seconds(), 8.0), 4.0).fill(Color::WHITE),
            )
        };
        let format = PixelFormat::SrgbRgba8 {
            premultiplied: false,
        };
        let mut original = base.clone().frame_renderer(Size::new(16.0, 16.0))?;
        let expected = original.render_output_at(
            Time::from_frame(2, FrameRate::from(30)),
            None,
            format,
            build,
        )?;
        let mut final_renderer = base
            .clone()
            .configure(|options| options.quality(motion_core::RenderQuality::Final))
            .frame_renderer(Size::new(16.0, 16.0))?;
        assert_eq!(
            final_renderer.render_output_at(
                Time::from_frame(2, FrameRate::from(30)),
                None,
                format,
                build
            )?,
            expected
        );
        let mut calls = 0;
        let mut draft = base
            .configure(|options| options.quality(motion_core::RenderQuality::Draft))
            .frame_renderer(Size::new(16.0, 16.0))?;
        draft.render_output_at(
            Time::from_frame(2, FrameRate::from(30)),
            None,
            format,
            |context| {
                calls += 1;
                build(context)
            },
        )?;
        assert_eq!(calls, 1);
        Ok(())
    }

    #[test]
    fn cloned_compositions_share_coverage_between_worker_sessions() -> Result<()> {
        let options = RenderOptions::default();
        let composition = Composition::new(8, 8, crate::FrameClock::new(30, 150))
            .with_options(options.clone())?;
        let mut first = composition.frame_renderer(Size::new(8.0, 8.0))?;
        first.render_frame(0, |_| {
            Node::from(Circle::new(Point::new(4.0, 4.0), 2.75).fill(Color::WHITE))
        })?;
        let bytes = first.cached_coverage_bytes();
        assert!(bytes > 0);
        let second = composition.clone().frame_renderer(Size::new(8.0, 8.0))?;
        assert_eq!(second.cached_coverage_bytes(), bytes);
        assert_eq!(
            options
                .cache
                .partition(crate::CachePartition::Coverage)
                .len(),
            1
        );
        options.cache.clear();
        assert_eq!(first.cached_coverage_bytes(), 0);
        assert_eq!(second.cached_coverage_bytes(), 0);
        Ok(())
    }

    #[test]
    fn builders_set_the_same_fields_as_assignment() {
        let size = PixelSize::new(64, 36);
        let options = RenderOptions::default()
            .size(size)
            .background(Color::WHITE)
            .quality(motion_core::RenderQuality::Draft)
            .backend(BackendPreference::Cpu)
            .allow_missing_assets(true);
        assert_eq!(options.size, Some(size));
        assert_eq!(options.background, Some(Color::WHITE));
        assert_eq!(options.quality, motion_core::RenderQuality::Draft);
        assert!(options.allow_missing_assets);
        assert_eq!(options.transparent().background, None);
    }

    #[test]
    fn deferred_timeline_errors_fail_before_frame_callbacks() {
        let called = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
        let observed = called.clone();
        let error = Timeline::new(Size::new(8.0, 4.0), 30)
            .track(|track| {
                track
                    .push(
                        (move |_: &SceneContext<'_>| {
                            observed.store(true, std::sync::atomic::Ordering::Relaxed);
                            Node::empty()
                        })
                        .with_duration(1.seconds()),
                    )
                    .gap((-1).seconds())
            })
            .build()
            .unwrap_err();
        assert!(!error.build_errors().is_empty());
        assert!(!called.load(std::sync::atomic::Ordering::Relaxed));
    }

    #[test]
    fn shared_options_validate_direct_sessions() {
        let options = RenderOptions {
            size: Some(PixelSize::new(8, 4)),
            ..RenderOptions::default()
        };
        let composition = Composition::new(16, 8, crate::FrameClock::new(30, 150))
            .with_options(options.clone())
            .unwrap();
        assert_eq!(composition.output_size(), PixelSize::new(8, 4));
        assert_eq!(composition.frame_count(), 150);
        assert_eq!(composition.snapshot_background(), Color::WHITE);
        let invalid = RenderOptions {
            size: Some(PixelSize::new(0, 4)),
            ..RenderOptions::default()
        };
        assert!(invalid.validate().is_err());
        let invalid = RenderOptions {
            limits: RasterLimits {
                max_work: 0,
                ..RasterLimits::default()
            },
            ..RenderOptions::default()
        };
        assert!(invalid.validate().is_err());
    }
}
