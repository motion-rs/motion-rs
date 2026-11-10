//! Deterministic scene compositions and rendering sessions.

use motion_core::Result;
use std::io;

mod execution;
mod frame_renderer;
mod session;

use execution::FrameExecution;

use crate::{RasterLimits, Snapshot};

use motion_core::{Node, PixelSize, SceneContext, Size, ValidationError};

pub use frame_renderer::{FrameRenderer, FrameStats, FrameTimings};
pub use session::{FrameRequest, RenderSession};

/// Low-level fixed-output render configuration; author scenes with Timeline.
/// Scene callbacks are pure functions of [`motion_core::SceneContext`].
/// Default timing is 150 frames at 30 FPS (five seconds). Dimensions are output pixels.
///
/// Defaults select a white opaque background and CPU rendering without font
/// discovery. Builders defer validation to rendering operations.
/// Scene callbacks must be deterministic functions of their time and fixed inputs;
/// no prior-frame history, wall-clock time, or unseeded randomness is supplied.
///
/// # Example
///
/// ```
/// use motion_render::prelude::*;
/// use motion_core::prelude::*;
/// use motion_core::{
///     DisplayList, RasterLimits, ResourcePool, ValidatedDisplayList, MAX_PATH_VERBS,
///     MAX_STROKE_VERBS, MAX_SVG_PATH_BYTES, sample_offsets,
/// };
/// use motion_render::{Composition, RenderPrepare, RenderOptions, Renderer, render_still};
/// # fn main() -> motion_core::Result {
/// let composition = Composition::new(640, 360).frame_rate(30).frame_count(90);
/// composition.render_frame(45, Size::new(640.0, 360.0), |context| {
///     assert_eq!(context.seconds(), 1.5);
///     Node::empty()
/// })?;
/// # Ok(())
/// # }
/// ```
#[derive(Clone)]
pub struct Composition {
    quality: motion_core::RenderQuality,
    size: (usize, usize),
    background: motion_core::Color,
    frame_rate: motion_core::FrameRate,
    frame_count: u64,
    blend_space: motion_core::drawing::BlendSpace,
    display: crate::DisplayTransform,
    motion_blur: Option<motion_core::authoring::sampling::MotionBlur>,
    shutter_sampling: motion_core::ShutterSampling,
    allow_missing_assets: bool,

    backend: crate::BackendPreference,
    context: crate::RenderContext,
    #[cfg(feature = "gpu")]
    gpu_context: Option<crate::GpuContext>,
    resources: Option<motion_core::Assets>,

    render_cache: crate::RenderCache,
    raster_limits: crate::RasterLimits,

    font_usage: motion_core::FontUsage,
}

impl Composition {
    /// Stores dimensions in output pixels with default timing, background, and backend.
    /// Rendering accepts dimensions in `1..=16384`; MP4 additionally requires even
    /// dimensions. This constructor does not validate them or allocate a target.
    pub fn new(width: usize, height: usize) -> Self {
        Self {
            quality: motion_core::RenderQuality::Final,
            size: (width, height),
            background: motion_core::Color::WHITE,
            frame_rate: motion_core::FrameRate::from(30),
            frame_count: 150,
            blend_space: motion_core::drawing::BlendSpace::LinearRgb,
            display: crate::DisplayTransform::default(),
            motion_blur: None,
            shutter_sampling: motion_core::ShutterSampling::default(),
            allow_missing_assets: false,
            backend: crate::BackendPreference::Cpu,
            context: crate::RenderContext::new(),
            #[cfg(feature = "gpu")]
            gpu_context: None,
            resources: None,
            render_cache: crate::RenderCache::default(),
            raster_limits: crate::RasterLimits::default(),
            font_usage: motion_core::FontUsage::default(),
        }
    }

    /// Creates a default composition, rejecting dimensions outside `1..=16384`.
    /// Does not load fonts, initialize a device or allocate a render target.
    pub fn try_new(
        width: usize,
        height: usize,
    ) -> std::result::Result<Self, motion_core::ValidationError> {
        let composition = Self::new(width, height);
        composition.preflight()?;
        Ok(composition)
    }

    /// Applies shared session settings with the same validation used by Timeline output.
    /// Absent size retains this composition's viewport. Output backgrounds are still
    /// selected by render_still/render_output_at; the opaque Snapshot background is retained.
    pub fn render_options(mut self, options: &crate::RenderOptions) -> motion_core::Result<Self> {
        options.validate()?;
        if let Some(size) = options.size {
            self.size = (size.width as usize, size.height as usize);
        }
        self.backend = options.backend;
        self.context = options.context.clone();
        self.raster_limits = options.limits;
        self.resources = options.resources.clone();
        self.render_cache = options.cache.clone();
        self.font_usage = options.font_usage.clone();
        self.quality = options.quality;
        self.motion_blur = options.motion_blur;
        self.shutter_sampling = options.shutter_sampling;
        self.blend_space = options.blend_space;
        self.display = options.display.clone();
        self.allow_missing_assets = options.allow_missing_assets;
        #[cfg(feature = "gpu")]
        {
            self.gpu_context = options.gpu_context.clone();
        }
        self.preflight()?;
        self.raster_limits
            .validate_size(self.configured_output_size())?;
        Ok(self)
    }

    /// Selects interactive approximations explicitly; Final is the default.
    pub fn quality(mut self, quality: motion_core::RenderQuality) -> Self {
        self.quality = quality;
        self
    }

    /// Current explicit rendering quality.
    pub fn render_quality(&self) -> motion_core::RenderQuality {
        self.quality
    }

    /// Selects compositing space while retaining linear-premultiplied storage.
    pub fn blend_space(mut self, space: motion_core::drawing::BlendSpace) -> Self {
        self.blend_space = space;
        self
    }

    /// Sets output grading and highlight compression without changing scene compositing.
    pub fn display(mut self, display: crate::DisplayTransform) -> Self {
        self.display = display;
        self
    }

    /// Enables whole-frame shutter capture; sampled still/preview/export
    /// entries evaluate repeatable callbacks.
    pub fn motion_blur(
        mut self,
        configuration: motion_core::authoring::sampling::MotionBlur,
    ) -> std::result::Result<Self, motion_core::ValidationError> {
        configuration.validate()?;
        self.motion_blur = Some(configuration);
        Ok(self)
    }

    /// Attaches a unified lazy project resource owner to scene contexts.
    pub fn resources(mut self, resources: motion_core::Assets) -> Self {
        self.resources = Some(resources);
        self
    }

    /// Allows resource placeholders instead of failing after a frame.
    /// Defaults false for still/video output; Studio preview opts in explicitly.
    pub fn allow_missing_assets(mut self, allow: bool) -> Self {
        self.allow_missing_assets = allow;
        self
    }

    /// Sets the exact rational rate used by Node sampling.
    /// Replaces a previous fps setting; a later fps call replaces this rate.
    /// Preflight rejects rates above the tick grid or unrepresentable durations.
    /// Still, video and Studio consumers preserve the rational rate.
    ///
    /// ```
    /// use motion_render::Composition;
    /// use motion_core::{Size, FrameRate, Node, Time};
    /// let composition = Composition::new(8, 4)
    ///     .frame_rate(FrameRate::NTSC)
    ///     .frame_count(30);
    /// let mut renderer = composition.frame_renderer(Size::new(8.0, 4.0))?;
    /// renderer.render_frame(1, |context| {
    ///     assert_eq!(context.frame_rate(), FrameRate::NTSC);
    ///     assert_eq!(context.time(), Time::from_frame(1, FrameRate::NTSC));
    ///     Node::empty()
    /// })?;
    /// # Ok::<(), motion_core::Error>(())
    /// ```
    pub fn frame_rate(mut self, rate: impl Into<motion_core::FrameRate>) -> Self {
        self.frame_rate = rate.into();
        self
    }

    /// Sets the number of frames, not seconds; zero is rejected during frame lookup.
    pub fn frame_count(mut self, frame_count: u64) -> Self {
        self.frame_count = frame_count;
        self
    }

    /// Sets the opaque output background. Its RGB channels are used; alpha is ignored.
    pub fn background(mut self, color: impl Into<motion_core::Color>) -> Self {
        self.background = color.into();
        self
    }

    /// Selects the requested backend without initializing it. Explicit GPU
    /// requests fail on unsupported configurations rather than silently falling
    /// back to CPU; software adapters require `allow_software`.
    pub fn backend(mut self, backend: crate::BackendPreference) -> Self {
        self.backend = backend;
        self
    }

    /// Backend requested for sessions created from this composition.
    pub fn requested_backend(&self) -> crate::BackendPreference {
        self.backend
    }

    /// Validated rational frame rate, without constructing scene content.
    pub fn configured_frame_rate(&self) -> motion_core::Result<motion_core::FrameRate> {
        self.preflight()?;
        Ok(self.frame_rate)
    }

    /// Opaque composition background; transparent output can override it.
    pub fn background_color(&self) -> motion_core::Color {
        self.background
    }

    /// Configured output dimensions without constructing scene content.
    pub fn configured_output_size(&self) -> motion_core::PixelSize {
        motion_core::PixelSize::new(self.size.0 as u32, self.size.1 as u32)
    }

    /// Number of configured output frames, independent of the rational rate.
    pub fn configured_frame_count(&self) -> u64 {
        self.frame_count
    }

    /// Changes only the output viewport while preserving native resources/timing.
    pub fn output_size(mut self, size: motion_core::PixelSize) -> motion_core::Result<Self> {
        self.size = (size.width as usize, size.height as usize);
        self.preflight()?;
        self.raster_limits.validate_size(size)?;
        Ok(self)
    }

    /// Sets the resources and structural policy used by preview and video export.
    /// Cloned contexts share asset snapshots and structural policy. Sessions retain
    /// independent targets/caches, and font snapshots come from explicit resources.
    pub fn render_context(mut self, context: crate::RenderContext) -> Self {
        self.context = context;
        self
    }

    /// The resource context used by default frame sessions and video rendering.
    pub fn context(&self) -> &crate::RenderContext {
        &self.context
    }

    fn shutter(&self) -> Option<motion_core::MotionBlur> {
        if self.quality == motion_core::RenderQuality::Draft {
            None
        } else {
            self.motion_blur
        }
    }

    /// Checks dimensions and timing without initializing fonts or a backend.
    /// Video-specific constraints, including even dimensions, are checked at export.
    /// Font data, selected-backend support and raster budgets are not checked here.
    /// Rates above the tick grid and durations outside signed ticks are rejected.
    pub fn preflight(&self) -> std::result::Result<(), motion_core::ValidationError> {
        self.display.validate()?;
        for (field, valid, message) in [
            (
                "width",
                (1..=16384).contains(&self.size.0),
                "Require 1..=16384 pixels",
            ),
            (
                "height",
                (1..=16384).contains(&self.size.1),
                "Require 1..=16384 pixels",
            ),
            (
                "frame_count",
                self.frame_count > 0,
                "Duration must be nonzero",
            ),
        ] {
            if !valid {
                return Err(motion_core::ValidationError::invalid(
                    "composition",
                    field,
                    message,
                ));
            }
        }
        let rate = self.frame_rate;
        if u64::from(rate.numerator())
            > motion_core::TICKS_PER_SECOND as u64 * u64::from(rate.denominator())
        {
            return Err(motion_core::ValidationError::invalid(
                "composition",
                "frame_rate",
                "Frame rate must not exceed one frame per tick",
            ));
        }
        self.checked_duration()?;
        Ok(())
    }

    fn checked_duration(&self) -> std::result::Result<motion_core::Time, ValidationError> {
        i64::try_from(self.frame_count)
            .ok()
            .and_then(|frames| motion_core::Time::checked_from_frame(frames, self.frame_rate))
            .ok_or_else(|| {
                ValidationError::invalid(
                    "composition",
                    "frame_count",
                    "Composition duration exceeds the signed tick range",
                )
            })
    }

    /// Builds and measures one frame without engine device initialization or font discovery.
    /// Calls the builder once after configuration and index validation; panics propagate.
    /// Scene diagnostics carry the requested frame index. Configuration/index errors
    /// occur before scene evaluation and are not given a scene diagnostic's frame tag.
    /// Explicit fonts may be used for layout while checking bounds. This does not
    /// record draw commands, probe backend devices or admit raster budgets.
    /// Custom callbacks/bounds implementations remain trusted.
    pub fn preflight_frame(
        &self,
        index: u32,
        design_size: motion_core::Size,
        build: impl FnOnce(&motion_core::SceneContext<'_>) -> motion_core::Node,
    ) -> Result<motion_core::SceneStats> {
        self.preflight()?;
        self.validate_frame_index(index)?;
        self.configured_output_size().mapping(design_size)?;
        let rate = self.configured_frame_rate()?;
        let cache = motion_core::SceneCache::new(0);
        let mut context = motion_core::SceneContext::new(
            motion_core::Time::from_frame(i64::from(index), rate),
            rate,
            design_size,
            &cache,
        );
        if let Some(resources) = &self.resources {
            context = context.with_assets(resources);
        }
        let mut draw = motion_core::DrawContext::default();
        if let Some(fonts) = self.resources.as_ref().and_then(motion_core::Assets::fonts) {
            draw = draw.with_fonts(fonts.clone());
        }
        build(&context)
            .preflight_with(&draw, self.context.validation_limits())
            .map_err(|error| motion_core::Error::from(error).at_frame(u64::from(index)))
    }

    fn validate_frame_index(&self, index: u32) -> Result {
        if self.frame_count == 0 || u64::from(index) >= self.frame_count {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "Duration must be nonzero, and frame index must be below duration",
            )
            .into());
        }
        Ok(())
    }

    /// Enables jitter, temporal weights and adaptive motion sampling.
    pub fn shutter_sampling(mut self, policy: motion_core::ShutterSampling) -> Self {
        self.shutter_sampling = policy;
        self
    }

    /// Builds the same render policy for preview/export consumers. `backend` replaces
    /// the requested backend; the software adapter policy of this composition is kept.
    pub fn studio_render_options(&self, backend: crate::BackendPreference) -> crate::RenderOptions {
        crate::RenderOptions {
            quality: self.quality,
            size: Some(self.configured_output_size()),
            background: Some(self.background.with_alpha(1.0)),
            motion_blur: self.motion_blur,
            shutter_sampling: self.shutter_sampling,
            blend_space: self.blend_space,
            display: self.display.clone(),
            allow_missing_assets: self.allow_missing_assets,
            backend: backend.with_software(self.backend.allow_software()),
            #[cfg(feature = "gpu")]
            gpu_context: self.gpu_context.clone(),
            resources: self.resources.clone(),
            context: self.context().clone(),
            cache: self.render_cache.clone(),
            limits: self.raster_limits,
            font_usage: self.font_usage.clone(),
        }
    }

    /// Shares rendering caches without changing backend, viewport or output policy.
    pub fn render_cache(mut self, cache: crate::RenderCache) -> Self {
        self.render_cache = cache;
        self
    }

    /// Shares a validated native GPU device/queue with preview/export sessions.
    /// This does not select GPU automatically.
    #[cfg(feature = "gpu")]
    pub fn gpu_context(mut self, context: crate::GpuContext) -> Self {
        self.gpu_context = Some(context);
        self
    }

    /// Sets this composition's per-frame raster allowances.
    ///
    /// Validates dimensions/timing, positive limits and output pixel admission
    /// without building scenes or initializing devices. New still, preview
    /// and video sessions inherit a copy; session overrides do not change this
    /// configuration. Concurrent sessions each
    /// receive the full allowance, not a shared process budget. Source assets,
    /// caches, allocator capacity and driver overhead are outside these logical
    /// limits, which do not cap process RSS or physical VRAM.
    ///
    /// ```
    /// use motion_render::prelude::*;
    /// use motion_core::prelude::*;
    /// use motion_core::{
    ///     DisplayList, RasterLimits, ResourcePool, ValidatedDisplayList, MAX_PATH_VERBS,
    ///     MAX_STROKE_VERBS, MAX_SVG_PATH_BYTES, sample_offsets,
    /// };
    /// use motion_render::{Composition, RenderPrepare, RenderOptions, Renderer, render_still};
    /// let composition = Composition::new(960, 540).raster_limits(RasterLimits {
    ///     max_bytes: 512 * 1024 * 1024,
    ///     ..RasterLimits::default()
    /// })?;
    /// assert_eq!(composition.configured_raster_limits().max_bytes, 512 * 1024 * 1024);
    /// # Ok::<(), Box<dyn std::error::Error>>(())
    /// ```
    pub fn raster_limits(
        mut self,
        limits: RasterLimits,
    ) -> std::result::Result<Self, ValidationError> {
        self.preflight()?;
        limits.validate_size(PixelSize::new(self.size.0 as u32, self.size.1 as u32))?;
        self.raster_limits = limits;
        Ok(self)
    }

    /// per-frame policy inherited by newly created sessions.
    /// Defaults to RasterLimits::default.
    pub fn configured_raster_limits(&self) -> RasterLimits {
        self.raster_limits
    }

    /// Creates an explicit consumer for Node scenes and logical dimensions.
    ///
    /// Uses this composition's timing, structural policy and opaque RGB background;
    /// background alpha is ignored, as in Snapshot's opaque contract.
    /// Renderer::Gpu requires the gpu feature, initializes lazily on the first
    /// admitted render, and executes prepared coverage on f32 compute
    /// buffers. Explicit GPU requests never fall back to CPU; Automatic policy may
    /// select CPU during startup. Both backends implement the same output-format
    /// contract, but supported GPU formats use dedicated packing kernels.
    /// GPU f32 arithmetic is not bit-identical to CPU intermediates.
    /// Unrepresentable parameters or device limits fail explicitly.
    /// Dimensions/design mapping validate before any scene callback or allocation.
    /// Inherits raster_limits; raster_limits overrides only this session.
    /// Existing frame_renderer/render_frame and export paths are unchanged.
    ///
    /// ```
    /// use motion_render::prelude::*;
    /// use motion_core::prelude::*;
    /// use motion_core::{
    ///     DisplayList, RasterLimits, ResourcePool, ValidatedDisplayList, MAX_PATH_VERBS,
    ///     MAX_STROKE_VERBS, MAX_SVG_PATH_BYTES, sample_offsets,
    /// };
    /// use motion_render::{Composition, RenderPrepare, RenderOptions, Renderer, render_still};
    /// let composition = Composition::new(8, 4).background(Color::BLACK);
    /// let mut renderer = composition.frame_renderer(Size::new(8.0, 4.0))?;
    /// let snapshot = renderer.render_frame(0, |context| {
    ///     Node::from(
    ///         Rect::new(0.0, 0.0, context.size().width, context.size().height).fill(Color::WHITE),
    ///     )
    ///     .opacity(0.5)
    /// })?;
    /// assert_eq!(snapshot.pixels()[0], 0xbbbbbb);
    /// assert!(renderer.last_stats().unwrap().work() > 0);
    /// # Ok::<(), Box<dyn std::error::Error>>(())
    /// ```
    pub fn frame_renderer(&self, design_size: Size) -> Result<FrameRenderer> {
        self.preflight()?;
        if matches!(self.backend, crate::BackendPreference::Gpu { .. }) && !cfg!(feature = "gpu") {
            return Err(motion_core::Error::GpuUnavailable(Box::new(
                ValidationError::unsupported(
                    "composition",
                    "renderer",
                    "Native GPU rendering requires the gpu feature",
                ),
            )));
        }
        let size = PixelSize::new(self.size.0 as u32, self.size.1 as u32);
        size.mapping(design_size)?;
        self.raster_limits.validate_size(size)?;
        Ok(FrameRenderer {
            composition: self.clone(),
            duration: self.checked_duration()?,
            session: RenderSession::new(design_size, self.studio_render_options(self.backend))?,
        })
    }

    /// Renders one frame without loading fonts; GPU selection initializes
    /// a device lazily. Prefer a reusable session to retain that device.
    /// Prefer frame_renderer for repeated or out-of-order frame requests.
    /// With global motion blur, use render_frame_sampled for repeatable builds.
    pub fn render_frame(
        &self,
        index: u32,
        design_size: Size,
        build: impl FnOnce(&SceneContext<'_>) -> Node,
    ) -> Result<Snapshot> {
        self.frame_renderer(design_size)?.render_frame(index, build)
    }

    /// Renders a repeatable callback, honoring whole-frame shutter settings.
    pub fn render_frame_sampled(
        &self,
        index: u32,
        design_size: Size,
        build: impl FnMut(&SceneContext<'_>) -> Node,
    ) -> Result<Snapshot> {
        self.frame_renderer(design_size)?
            .render_frame_sampled(index, build)
    }
}

#[cfg(test)]
mod _contract_tests {
    use super::Composition;
    use motion_core::{
        Color, Easing, FrameRate, Keyframes, Node, NodeModifiers, Rect, SceneCache, SceneContext,
        Size, Time, TimeLiteral, Tween,
    };

    #[test]
    fn cpu_consumer_bypasses_fonts_and_composites_in_linear_light() -> motion_core::Result {
        let frame = Composition::new(8, 4)
            .background(Color::BLACK)
            .render_frame(0, Size::new(8.0, 4.0), |_| {
                Node::from(Rect::new(0.0, 0.0, 8.0, 4.0).fill(Color::WHITE)).opacity(0.5)
            })?;
        assert_eq!(frame.pixels()[0], 0xbbbbbb);
        Ok(())
    }

    #[test]
    fn local_ticks_keep_sequence_boundaries_samples_and_seek_order() {
        let cache = SceneCache::new(0);
        let rate = FrameRate::from(30);
        let context = |index: i64| {
            SceneContext::new(
                Time::from_frame(index, rate),
                rate,
                Size::new(640.0, 360.0),
                &cache,
            )
        };
        let tween = Tween::new(0.0, 100.0, 1.seconds()).easing(Easing::QuadraticIn);
        assert_eq!(context(15).sample(&tween), 25.0);
        let track = Keyframes::new(0.0).to(1.seconds(), 100.0, Easing::QuadraticOut);
        for (index, expected) in [(25, 100.0), (10, 0.0), (15, 75.0), (25, 100.0)] {
            let local = SceneContext::new(
                Time::from_frame(index, FrameRate::from(10)),
                FrameRate::from(10),
                Size::new(640.0, 360.0),
                &cache,
            )
            .shifted(1.seconds());
            assert_eq!(local.sample(&track), expected);
        }
        let calls = std::cell::Cell::new(0);
        for (index, active) in [(9, false), (10, true), (39, true), (40, false), (10, true)] {
            let before = calls.get();
            context(index).during(
                Time::from_frame(10, rate),
                Time::from_frame(30, rate),
                |local| {
                    calls.set(calls.get() + 1);
                    assert_eq!(local.time(), Time::from_frame(index - 10, rate));
                    assert_eq!(local.global_time(), Time::from_frame(index, rate));
                    assert_eq!(local.size(), Size::new(640.0, 360.0));
                    Node::empty()
                },
            );
            assert_eq!(calls.get() - before, usize::from(active));
        }
        assert_eq!(context(45).time(), 1.5.seconds());
        assert_eq!(context(45).shifted(1.seconds()).time(), 500.milliseconds());
        assert_eq!(Time::from_frame(11, FrameRate::from(11)), 1.seconds());
    }

    #[test]
    fn sessions_preserve_rational_metadata_and_reject_invalid_indices_before_building()
    -> motion_core::Result {
        let rate = FrameRate::NTSC;
        let composition = Composition::new(640, 360).frame_rate(rate).frame_count(61);
        let mut renderer = composition.frame_renderer(Size::new(640.0, 360.0))?;
        for index in [60, 0, 30, 60] {
            renderer.render_frame(index, |context| {
                assert_eq!(context.time(), Time::from_frame(i64::from(index), rate));
                assert_eq!(context.frame_rate(), rate);
                assert_eq!(context.size(), Size::new(640.0, 360.0));
                Node::empty()
            })?;
        }
        assert!(
            renderer
                .render_frame(61, |_| panic!("out-of-range builder"))
                .is_err()
        );
        assert!(
            Composition::new(8, 4)
                .frame_count(0)
                .frame_renderer(Size::new(8.0, 4.0))
                .is_err()
        );
        Ok(())
    }

    #[test]
    fn cached_renderers_produce_independent_frames_in_any_order() -> motion_core::Result {
        let composition = Composition::new(64, 64).background(Color::WHITE);
        let mut renderer = composition.frame_renderer(Size::new(64.0, 64.0))?;
        let mut previous = None;
        for index in [2, 0, 2] {
            let output = renderer.render_frame(index, |context| {
                let width = (context.time().to_frame(context.frame_rate()) + 1) as f64 * 16.0;
                Node::from(Rect::new(0.0, 0.0, width, 64.0).fill(Color::from_hex(0x118677)))
            })?;
            assert_eq!(
                output.pixels()[5 * 64 + 40],
                if index == 2 { 0x118677 } else { 0xffffff }
            );
            if index == 2 {
                if let Some(previous) = &previous {
                    assert_eq!(output.pixels(), previous);
                }
                previous = Some(output.pixels().to_vec());
            }
        }
        Ok(())
    }
}
