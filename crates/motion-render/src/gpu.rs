//! Native GPU consumption of shared prepared coverage, without implicit layers.

use crate::prepared::*;
use crate::raster::prepare::pixel_bounds;
use motion_core::budget::Budget;
use motion_core::drawing::BlendMode;
use motion_core::{PixelSize, ValidationError};
use std::{mem::size_of, ops::Range};

mod abi;
mod context;
mod device;
mod execute;
mod layout;
mod output_plan;
mod particle_tiles;
mod plan;
#[cfg(test)]
mod target_tests;
#[cfg(test)]
mod tests;
mod timing;

pub use device::*;
pub use output_plan::OutputPlan;
pub use timing::{GpuTimingHandle, GpuTimings};

pub(crate) use abi::output_shader;
use abi::{
    FrameHeader, GpuOp, GpuUniforms, OperationHeader, PaintKind, SourceFlags, UniformRect,
    shader_constants,
};

use particle_tiles::ParticleTiles;

const DIRECT_COVERAGE: u32 = u32::MAX - 2;

/// Clips with more strips and spans than this use dense coverage, because a pixel that is
/// not drawn through its own record scans every direct record.
const DIRECT_CLIP_RECORDS: usize = 16;

struct Step {
    first: usize,
    second: usize,
    parameters: GpuUniforms,
    data: Vec<f32>,
    quads: Vec<[f32; 4]>,
}

const SPARSE_COVERAGE: u32 = u32::MAX;

const CONDITIONAL_COVERAGE: u32 = u32::MAX - 1;

struct Layer {
    image: usize,
    bounds: PixelRect,
    opacity: f64,
    mode: BlendMode,
    clip: Option<usize>,
    geometric: Option<usize>,
    mask: Option<motion_core::MaskMode>,
    filters: Option<usize>,
}

/// Device-independent ordered GPU operations with static raster admission.
/// Particle index bytes/work are admitted from a GPU prefix summary before composition.
/// Constructing a plan does not initialize a device or submit work.
pub(crate) struct Plan {
    size: PixelSize,
    output: usize,
    steps: Vec<Step>,
    coverage: Vec<u32>,
    last_uses: Vec<usize>,
    texture_slots: Vec<usize>,
    texture_sizes: Vec<PixelSize>,
    compute_texture_slots: Vec<usize>,
    compute_texture_sizes: Vec<PixelSize>,
    texture_blends: Vec<bool>,
    texture_bounded: Vec<bool>,
    texture_data_sizes: Vec<u64>,

    display: [u32; 32],
    lut_data: Vec<f32>,
    packed_output: Option<(PixelSize, crate::PixelFormat)>,

    limits: RasterLimits,

    /// Static logical charges; returned linear-frame stats also include dynamic tile charges.
    pub stats: WorkStats,
}

struct Planner<'a> {
    frame: &'a PreparedFrame,
    steps: Vec<Step>,

    budget: Budget,
}

fn image_size(bounds: PixelRect) -> PixelSize {
    PixelSize::new(
        (bounds.right - bounds.left).max(1),
        (bounds.bottom - bounds.top).max(1),
    )
}

fn image_bytes(bounds: PixelRect) -> u64 {
    let size = image_size(bounds);
    u64::from(size.width) * u64::from(size.height) * 16
}

/// Shared native device/queue/pipeline. Clones reference the same GPU and serialize
/// error-scoped submissions; no hidden global device or per-frame device creation.
#[derive(Clone)]
pub struct GpuContext {
    device: wgpu::Device,
    loss: std::sync::Arc<crate::gpu::DeviceMonitor>,
    queue: wgpu::Queue,
    info: crate::GpuInfo,
    submissions: std::sync::Arc<std::sync::Mutex<()>>,

    particle_states: std::sync::Arc<wgpu::ComputePipeline>,
    particle_bins: [std::sync::Arc<wgpu::ComputePipeline>; 3],
    textures: std::sync::Arc<TexturePipelines>,
    present_shader: std::sync::Arc<std::sync::OnceLock<wgpu::ShaderModule>>,
    present: std::sync::Arc<[std::sync::OnceLock<wgpu::RenderPipeline>; 3]>,
    present_check: std::sync::Arc<std::sync::OnceLock<wgpu::ComputePipeline>>,
    packing: std::sync::Arc<std::sync::OnceLock<wgpu::ComputePipeline>>,
    timestamp_marker: std::sync::Arc<std::sync::OnceLock<wgpu::ComputePipeline>>,
    alpha_thresholds: std::sync::Arc<std::sync::OnceLock<wgpu::Buffer>>,

    working_pool: std::sync::Arc<std::sync::Mutex<WorkingResourcePool>>,

    self_check_result: std::sync::Arc<std::sync::OnceLock<Result<(), String>>>,

    #[cfg(test)]
    fill_batches_encoded: std::sync::Arc<std::sync::atomic::AtomicUsize>,
    #[cfg(test)]
    batch_resolves_encoded: std::sync::Arc<std::sync::atomic::AtomicUsize>,
    #[cfg(test)]
    layer_captures_encoded: std::sync::Arc<std::sync::atomic::AtomicUsize>,
    #[cfg(test)]
    working_textures_created: std::sync::Arc<std::sync::atomic::AtomicUsize>,
    #[cfg(test)]
    color_image_buffers_encoded: std::sync::Arc<std::sync::atomic::AtomicUsize>,
    #[cfg(test)]
    hardware_plus_encoded: std::sync::Arc<std::sync::atomic::AtomicUsize>,
    #[cfg(test)]
    batch_data_buffers_encoded: std::sync::Arc<std::sync::atomic::AtomicU64>,
    #[cfg(test)]
    batch_data_copies_encoded: std::sync::Arc<std::sync::atomic::AtomicU64>,
    #[cfg(test)]
    batch_data_bytes_encoded: std::sync::Arc<std::sync::atomic::AtomicU64>,
    #[cfg(test)]
    batch_bind_groups_encoded: std::sync::Arc<std::sync::atomic::AtomicU64>,
}

/// Serialized execution session sharing a device, queue and pipelines.
pub struct Gpu {
    context: GpuContext,
    readback: Option<(u64, wgpu::Buffer, wgpu::Buffer)>,
    packed_output: Option<(u64, wgpu::Buffer)>,
    timing: Option<GpuTimingHandle>,
    encode_time: std::time::Duration,
    readback_time: std::time::Duration,
}

impl std::ops::Deref for Gpu {
    type Target = GpuContext;

    fn deref(&self) -> &GpuContext {
        &self.context
    }
}

#[derive(Clone)]
struct TexturePipelines {
    compute: wgpu::ComputePipeline,
    particles: wgpu::ComputePipeline,
    check: wgpu::ComputePipeline,
    backdrop: wgpu::RenderPipeline,
    blends: Option<[wgpu::RenderPipeline; 2]>,
    resolve: wgpu::ComputePipeline,
}

type ScopedError = std::pin::Pin<Box<dyn std::future::Future<Output = Option<wgpu::Error>>>>;

/// Submitted same-device texture rendering. Finish checks status and scoped errors
/// before publication; resources stay leased until its own submission completes.
pub struct PendingTextureFrame {
    context: GpuContext,
    submission: wgpu::SubmissionIndex,
    readback: wgpu::Buffer,
    completion: std::sync::mpsc::Receiver<Result<(), wgpu::BufferAsyncError>>,
    mapped: bool,
    validation: ScopedError,
    memory: ScopedError,
    working: WorkingLease,
    maximum: u64,
    timestamps: Option<timing::TimestampRecorder>,
}

/// Submitted encoded GPU output with independent scope and mapping completion.
pub(crate) struct PendingOutputFrame {
    context: GpuContext,
    submission: wgpu::SubmissionIndex,
    readback: wgpu::Buffer,
    packed_output: wgpu::Buffer,
    status: wgpu::Buffer,
    completion: std::sync::mpsc::Receiver<Result<(), wgpu::BufferAsyncError>>,
    validation: ScopedError,
    memory: ScopedError,
    size: PixelSize,
    format: crate::PixelFormat,
    limits: RasterLimits,
    working: WorkingLease,
    timestamps: Option<timing::TimestampRecorder>,
}

/// Submitted linear frame with independently owned readback and error scopes.
/// Dropping discards its output; finish waits for completion on the shared device.
pub(crate) struct PendingLinearFrame {
    context: GpuContext,
    submission: wgpu::SubmissionIndex,
    readback: wgpu::Buffer,
    status: wgpu::Buffer,
    completion: std::sync::mpsc::Receiver<Result<(), wgpu::BufferAsyncError>>,
    validation: ScopedError,
    memory: ScopedError,
    raster_width: u32,
    output_bounds: PixelRect,
    stats: WorkStats,
    working: WorkingLease,
    cache_limit: u64,
    timestamps: Option<timing::TimestampRecorder>,
}

#[derive(Default)]
struct WorkingResourcePool {
    buffers: Vec<wgpu::Buffer>,
    staging: Vec<StagingBuffer>,
    textures: Vec<wgpu::Texture>,
    bytes: u64,
}

struct StagingBuffer {
    buffer: wgpu::Buffer,
    mapped: std::sync::Arc<std::sync::OnceLock<bool>>,
}

#[derive(Default)]
struct WorkingLease {
    buffers: Vec<wgpu::Buffer>,
    textures: Vec<wgpu::Texture>,
}

impl std::ops::Deref for WorkingLease {
    type Target = Vec<wgpu::Buffer>;

    fn deref(&self) -> &Self::Target {
        &self.buffers
    }
}

impl std::ops::DerefMut for WorkingLease {
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.buffers
    }
}

impl WorkingResourcePool {
    fn trim(&mut self, maximum: u64) {
        while self.bytes > maximum
            || self.buffers.len() + self.staging.len() > 512
            || self.textures.len() > 32
        {
            if let Some(buffer) = self.buffers.pop() {
                self.bytes -= buffer.size();
            } else if let Some(staging) = self.staging.pop() {
                self.bytes -= staging.buffer.size();
            } else if let Some(texture) = self.textures.pop() {
                self.bytes -= u64::from(texture.width()) * u64::from(texture.height()) * 16;
            } else {
                break;
            }
        }
    }
}

/// Up to three submitted frames with aggregate logical admission and ordered completion.
/// Retains the synchronous device policy; no encoder or destination is committed here.
pub struct FrameQueue {
    renderer: Gpu,
    pending: std::collections::VecDeque<(u64, PendingLinearFrame)>,
    packed: std::collections::VecDeque<(u64, PendingOutputFrame)>,
    bytes: u64,
    maximum: u64,
}

fn device_error(field: &str, message: impl Into<String>) -> ValidationError {
    ValidationError::unsupported("gpu", field, message)
}
