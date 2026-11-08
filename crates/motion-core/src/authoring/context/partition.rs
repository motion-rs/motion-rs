//! Rendering-cache partitions and the entry and byte limits shared by cache constructors.

use std::sync::Arc;

use super::SceneCache;

/// Independently bounded pure rendering-cache domains.
#[derive(Clone, Copy, Debug)]
#[repr(usize)]
pub enum CachePartition {
    /// Device-space coverage keyed by geometry, transform and tolerance.
    Coverage,
    /// Shaped paragraph layouts keyed by text and font environment.
    Paragraph,
    /// Font-unit outlines keyed by immutable font identity and glyph.
    Glyph,
    /// Linear color lookup tables keyed by gradient content.
    Gradient,
    /// Compiled formula content keyed by source and compiler configuration.
    Formula,
    /// Full-path preview ink atlases keyed by material parameters and device density.
    Ink,
    /// Explicit pure author computations shared by render workers and setting changes.
    Scene,
}

/// Cloneable rendering cache shared by sessions and export workers.
/// Each partition has its own entry and logical-byte allowance; external Arc pins
/// and allocator overhead are not included. Computation runs outside cache locks.
#[derive(Clone, Debug)]
pub struct RenderCache {
    partitions: [Arc<SceneCache>; 7],
}

impl Default for RenderCache {
    fn default() -> Self {
        Self {
            partitions: std::array::from_fn(|index| {
                let (entries, bytes) = match index {
                    0 => (2048, 16 * 1024 * 1024),
                    1 => (256, 8 * 1024 * 1024),
                    2 => (2048, 16 * 1024 * 1024),
                    3 => (64, 8 * 1024 * 1024),
                    5 => (64, 32 * 1024 * 1024),
                    6 => (256, 8 * 1024 * 1024),
                    _ => (32, 32 * 1024 * 1024),
                };
                Arc::new(SceneCache::with_limits(CacheLimits { entries, bytes }))
            }),
        }
    }
}

/// Entry-count and payload-byte allowance shared by the cache constructors.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct CacheLimits {
    /// Maximum retained entries; zero disables caching where the cache allows it.
    pub entries: usize,
    /// Caller-declared retained payload bytes.
    pub bytes: usize,
}

impl RenderCache {
    /// Creates independent LRU partitions with the supplied per-partition limits.
    pub fn with_limits(limits: CacheLimits) -> Self {
        Self {
            partitions: std::array::from_fn(|_| Arc::new(SceneCache::with_limits(limits))),
        }
    }

    /// Shared storage for one domain. Costed entries must include owned key payloads.
    pub fn partition(&self, partition: CachePartition) -> &Arc<SceneCache> {
        &self.partitions[partition as usize]
    }

    /// Isolates one domain while sharing the other partitions and preserving limits.
    pub fn fork_partition(&self, partition: CachePartition) -> Self {
        let mut cache = self.clone();
        let source = self.partition(partition);
        cache.partitions[partition as usize] = Arc::new(SceneCache::with_limits(CacheLimits {
            entries: source.capacity(),
            bytes: source.max_bytes(),
        }));
        cache
    }

    /// Releases all retained entries without invalidating outstanding value handles.
    pub fn clear(&self) {
        for partition in &self.partitions {
            partition.clear();
        }
    }
}
