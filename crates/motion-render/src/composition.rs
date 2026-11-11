//! Deterministic scene compositions and rendering sessions.

use motion_core::Result;
use std::io;

mod execution;
mod frame_renderer;
mod session;

use execution::FrameExecution;

use crate::{FrameBuffer, RasterLimits};

use motion_core::{Node, PixelSize, SceneContext, Size, ValidationError};

pub use frame_renderer::{FrameRenderer, FrameReport, FrameStats, FrameTimings};
pub use session::{FrameAt, FrameRequest, RenderSession};

/// Frame rate and frame count of a composition; there is no default clock.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct FrameClock {
    rate: motion_core::FrameRate,
    count: u64,
}

impl FrameClock {
    /// Creates a clock of `count` frames at `rate`; `count` is frames, not seconds.
    pub fn new(rate: motion_core::FrameRate, count: u64) -> Self {
        Self { rate, count }
    }

    /// Exact rational frame rate.
    pub fn rate(self) -> motion_core::FrameRate {
        self.rate
    }

    /// Number of frames; zero is rejected by preflight.
    pub fn count(self) -> u64 {
        self.count
    }
}

/// Low-level fixed-output render configuration; author scenes with Timeline.
/// Scene callbacks are pure functions of [`motion_core::SceneContext`].
/// The frame clock is explicit. Dimensions are output pixels.
///
/// Rendering policy lives in one [`crate::RenderOptions`]: defaults select CPU rendering
/// without font discovery, and the opaque `render_frame*` background is white unless
/// `RenderOptions::background` is set. Validation happens in preflight and rendering.
/// Scene callbacks must be deterministic functions of their time and fixed inputs;
/// no prior-frame history, wall-clock time, or unseeded randomness is supplied.
///
/// # Example
///
/// ```
/// use motion_render::prelude::*;
/// use motion_core::prelude::*;
/// use motion_core::{
///     DisplayList, RasterLimits, ResourceStore, ValidatedDisplayList, MAX_PATH_VERBS,
///     MAX_STROKE_VERBS, MAX_SVG_PATH_BYTES, sample_offsets,
/// };
/// use motion_render::{Composition, RenderPrepare, RenderOptions, Renderer, render_still};
/// # fn main() -> motion_core::Result {
/// let composition = Composition::new(640, 360, motion_render::FrameClock::new(fps!(30), 90));
/// composition.render_frame(45, Size::new(640.0, 360.0), |context| {
///     assert_eq!(context.seconds(), 1.5);
///     Node::empty()
/// })?;
/// # Ok(())
/// # }
/// ```
#[derive(Clone)]
pub struct Composition {
    options: crate::RenderOptions,
    clock: FrameClock,
}

impl Composition {
    /// Stores dimensions in output pixels with default options and the given clock.
    /// Rendering accepts dimensions in `1..=16384`; MP4 additionally requires even
    /// dimensions. This constructor does not validate them or allocate a target.
    pub fn new(width: usize, height: usize, clock: FrameClock) -> Self {
        let pixels = |value: usize| u32::try_from(value).unwrap_or(u32::MAX);
        Self {
            options: crate::RenderOptions::default()
                .size(PixelSize::new(pixels(width), pixels(height))),
            clock,
        }
    }

    /// Creates a default composition, rejecting dimensions outside `1..=16384`.
    /// Does not load fonts, initialize a device or allocate a render target.
    pub fn try_new(
        width: usize,
        height: usize,
        clock: FrameClock,
    ) -> std::result::Result<Self, motion_core::ValidationError> {
        let composition = Self::new(width, height, clock);
        composition.preflight()?;
        Ok(composition)
    }

    /// Creates a checked composition using the timeline's exact rate and frame count.
    /// The output size is `options.size`, or the rounded-up design size; resources,
    /// limits, backend and shutter settings come from `options`.
    pub fn for_timeline(
        timeline: &motion_core::Timeline,
        options: &crate::RenderOptions,
    ) -> Result<Self> {
        let size = crate::options::size(timeline, options)?;
        Self::new(
            size.width as usize,
            size.height as usize,
            FrameClock::new(timeline.frame_rate(), timeline.frame_count()),
        )
        .with_options(options.clone())
    }

    /// Replaces the render options with the same validation used by Timeline output.
    /// Absent `size` retains this composition's viewport. Output backgrounds are still
    /// selected by render_still/render_output_at; `render_frame*` output composites over the
    /// opaque `background`, white when absent.
    pub fn with_options(mut self, mut options: crate::RenderOptions) -> Result<Self> {
        options.validate()?;
        options.size = options.size.or(self.options.size);
        self.options = options;
        self.preflight()?;
        self.options.limits.validate_size(self.output_size())?;
        Ok(self)
    }

    /// Adjusts individual options, for example `|options| options.quality(..)`.
    /// Validation is deferred to preflight and rendering.
    pub fn configure(
        mut self,
        adjust: impl FnOnce(crate::RenderOptions) -> crate::RenderOptions,
    ) -> Self {
        let size = self.options.size;
        self.options = adjust(self.options);
        self.options.size = self.options.size.or(size);
        self
    }

    /// Replaces the frame clock.
    pub fn with_clock(mut self, clock: FrameClock) -> Self {
        self.clock = clock;
        self
    }

    /// Changes only the output viewport while preserving native resources/timing.
    pub fn with_output_size(mut self, size: PixelSize) -> Result<Self> {
        self.options.size = Some(size);
        self.preflight()?;
        self.options.limits.validate_size(size)?;
        Ok(self)
    }

    /// Render options shared by every session created from this composition.
    /// `size` is always present and equals [`Self::output_size`].
    pub fn options(&self) -> &crate::RenderOptions {
        &self.options
    }

    /// Frame rate and count.
    pub fn clock(&self) -> FrameClock {
        self.clock
    }

    /// Exact rational frame rate used by Node sampling. Preflight rejects rates above
    /// the tick grid; still, video and Studio consumers preserve the rational rate.
    pub fn frame_rate(&self) -> motion_core::FrameRate {
        self.clock.rate
    }

    /// Number of output frames, independent of the rational rate.
    pub fn frame_count(&self) -> u64 {
        self.clock.count
    }

    /// Output dimensions without constructing scene content.
    pub fn output_size(&self) -> PixelSize {
        self.options.size.unwrap_or(PixelSize::new(0, 0))
    }

    /// Per-frame raster allowances inherited by newly created sessions.
    pub fn raster_limits(&self) -> RasterLimits {
        self.options.limits
    }

    /// Requested backend for sessions created from this composition.
    pub fn requested_backend(&self) -> crate::BackendPreference {
        self.options.backend
    }

    /// The resource context used by default frame sessions and video rendering.
    pub fn context(&self) -> &crate::RenderContext {
        &self.options.context
    }

    /// The single opaque-output rule: `render_frame*` composites over
    /// `RenderOptions::background` with its alpha ignored, or over white when no
    /// background is set. `render_output_at` and `render_still` instead use their
    /// explicit background and preserve transparency when it is `None`.
    pub fn snapshot_background(&self) -> motion_core::Color {
        self.options
            .background
            .unwrap_or(motion_core::Color::WHITE)
            .with_alpha(1.0)
    }

    fn shutter(&self) -> Option<motion_core::MotionBlur> {
        if self.options.quality == motion_core::RenderQuality::Draft {
            None
        } else {
            self.options.motion_blur
        }
    }

    /// Checks dimensions and timing without initializing fonts or a backend.
    /// Video-specific constraints, including even dimensions, are checked at export.
    /// Font data, selected-backend support and raster budgets are not checked here.
    /// Rates above the tick grid and durations outside signed ticks are rejected.
    pub fn preflight(&self) -> std::result::Result<(), motion_core::ValidationError> {
        self.options.display.validate()?;
        if let Some(shutter) = self.options.motion_blur {
            shutter.validate()?;
        }
        let size = self.output_size();
        for (field, valid, message) in [
            (
                "width",
                (1..=16384).contains(&size.width),
                "Require 1..=16384 pixels",
            ),
            (
                "height",
                (1..=16384).contains(&size.height),
                "Require 1..=16384 pixels",
            ),
            (
                "frame_count",
                self.clock.count > 0,
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
        let rate = self.clock.rate;
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
        i64::try_from(self.clock.count)
            .ok()
            .and_then(|frames| motion_core::Time::checked_from_frame(frames, self.clock.rate))
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
        self.output_size().mapping(design_size)?;
        let rate = self.clock.rate;
        let cache = motion_core::SceneCache::new(0);
        let mut context = motion_core::SceneContext::new(
            motion_core::Time::from_frame(i64::from(index), rate),
            rate,
            design_size,
            &cache,
        );
        if let Some(resources) = &self.options.resources {
            context = context.with_assets(resources);
        }
        let mut draw = motion_core::DrawContext::default();
        if let Some(fonts) = self
            .options
            .resources
            .as_ref()
            .and_then(motion_core::Assets::fonts)
        {
            draw = draw.with_fonts(fonts.clone());
        }
        build(&context)
            .preflight_with(&draw, self.options.context.validation_limits())
            .map_err(|error| motion_core::Error::from(error).at_frame(u64::from(index)))
    }

    fn validate_frame_index(&self, index: u32) -> Result {
        if self.clock.count == 0 || u64::from(index) >= self.clock.count {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "Duration must be nonzero, and frame index must be below duration",
            )
            .into());
        }
        Ok(())
    }

    /// Builds the same render policy for preview/export consumers. `backend` replaces
    /// the requested backend; the software adapter policy of this composition is kept.
    pub fn studio_render_options(&self, backend: crate::BackendPreference) -> crate::RenderOptions {
        crate::RenderOptions {
            background: Some(self.snapshot_background()),
            backend: backend.with_software(self.options.backend.allow_software()),
            ..self.options.clone()
        }
    }

    /// Creates an explicit consumer for Node scenes and logical dimensions.
    ///
    /// Uses this composition's timing, structural policy and opaque RGB background;
    /// background alpha is ignored, as in the opaque `render_frame*` contract.
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
    ///     DisplayList, RasterLimits, ResourceStore, ValidatedDisplayList, MAX_PATH_VERBS,
    ///     MAX_STROKE_VERBS, MAX_SVG_PATH_BYTES, sample_offsets,
    /// };
    /// use motion_render::{Composition, RenderPrepare, RenderOptions, Renderer, render_still};
    /// let composition = Composition::new(8, 4, motion_render::FrameClock::new(fps!(30), 150)).with_options(motion_render::RenderOptions::default().background(Color::BLACK))?;
    /// let mut renderer = composition.frame_renderer(Size::new(8.0, 4.0))?;
    /// let snapshot = renderer.render_frame(0, |context| {
    ///     Node::from(
    ///         Rect::new(0.0, 0.0, context.size().width, context.size().height).fill(Color::WHITE),
    ///     )
    ///     .opacity(0.5)
    /// })?;
    /// assert_eq!(snapshot.packed_rgb()[0], 0xbbbbbb);
    /// assert!(renderer.last_stats().unwrap().work() > 0);
    /// # Ok::<(), Box<dyn std::error::Error>>(())
    /// ```
    pub fn frame_renderer(&self, design_size: Size) -> Result<FrameRenderer> {
        self.preflight()?;
        if matches!(self.options.backend, crate::BackendPreference::Gpu { .. })
            && !cfg!(feature = "gpu")
        {
            return Err(motion_core::Error::GpuUnavailable(Box::new(
                ValidationError::unsupported(
                    "composition",
                    "renderer",
                    "Native GPU rendering requires the gpu feature",
                ),
            )));
        }
        let size = self.output_size();
        size.mapping(design_size)?;
        self.options.limits.validate_size(size)?;
        Ok(FrameRenderer {
            composition: self.clone(),
            duration: self.checked_duration()?,
            session: RenderSession::new(
                design_size,
                self.studio_render_options(self.options.backend),
            )?,
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
    ) -> Result<FrameBuffer> {
        self.frame_renderer(design_size)?.render_frame(index, build)
    }

    /// Renders a repeatable callback, honoring whole-frame shutter settings.
    pub fn render_frame_sampled(
        &self,
        index: u32,
        design_size: Size,
        build: impl FnMut(&SceneContext<'_>) -> Node,
    ) -> Result<FrameBuffer> {
        self.frame_renderer(design_size)?
            .render_frame_sampled(index, build)
    }
}

#[cfg(test)]
mod _contract_tests {
    use super::Composition;
    use motion_core::Shape;
    use motion_core::{
        Color, Easing, FrameRate, Keyframes, Node, NodeModifiers, Rect, SceneCache, SceneContext,
        Size, Time, TimeLiteral, Tween,
    };

    #[test]
    fn cpu_consumer_bypasses_fonts_and_composites_in_linear_light() -> motion_core::Result {
        let frame = Composition::new(8, 4, crate::FrameClock::new(motion_core::fps!(30), 150))
            .with_options(crate::RenderOptions::default().background(Color::BLACK))?
            .render_frame(0, Size::new(8.0, 4.0), |_| {
                Node::from(Rect::new(0.0, 0.0, 8.0, 4.0).fill(Color::WHITE)).opacity(0.5)
            })?;
        assert_eq!(frame.packed_rgb()[0], 0xbbbbbb);
        Ok(())
    }

    #[test]
    fn local_ticks_keep_sequence_boundaries_samples_and_seek_order() {
        let cache = SceneCache::new(0);
        let rate = motion_core::fps!(30);
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
                Time::from_frame(index, motion_core::fps!(10)),
                motion_core::fps!(10),
                Size::new(640.0, 360.0),
                &cache,
            )
            .shifted(1.seconds().offset());
            assert_eq!(local.sample(&track), expected);
        }
        let calls = std::cell::Cell::new(0);
        for (index, active) in [(9, false), (10, true), (39, true), (40, false), (10, true)] {
            let before = calls.get();
            context(index).during(
                Time::from_frame(10, rate),
                motion_core::Duration::from_frame(30, rate),
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
        assert_eq!(context(45).time(), 1.5.seconds().after_origin());
        assert_eq!(
            context(45).shifted(1.seconds().offset()).time(),
            500.milliseconds().after_origin()
        );
        assert_eq!(
            Time::from_frame(11, motion_core::fps!(11)),
            1.seconds().after_origin()
        );
    }

    #[test]
    fn sessions_preserve_rational_metadata_and_reject_invalid_indices_before_building()
    -> motion_core::Result {
        let rate = FrameRate::NTSC;
        let composition = Composition::new(640, 360, crate::FrameClock::new(rate, 61));
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
            Composition::new(8, 4, crate::FrameClock::new(motion_core::fps!(30), 0))
                .frame_renderer(Size::new(8.0, 4.0))
                .is_err()
        );
        Ok(())
    }

    #[test]
    fn cached_renderers_produce_independent_frames_in_any_order() -> motion_core::Result {
        let composition =
            Composition::new(64, 64, crate::FrameClock::new(motion_core::fps!(30), 150))
                .with_options(crate::RenderOptions::default().background(Color::WHITE))?;
        let mut renderer = composition.frame_renderer(Size::new(64.0, 64.0))?;
        let mut previous = None;
        for index in [2, 0, 2] {
            let output = renderer.render_frame(index, |context| {
                let width = (context.time().to_frame(context.frame_rate()) + 1) as f64 * 16.0;
                Node::from(Rect::new(0.0, 0.0, width, 64.0).fill(Color::from_hex(0x118677)))
            })?;
            assert_eq!(
                output.packed_rgb()[5 * 64 + 40],
                if index == 2 { 0x118677 } else { 0xffffff }
            );
            if index == 2 {
                if let Some(previous) = &previous {
                    assert_eq!(output.packed_rgb(), *previous);
                }
                previous = Some(output.packed_rgb());
            }
        }
        Ok(())
    }
}
