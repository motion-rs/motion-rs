//! Bounded device-local immutable resource buffers shared across frames and contexts.

use super::{GpuContext, Planner};
use motion_core::sync::MutexExt;
use motion_core::{CacheLimits, ValidationError};
use std::collections::VecDeque;
use std::mem::size_of;
use std::sync::Arc;

#[cfg(test)]
mod frame_tests;
#[cfg(test)]
mod sprite_tests;

/// Default retained immutable resource entries per device.
const DEFAULT_RESOURCE_ENTRIES: usize = 64;

/// Default combined host-key and device-payload residency allowance in bytes.
const DEFAULT_RESOURCE_BYTES: usize = 64 * 1024 * 1024;

/// Maximum configured entry count, bounding lookup and eviction work.
const MAX_RESOURCE_ENTRIES: usize = 4096;

/// Maximum equal-hash resources retained, bounding exact collision comparisons.
const MAX_HASH_MATCHES: usize = 4;

/// Worst-case hashing, collision comparison and upload work per resource word.
const RESOURCE_WORD_WORK: u64 = MAX_HASH_MATCHES as u64 + 2;

/// Lookup, LRU relocation and eviction work per configured entry.
const RESOURCE_ENTRY_WORK: u64 = 3;

/// Vector owner and Arc counters in addition to the already admitted resource words.
const RESOURCE_OWNER_BYTES: u64 = (size_of::<Vec<f32>>() + 2 * size_of::<usize>()) as u64;

/// Device storage plus upload staging for each already admitted host word.
const RESOURCE_UPLOAD_BYTES_PER_WORD: u64 = 2 * size_of::<f32>() as u64;

/// Fixed FNV-1a offset basis for immutable encoded resource words.
const RESOURCE_HASH_OFFSET: u64 = 0xcbf29ce484222325;

/// Fixed FNV-1a prime; hash collisions still require exact bitwise equality.
const RESOURCE_HASH_PRIME: u64 = 0x100000001b3;

/// Cumulative observations of the shared device's immutable resource cache.
///
/// In-flight buffer leases survive eviction and are admitted by their frame,
/// outside the cache's idle-retention allowance. These are logical payload totals,
/// not driver allocations or a process memory cap.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct GpuResidencyStats {
    /// Resource requests satisfied without uploading immutable words again.
    pub hits: u64,
    /// Resource requests that uploaded immutable words.
    pub misses: u64,
    /// Device bytes uploaded on misses, including deliberately uncached resources.
    pub upload_bytes: u64,
    /// Entries removed by LRU eviction, explicit clearing or limit changes.
    pub evictions: u64,
    /// Requests not retained because of limits or the bounded hash-collision bucket.
    pub bypasses: u64,
    /// Currently retained immutable resources.
    pub entries: u64,
    /// Host comparison words, device payload and entry records currently retained.
    pub retained_bytes: u64,
}

#[derive(Clone)]
pub(in crate::gpu) struct ResidentResource {
    hash: u64,
    pub(in crate::gpu) words: Arc<Vec<f32>>,
}

impl ResidentResource {
    fn new(words: Vec<f32>) -> Self {
        let hash = words.iter().fold(RESOURCE_HASH_OFFSET, |hash, word| {
            (hash ^ u64::from(word.to_bits())).wrapping_mul(RESOURCE_HASH_PRIME)
        });
        Self {
            hash,
            words: Arc::new(words),
        }
    }

    fn same_content(&self, other: &Self) -> bool {
        self.hash == other.hash
            && self.words.len() == other.words.len()
            && self
                .words
                .iter()
                .zip(other.words.iter())
                .all(|(first, second)| first.to_bits() == second.to_bits())
    }
}

pub(in crate::gpu) struct ResidentStore {
    limits: CacheLimits,
    entries: VecDeque<ResidentEntry>,
    stats: GpuResidencyStats,
}

impl Default for ResidentStore {
    fn default() -> Self {
        Self {
            limits: CacheLimits {
                max_entries: DEFAULT_RESOURCE_ENTRIES,
                max_bytes: DEFAULT_RESOURCE_BYTES,
            },
            entries: VecDeque::new(),
            stats: GpuResidencyStats::default(),
        }
    }
}

impl ResidentStore {
    fn evict(&mut self) -> Result<(), ValidationError> {
        let entry = self.entries.pop_front().ok_or_else(|| {
            ValidationError::invalid(
                "gpu.residency",
                "entries",
                "Cannot evict an empty resource cache",
            )
        })?;
        self.stats.retained_bytes -= entry.bytes;
        self.stats.entries -= 1;
        self.stats.evictions = self.stats.evictions.saturating_add(1);
        Ok(())
    }

    fn buffer(
        &mut self,
        context: &GpuContext,
        resource: &ResidentResource,
    ) -> Result<wgpu::Buffer, ValidationError> {
        let mut matches = 0;
        for index in 0..self.entries.len() {
            let entry = &self.entries[index];
            matches += usize::from(entry.resource.hash == resource.hash);
            if entry.resource.hash != resource.hash
                || entry.resource.words.len() != resource.words.len()
            {
                continue;
            }
            if entry.resource.same_content(resource) {
                let entry = self.entries.remove(index).ok_or_else(|| {
                    ValidationError::invalid(
                        "gpu.residency",
                        "entries",
                        "Resource cache index is out of range",
                    )
                })?;
                let buffer = entry.buffer.clone();
                self.entries.push_back(entry);
                self.stats.hits = self.stats.hits.saturating_add(1);
                return Ok(buffer);
            }
        }
        let payload = resource.words.len() as u64 * size_of::<f32>() as u64;
        let bytes = payload
            .checked_mul(2)
            .and_then(|bytes| {
                bytes.checked_add(size_of::<ResidentEntry>() as u64 + RESOURCE_OWNER_BYTES)
            })
            .ok_or_else(|| {
                ValidationError::limit(
                    "gpu.residency",
                    "max_bytes",
                    "Resource residency size overflow",
                )
            })?;
        let retain = self.limits.max_entries != 0
            && bytes <= self.limits.max_bytes as u64
            && matches < MAX_HASH_MATCHES;
        if retain {
            while self.entries.len() >= self.limits.max_entries
                || self.stats.retained_bytes > self.limits.max_bytes as u64 - bytes
            {
                self.evict()?;
            }
            self.entries.try_reserve(1).map_err(|_| {
                ValidationError::limit(
                    "gpu.residency",
                    "max_bytes",
                    "Could not reserve a resident resource entry",
                )
            })?;
        }
        let buffer = context.upload_buffer(
            "Immutable resident resource",
            bytemuck::cast_slice(resource.words.as_slice()),
            wgpu::BufferUsages::STORAGE,
        );
        self.stats.misses = self.stats.misses.saturating_add(1);
        self.stats.upload_bytes = self.stats.upload_bytes.saturating_add(payload);
        if retain {
            self.entries.push_back(ResidentEntry {
                resource: resource.clone(),
                buffer: buffer.clone(),
                bytes,
            });
            self.stats.entries += 1;
            self.stats.retained_bytes += bytes;
        } else {
            self.stats.bypasses = self.stats.bypasses.saturating_add(1);
        }
        Ok(buffer)
    }
}

impl GpuContext {
    /// Configures the shared immutable image/mip/atlas LRU cache.
    ///
    /// Defaults to 64 entries and 64 MiB of combined host comparison and device
    /// payload bytes. Zero entries or bytes disables retention. Entry counts
    /// above 4096 are rejected. Lowering limits evicts immediately; outstanding
    /// frame leases remain valid. No device or resource IO is performed.
    pub fn set_residency_limits(&self, limits: CacheLimits) -> motion_core::Result<()> {
        if limits.max_entries > MAX_RESOURCE_ENTRIES {
            return Err(ValidationError::invalid(
                "gpu.residency",
                "max_entries",
                "Residency accepts at most 4096 entries",
            )
            .into());
        }
        let mut store = self.residency.lock_unpoisoned();
        store.limits = limits;
        while !store.entries.is_empty()
            && (store.entries.len() > limits.max_entries
                || store.stats.retained_bytes > limits.max_bytes as u64)
        {
            store.evict()?;
        }
        Ok(())
    }

    /// Cumulative resource upload observations and current retained logical bytes.
    pub fn residency_stats(&self) -> GpuResidencyStats {
        self.residency.lock_unpoisoned().stats
    }

    /// Evicts cached resources without invalidating frames already using them.
    pub fn clear_resident_resources(&self) -> motion_core::Result<()> {
        let mut store = self.residency.lock_unpoisoned();
        while !store.entries.is_empty() {
            store.evict()?;
        }
        Ok(())
    }

    pub(in crate::gpu) fn resource_buffer(
        &self,
        resource: &ResidentResource,
    ) -> Result<wgpu::Buffer, ValidationError> {
        self.residency.lock_unpoisoned().buffer(self, resource)
    }
}

impl Planner<'_> {
    pub(in crate::gpu) fn attach_resource(
        &mut self,
        step: usize,
        words: Vec<f32>,
    ) -> Result<(), ValidationError> {
        if words.is_empty() {
            return Err(ValidationError::invalid(
                "gpu.resources",
                "words",
                "An immutable GPU resource cannot be empty",
            ));
        }
        let target = self
            .steps
            .get_mut(step.checked_sub(1).ok_or_else(|| {
                ValidationError::invalid("gpu.resources", "step", "Resource needs an existing step")
            })?)
            .ok_or_else(|| {
                ValidationError::invalid("gpu.resources", "step", "Resource step is out of range")
            })?;
        if target.resource.is_some() {
            return Err(ValidationError::invalid(
                "gpu.resources",
                "step",
                "A step already has an immutable resource",
            ));
        }
        // Host words were admitted by Planner::data; misses must also admit GPU and staging.
        let bytes = (words.len() as u64)
            .checked_mul(RESOURCE_UPLOAD_BYTES_PER_WORD)
            .and_then(|bytes| bytes.checked_add(RESOURCE_OWNER_BYTES))
            .ok_or_else(|| {
                ValidationError::limit(
                    "gpu.resources",
                    "max_bytes",
                    "Immutable resource upload size overflow",
                )
            })?;
        self.budget.bytes(bytes, "gpu.resources")?;
        self.budget.work(
            words.len() as u64 * RESOURCE_WORD_WORK
                + MAX_RESOURCE_ENTRIES as u64 * RESOURCE_ENTRY_WORK,
            "gpu.resources",
        )?;
        target.resource = Some(ResidentResource::new(words));
        Ok(())
    }
}

struct ResidentEntry {
    resource: ResidentResource,
    buffer: wgpu::Buffer,
    bytes: u64,
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::gpu::{FrameInput, Gpu, Plan};
    use crate::{RasterLimits, RenderPrepare};
    use motion_core::budget::Budget;
    use motion_core::{Color, Node, PixelSize, Rect, Shape, Size, ValidationLimits};

    #[test]
    fn resource_identity_is_exact_even_for_hash_collisions() {
        let first = ResidentResource::new(vec![0.0, 0.5, 1.0]);
        let mut other = ResidentResource::new(vec![0.0, 0.25, 1.0]);
        other.hash = first.hash;
        assert!(!first.same_content(&other));
        assert!(first.same_content(&ResidentResource::new(vec![0.0, 0.5, 1.0])));
        let mut signed_zero = ResidentResource::new(vec![-0.0, 0.5, 1.0]);
        signed_zero.hash = first.hash;
        assert!(!first.same_content(&signed_zero));
    }

    #[test]
    fn resource_admission_is_exact_and_independent_of_cache_state() -> motion_core::Result {
        let frame = Node::from(Rect::new(0.0, 0.0, 1.0, 1.0).fill(Color::WHITE))
            .compile(
                Size::new(1.0, 1.0),
                PixelSize::new(1, 1),
                ValidationLimits::default(),
            )?
            .prepare(ValidationLimits::default(), RasterLimits::default())?;
        let bytes = 4 * 12 + RESOURCE_OWNER_BYTES;
        let work = 4 + 4 * RESOURCE_WORD_WORK + MAX_RESOURCE_ENTRIES as u64 * RESOURCE_ENTRY_WORK;
        for (maximum_bytes, maximum_work, accepted) in [
            (bytes, work, true),
            (bytes - 1, work, false),
            (bytes, work - 1, false),
        ] {
            let mut planner = Planner {
                frame: frame.gpu_view(),
                steps: Plan::new(&frame, RasterLimits::default())?.steps,
                budget: Budget::new(frame.size(), RasterLimits::default())?,
            };
            let step = 1;
            planner.budget = Budget::new(
                frame.size(),
                RasterLimits {
                    max_bytes: maximum_bytes,
                    max_work: maximum_work,
                    ..RasterLimits::default()
                },
            )?;
            planner.budget.bytes(16, "gpu.data")?;
            planner.budget.work(4, "gpu.data")?;
            let words = vec![0.5; 4];
            assert_eq!(planner.attach_resource(step, words).is_ok(), accepted);
            assert_eq!(planner.steps[step - 1].resource.is_some(), accepted);
            if accepted {
                assert_eq!(planner.budget.peak_bytes(), bytes);
                assert_eq!(planner.budget.work_done(), work);
            }
        }
        Ok(())
    }

    #[test]
    #[ignore = "requires a native GPU; verifies immutable resource residency and in-flight eviction"]
    fn native_resources_reuse_content_evict_safely_and_allow_disabling() -> motion_core::Result {
        let gpu = Gpu::new()?;
        let context = &gpu.context;
        context.clear_resident_resources()?;
        let before = context.residency_stats();
        let first = ResidentResource::new(vec![0.25, 0.5, 0.75, 1.0]);
        let retained = context.resource_buffer(&first)?;
        assert_eq!(
            retained,
            context.resource_buffer(&ResidentResource::new(first.words.as_ref().clone()))?
        );
        assert_eq!(context.residency_stats().hits, before.hits + 1);
        assert_eq!(
            context.residency_stats().upload_bytes,
            before.upload_bytes + 16
        );
        let mut collision = ResidentResource::new(vec![0.75, 0.5, 0.25, 1.0]);
        collision.hash = first.hash;
        let distinct = context.resource_buffer(&collision)?;
        assert_ne!(retained, distinct);
        assert_eq!(context.residency_stats().entries, 2);
        context.set_residency_limits(CacheLimits {
            max_entries: 1,
            max_bytes: DEFAULT_RESOURCE_BYTES,
        })?;
        assert_eq!(context.residency_stats().entries, 1);
        assert_eq!(context.resource_buffer(&collision)?, distinct);
        assert_ne!(context.resource_buffer(&first)?, retained);
        assert_eq!(context.residency_stats().entries, 1);
        let before_clear = context.clone();
        {
            let _submission = context.acquire_submission()?;
            before_clear.clear_resident_resources()?;
        }
        assert_eq!(context.residency_stats().retained_bytes, 0);
        assert_eq!(read_resource(context, &retained)?, first.words.as_slice());
        context.set_residency_limits(CacheLimits {
            max_entries: 0,
            max_bytes: 0,
        })?;
        let first_uncached = context.resource_buffer(&first)?;
        assert_ne!(context.resource_buffer(&first)?, first_uncached);
        assert_eq!(context.residency_stats().entries, 0);
        assert_eq!(context.residency_stats().bypasses, before.bypasses + 2);
        assert!(
            context
                .set_residency_limits(CacheLimits {
                    max_entries: MAX_RESOURCE_ENTRIES + 1,
                    max_bytes: DEFAULT_RESOURCE_BYTES
                })
                .is_err()
        );
        Ok(())
    }

    #[test]
    #[ignore = "requires a native GPU; verifies byte limits and collision-bounded cache work"]
    fn native_resource_byte_boundaries_and_hash_buckets_are_bounded() -> motion_core::Result {
        let gpu = Gpu::new()?;
        let context = &gpu.context;
        context.clear_resident_resources()?;
        let first = ResidentResource::new(vec![0.25; 4]);
        let retained = context.resource_buffer(&first)?;
        let bytes = context.residency_stats().retained_bytes as usize;
        context.set_residency_limits(CacheLimits {
            max_entries: 8,
            max_bytes: bytes,
        })?;
        assert_eq!(context.resource_buffer(&first)?, retained);
        context.resource_buffer(&ResidentResource::new(vec![0.5; 4]))?;
        assert_eq!(context.residency_stats().entries, 1);
        assert_eq!(context.residency_stats().retained_bytes as usize, bytes);
        context.set_residency_limits(CacheLimits {
            max_entries: 8,
            max_bytes: bytes - 1,
        })?;
        assert_eq!(context.residency_stats().entries, 0);
        let before = context.residency_stats();
        context.resource_buffer(&first)?;
        assert_eq!(context.residency_stats().entries, 0);
        assert_eq!(context.residency_stats().bypasses, before.bypasses + 1);
        context.set_residency_limits(CacheLimits {
            max_entries: DEFAULT_RESOURCE_ENTRIES,
            max_bytes: DEFAULT_RESOURCE_BYTES,
        })?;
        let before = context.residency_stats();
        for index in 0..=MAX_HASH_MATCHES {
            let mut resource = ResidentResource::new(vec![index as f32 * 0.125; 4]);
            resource.hash = 0;
            context.resource_buffer(&resource)?;
        }
        assert_eq!(context.residency_stats().entries, MAX_HASH_MATCHES as u64);
        assert_eq!(context.residency_stats().bypasses, before.bypasses + 1);
        assert_eq!(
            context.residency_stats().misses,
            before.misses + MAX_HASH_MATCHES as u64 + 1
        );
        Ok(())
    }

    fn read_resource(
        context: &GpuContext,
        resource: &wgpu::Buffer,
    ) -> motion_core::Result<Vec<f32>> {
        let device = context.device();
        let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("Read retained resource"),
            source: wgpu::ShaderSource::Wgsl(std::borrow::Cow::Borrowed(
                "@group(0) @binding(0) var<storage, read> source: array<f32>;
                 @group(0) @binding(1) var<storage, read_write> output: array<f32>;
                 @compute @workgroup_size(1) fn main() {
                     for (var index = 0u; index < 4u; index += 1u) { output[index] = source[index]; }
                 }",
            )),
        });
        let pipeline = device.create_compute_pipeline(&wgpu::ComputePipelineDescriptor {
            label: Some("Read retained resource"),
            layout: None,
            module: &shader,
            entry_point: Some("main"),
            compilation_options: Default::default(),
            cache: None,
        });
        let output = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("Retained resource values"),
            size: 16,
            mapped_at_creation: false,
            usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_SRC,
        });
        let readback = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("Retained resource readback"),
            size: 16,
            mapped_at_creation: false,
            usage: wgpu::BufferUsages::MAP_READ | wgpu::BufferUsages::COPY_DST,
        });
        let group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("Read retained resource"),
            layout: &pipeline.get_bind_group_layout(0),
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: resource.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: output.as_entire_binding(),
                },
            ],
        });
        let mut encoder = device.create_command_encoder(&Default::default());
        {
            let mut pass = encoder.begin_compute_pass(&Default::default());
            pass.set_pipeline(&pipeline);
            pass.set_bind_group(0, &group, &[]);
            pass.dispatch_workgroups(1, 1, 1);
        }
        encoder.copy_buffer_to_buffer(&output, 0, &readback, 0, 16);
        context.queue().submit([encoder.finish()]);
        let (send, receive) = std::sync::mpsc::sync_channel(1);
        readback
            .slice(..)
            .map_async(wgpu::MapMode::Read, move |result| {
                send.send(result).unwrap();
            });
        device
            .poll(wgpu::PollType::wait_indefinitely())
            .map_err(|error| motion_core::Error::Gpu(Box::new(error)))?;
        receive
            .recv()
            .map_err(|error| motion_core::Error::Gpu(Box::new(error)))?
            .map_err(|error| motion_core::Error::Gpu(Box::new(error)))?;
        let words = bytemuck::cast_slice::<u8, f32>(
            &readback
                .slice(..)
                .get_mapped_range()
                .map_err(|error| motion_core::Error::Gpu(Box::new(error)))?,
        )
        .to_vec();
        readback.unmap();
        Ok(words)
    }
}
