//! Explicit CPU/GPU consumption without implicit font discovery.

use super::*;
use crate::{LinearRenderStats, OutputStats, PreparationStats, Snapshot};
use motion_core::{Color, Node, Result, SceneContext, Time};

/// Opt-in host stage observations and separately identified device timestamps.
/// Native texture completion is timed separately by its consumer. These values
/// do not alter budgets and are not performance acceptance measurements.
#[derive(Clone, Copy, Debug, Default)]
pub struct FrameTimings {
    /// Scene evaluation, including caller-side debugging work.
    pub evaluation: std::time::Duration,
    /// Bounded node layout and validated command compilation.
    pub compile: std::time::Duration,
    /// Coverage and paint preparation.
    pub coverage: std::time::Duration,
    /// CPU planning of native GPU instructions.
    pub plan: std::time::Duration,
    /// CPU command encoding and submission, excluding completion waits.
    pub encode: std::time::Duration,
    /// Standalone CPU linear composition; zero when fused with output conversion.
    pub composition: std::time::Duration,
    /// Memory output conversion, or combined CPU composition/conversion when fused.
    pub output: std::time::Duration,
    /// Whether output includes CPU composition rather than a separable conversion stage.
    pub fused_output: bool,
    /// CPU completion wait, mapping and output extraction, not device execution time.
    pub readback: std::time::Duration,
    /// CPU assembly of the opaque RGB snapshot after output conversion.
    pub snapshot: std::time::Duration,
    /// Consumer-side CPU wall time spent enqueueing a texture upload.
    pub upload: std::time::Duration,
    /// Consumer-side CPU wall time of native texture preparation and submission.
    pub submission: std::time::Duration,
    /// Actual device-stage timestamps; None when disabled, unsupported or not completed.
    #[cfg(feature = "gpu")]
    pub gpu: Option<crate::gpu::GpuTimings>,
}

/// Logical pipeline totals for one successfully completed frame.
///
/// Preparation/rendering peaks include retained prepared storage. CPU conversion
/// runs after prepared storage is dropped; GPU conversion includes its working
/// and packed output resources. Snapshot packing runs after linear storage is
/// dropped. Source nodes/lists, bounded scene/coverage caches, allocator capacity and overhead are
/// outside these logical totals, which are not a process memory cap.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct FrameStats {
    /// Coverage preparation, including filter metadata and scratch.
    pub preparation: PreparationStats,
    /// Linear-light composition, including the retained prepared frame.
    pub linear: LinearRenderStats,
    /// SDR sRGB RGBA8 conversion, including its CPU linear or GPU working resources.
    pub output: OutputStats,
    /// Opaque RGB snapshot packing, including the source RGBA8 frame.
    pub snapshot: OutputStats,
}

impl FrameStats {
    /// Largest live logical byte total across the sequential pipeline stages.
    pub fn peak_bytes(&self) -> u64 {
        self.preparation
            .peak_bytes
            .max(self.linear.peak_bytes)
            .max(self.output.peak_bytes)
            .max(self.snapshot.peak_bytes)
    }

    /// Total admitted work across preparation, composition, conversion and packing.
    pub fn work(&self) -> u64 {
        self.preparation
            .work
            .saturating_add(self.linear.work)
            .saturating_add(self.output.work)
            .saturating_add(self.snapshot.work)
    }
}

/// Composition-timed wrapper around a reusable RenderSession.
///
/// Created explicitly by Composition::frame_renderer. Evaluation receives
/// signed-tick SceneContext inputs and a session-owned entry-bounded scene cache.
/// Also exposes typed memory and external-texture output. No implicit font discovery
/// or encoding occurs. Backend/device work is deferred until execution; explicit
/// GPU selection never silently falls back, while Automatic may choose CPU at startup.
pub struct FrameRenderer {
    pub(super) composition: Composition,
    pub(super) duration: Time,
    pub(super) session: RenderSession,
}

impl std::ops::Deref for FrameRenderer {
    type Target = RenderSession;

    fn deref(&self) -> &RenderSession {
        &self.session
    }
}

impl std::ops::DerefMut for FrameRenderer {
    fn deref_mut(&mut self) -> &mut RenderSession {
        &mut self.session
    }
}

impl FrameRenderer {
    fn execution(&mut self) -> FrameExecution<'_> {
        FrameExecution {
            composition: self.composition.clone(),
            duration: self.duration,
            time: None,
            limits: self.session.limits,
            session: &mut self.session,
        }
    }

    /// Renders an opaque frame using this composition's exact clock.
    pub fn render_frame(
        &mut self,
        index: u32,
        build: impl FnOnce(&SceneContext<'_>) -> Node,
    ) -> Result<Snapshot> {
        self.execution().render_frame(index, build)
    }

    /// Renders a repeatable frame including configured shutter samples.
    pub fn render_frame_sampled(
        &mut self,
        index: u32,
        build: impl FnMut(&SceneContext<'_>) -> Node,
    ) -> Result<Snapshot> {
        self.execution().render_frame_sampled(index, build)
    }

    /// Prepares exact ticks, shutter and resources without selecting or submitting a GPU.
    /// Returned remaining work allowance excludes the preparation already performed.
    pub fn prepare_output_at(
        &mut self,
        time: Time,
        background: Option<Color>,
        build: impl FnMut(&SceneContext<'_>) -> Node,
    ) -> Result<(crate::PreparedFrame, RasterLimits)> {
        self.execution().prepare_output_at(time, background, build)
    }

    /// Renders exact ticks to owned memory through the shared request executor.
    pub fn render_output_at(
        &mut self,
        time: Time,
        background: Option<Color>,
        format: crate::PixelFormat,
        build: impl FnMut(&SceneContext<'_>) -> Node,
    ) -> Result<crate::FrameBuffer> {
        let rate = self.composition.clock.rate;
        let mut output = crate::FrameBuffer::vacant();
        self.session.render(
            FrameRequest {
                at: FrameAt::Time(time),
                frame_rate: rate,
                duration: self.duration.since_origin(),
                background,
                target: crate::RenderTarget::Memory(&mut output),
                format,
            },
            build,
        )?;
        Ok(output)
    }

    /// Renders exact composition frames through the same request executor.
    #[cfg(feature = "gpu")]
    pub fn render_to_texture(
        &mut self,
        index: u32,
        background: Option<Color>,
        texture: &wgpu::Texture,
        format: crate::PixelFormat,
        build: impl FnMut(&SceneContext<'_>) -> Node,
    ) -> Result<()> {
        self.last_stats = None;
        self.asset_diagnostics.clear();
        self.composition.validate_frame_index(index)?;
        let rate = self.composition.clock.rate;
        self.session.render(
            FrameRequest {
                at: FrameAt::Index(index),
                frame_rate: rate,
                duration: self.duration.since_origin(),
                background,
                target: crate::RenderTarget::Texture(texture),
                format,
            },
            build,
        )
    }

    /// Shared context after startup selection, if GPU was selected.
    ///
    /// Submits exact composition frames without waiting for GPU completion.
    /// Finish the returned handle before publishing or reusing the target texture.
    #[cfg(feature = "gpu")]
    pub fn submit_to_texture(
        &mut self,
        index: u32,
        background: Option<Color>,
        texture: &wgpu::Texture,
        format: crate::PixelFormat,
        build: impl FnMut(&SceneContext<'_>) -> Node,
    ) -> Result<crate::gpu::PendingTextureFrame> {
        self.last_stats = None;
        self.asset_diagnostics.clear();
        self.composition.validate_frame_index(index)?;
        self.execution()
            .submit_to_texture(index, background, texture, format, build)
    }

    /// Shared context after startup selection, if GPU was selected.
    #[cfg(feature = "gpu")]
    pub fn selected_gpu_context(&mut self) -> Result<Option<crate::GpuContext>> {
        self.execution().selected_gpu_context()
    }

    /// Configured backend policy, without selecting or initializing a device.
    /// Automatic remains Automatic after its startup decision has been made.
    pub fn requested_backend(&self) -> crate::BackendPreference {
        self.composition.options.backend
    }
}

#[cfg(test)]
mod tests;
