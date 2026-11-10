//! Reusable backend selection, caches and explicit frame requests.
//!
//! CPU and GPU backends use only explicitly supplied fonts; nothing is discovered implicitly.

use super::*;
use crate::{RasterLimits, RenderContext, Renderer};
use motion_core::{
    Color, Node, PixelSize, Result, SceneCache, SceneContext, Size, Time, ValidationError,
};

/// Reusable rendering state, independent of composition timing and frame order.
///
/// Owns authoring/coverage caches, backend selection and latest-request diagnostics.
/// Resources and injected GPU handles may be shared through cloned RenderOptions.
/// Rendering is synchronous; sharing a device does not merge separate sessions'
/// raster allowances or diagnostic sinks into a process-wide budget. RenderCache
/// partitions are shared when sessions reuse cloned RenderOptions.
pub struct RenderSession {
    pub(super) options: crate::RenderOptions,
    pub(super) design_size: Size,
    pub(super) size: PixelSize,
    pub(super) backend: Option<crate::BackendDecision>,
    #[cfg(feature = "gpu")]
    pub(super) gpu: Option<crate::gpu::Gpu>,

    pub(super) scene_cache: std::sync::Arc<SceneCache>,
    pub(super) layout_cache: std::sync::Arc<SceneCache>,
    pub(super) coverage_cache: crate::prepared::CoverageCache,
    pub(super) cpu_pool: crate::cpu::pool::SurfacePool,
    pub(super) output_pool: crate::output::OutputPool,
    pub(super) limits: RasterLimits,

    pub(super) last_stats: Option<FrameStats>,
    pub(super) timings: Option<FrameTimings>,
    pub(super) asset_diagnostics: Vec<std::sync::Arc<motion_core::Error>>,
    pub(super) diagnostics: motion_core::RequestDiagnostics,
}

/// One exact-time render with an explicit destination and output contract.
pub struct FrameRequest<'a> {
    /// Exact root local/global time, required to lie in 0..duration.
    /// Nested scenes may derive different local clocks from this root time.
    pub time: Time,
    /// Rational timeline rate used by scene callbacks.
    pub frame_rate: motion_core::FrameRate,
    /// Positive exclusive timeline end, including shutter boundary handling.
    /// Frames must fit the session's u32 addressing; no frame-index quantization
    /// is applied to the supplied time itself.
    pub duration: Time,
    /// Per-request background, overriding the session options' background choice.
    /// None preserves transparency; opaque output formats may reject it.
    pub background: Option<Color>,
    /// Memory replaced only on success, or a compatible same-device texture.
    /// Texture writes are not a transactional rollback guarantee on GPU failure.
    pub target: crate::RenderTarget<'a>,
    /// Requested output color/alpha/storage contract.
    pub format: crate::PixelFormat,
}

impl RenderSession {
    /// Validates policy and design mapping without evaluating content or opening devices.
    pub fn new(design_size: Size, options: crate::RenderOptions) -> Result<Self> {
        options.validate()?;
        let size = options.size.unwrap_or(PixelSize::new(
            design_size.width.ceil() as u32,
            design_size.height.ceil() as u32,
        ));
        size.mapping(design_size)?;
        options.limits.validate_size(size)?;
        let scene_cache = options
            .cache
            .partition(crate::CachePartition::Scene)
            .clone();
        let layout_cache = options
            .cache
            .partition(crate::CachePartition::Paragraph)
            .clone();
        let coverage_cache = crate::prepared::CoverageCache::new(&options.cache);
        let limits = options.limits;
        #[cfg(feature = "gpu")]
        let gpu = options
            .gpu_context
            .clone()
            .map(crate::gpu::Gpu::from_context);
        Ok(Self {
            options,
            design_size,
            size,
            backend: None,
            #[cfg(feature = "gpu")]
            gpu,
            scene_cache,
            layout_cache,
            coverage_cache,
            cpu_pool: crate::cpu::pool::SurfacePool::default(),
            output_pool: crate::output::OutputPool::default(),
            limits,
            last_stats: None,
            timings: None,
            asset_diagnostics: Vec::new(),
            diagnostics: motion_core::RequestDiagnostics::default(),
        })
    }

    /// Executes one exact-time request synchronously, including configured shutter samples.
    /// Caches survive requests; each attempt installs a new diagnostic sink and clears
    /// prior totals. Memory output remains unchanged on error and its retained payload
    /// is included in admission. Output textures must satisfy their format/device contract.
    /// User callbacks may run more than once for motion blur; their panics propagate.
    pub fn render(
        &mut self,
        request: FrameRequest<'_>,
        build: impl FnMut(&SceneContext<'_>) -> Node,
    ) -> Result<()> {
        self.last_stats = None;
        self.asset_diagnostics.clear();
        self.diagnostics = motion_core::RequestDiagnostics::default();
        if request.time < Time::ZERO
            || request.time >= request.duration
            || request.duration <= Time::ZERO
        {
            return Err(ValidationError::invalid(
                "_frame",
                "time",
                "Output time must lie within the requested duration",
            )
            .into());
        }
        let count = (request.duration - Time::from_ticks(1))
            .checked_to_frame(request.frame_rate)
            .ok_or_else(|| {
                ValidationError::invalid(
                    "_frame",
                    "duration",
                    "Requested duration exceeds frame addressing",
                )
            })?;
        let count = u32::try_from(count)
            .map_err(|error| motion_core::Error::Parameters(Box::new(error)))?
            .checked_add(1)
            .ok_or_else(|| {
                ValidationError::invalid(
                    "_frame",
                    "duration",
                    "Requested duration exceeds frame addressing",
                )
            })?;
        let composition = Composition::new(
            self.size.width as usize,
            self.size.height as usize,
            crate::composition::FrameClock::new(request.frame_rate, u64::from(count)),
        )
        .with_options(self.options.clone())?;
        let retained = match &request.target {
            crate::RenderTarget::Memory(output) if !output.planes().is_empty() => {
                output.retained_bytes()
            }
            _ => 0,
        };
        let mut limits = self.limits;
        limits.max_bytes = limits
            .max_bytes
            .checked_sub(retained)
            .filter(|bytes| *bytes > 0)
            .ok_or_else(|| {
                ValidationError::limit(
                    "output",
                    "max_bytes",
                    "Retained target exhausts the request allowance",
                )
            })?;
        let mut execution = FrameExecution {
            composition,
            duration: request.duration,
            time: Some(request.time),
            limits,
            session: self,
        };
        match request.target {
            crate::RenderTarget::Memory(output) => {
                let result = execution.render_output_at(
                    request.time,
                    request.background,
                    request.format,
                    build,
                )?;
                if let Some(stats) = execution.last_stats.as_mut() {
                    stats.preparation.peak_bytes += retained;
                    stats.linear.peak_bytes += retained;
                    stats.output.peak_bytes += retained;
                }
                *output = result;
                Ok(())
            }
            #[cfg(feature = "gpu")]
            crate::RenderTarget::Texture(texture) => {
                let index = u32::try_from(request.time.to_frame(request.frame_rate))
                    .map_err(|error| motion_core::Error::Parameters(Box::new(error)))?;
                execution.render_to_texture(
                    index,
                    request.background,
                    texture,
                    request.format,
                    build,
                )
            }
        }
    }

    /// Structural policy and preloaded project registry.
    pub fn context(&self) -> &RenderContext {
        &self.options.context
    }

    /// Startup backend decision and optional fallback reason.
    pub fn backend_decision(&self) -> Option<&crate::BackendDecision> {
        self.backend.as_ref()
    }

    /// Initial configured renderer, without initializing a device.
    /// Automatic reports CPU here even after selecting GPU; use backend_decision
    /// for the actual startup choice and any fallback reason.
    pub fn renderer(&self) -> Renderer {
        if matches!(self.options.backend, crate::BackendPreference::Gpu { .. }) {
            Renderer::Gpu
        } else {
            Renderer::Cpu
        }
    }

    /// Initialized adapter metadata without probing a new device.
    pub fn gpu_info(&self) -> Option<crate::GpuInfo> {
        #[cfg(feature = "gpu")]
        if let Some(gpu) = &self.gpu {
            return Some(gpu.info().clone());
        }
        None
    }

    /// Injected/shared device without selecting the composition backend.
    #[cfg(feature = "gpu")]
    pub fn shared_gpu_context(&self) -> Option<crate::GpuContext> {
        self.gpu.as_ref().map(|gpu| gpu.context().clone())
    }

    /// Session-owned pure computation cache.
    pub fn cache(&self) -> &SceneCache {
        &self.scene_cache
    }

    /// Replaces only this session's positive raster allowances.
    pub fn raster_limits(
        &mut self,
        limits: RasterLimits,
    ) -> std::result::Result<(), ValidationError> {
        limits.validate_size(self.size)?;
        self.limits = limits;
        self.options.limits = limits;
        Ok(())
    }

    /// Logical retained paragraph layouts and keys, excluding external Arc pins and font glyph caches.
    pub fn cached_layout_bytes(&self) -> usize {
        self.layout_cache.retained_bytes()
    }

    /// Retained coverage-cache logical payload, independent of active frame storage.
    /// Allocator/node capacity and overhead are not process RSS accounting.
    pub fn cached_coverage_bytes(&self) -> u64 {
        self.coverage_cache.retained_bytes()
    }

    /// Retained logical gradient-table payload, excluding allocator capacity.
    /// This session cache is separate from active-frame accounting and process RSS.
    pub fn cached_paint_bytes(&self) -> u64 {
        self.coverage_cache.retained_paint_bytes()
    }

    /// Idle reusable CPU pixel capacity, separate from active frame payload totals.
    pub fn pooled_cpu_bytes(&self) -> u64 {
        self.cpu_pool.bytes()
    }

    /// Idle encoded output capacity; live returned frames remain separately owned.
    pub fn pooled_output_bytes(&self) -> u64 {
        self.output_pool.bytes()
    }

    /// Idle native GPU readback storage, separate from active frame payload totals.
    #[cfg(feature = "gpu")]
    pub fn pooled_gpu_readback_bytes(&self) -> u64 {
        self.gpu
            .as_ref()
            .map_or(0, crate::gpu::Gpu::pooled_readback_bytes)
    }

    /// Enables optional stage wall-clock observation; disabled by default.
    pub fn enable_timings(&mut self, enabled: bool) {
        self.timings = enabled.then(FrameTimings::default);
        #[cfg(feature = "gpu")]
        if let Some(gpu) = &mut self.gpu {
            gpu.enable_timings(enabled);
        }
    }

    /// Latest successful memory-output totals; None before a request or after failure.
    /// External-texture requests do not populate this memory-conversion report.
    pub fn last_stats(&self) -> Option<&FrameStats> {
        self.last_stats.as_ref()
    }

    /// Latest opt-in CPU stage observations, including partial failed attempts.
    pub fn timings(&self) -> Option<FrameTimings> {
        self.timings
    }

    /// Resource failures belonging only to the latest request.
    pub fn asset_diagnostics(&self) -> &[std::sync::Arc<motion_core::Error>] {
        &self.asset_diagnostics
    }

    /// Explicit diagnostic sink for the latest request and caller-managed background tasks.
    /// Clone it and use Handle::get_with on other threads; rendering does not start
    /// background resource tasks automatically. Old clones retain the old request's sink.
    pub fn diagnostics(&self) -> &motion_core::RequestDiagnostics {
        &self.diagnostics
    }
}
