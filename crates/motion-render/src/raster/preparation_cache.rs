//! Caches a session reuses across frames, split by what each backend may share.

use super::CoverageCache;
use super::paint::TableCache;

/// Session-lifetime preparation caches.
///
/// Paint tables are backend-neutral. Coverage holds CPU strips and spans, which the GPU only
/// consults for analytic fills it keeps on the CPU (rectangles and decal patterns).
pub(crate) struct PreparationCache {
    pub(crate) coverage: CoverageCache,
    pub(crate) paints: TableCache,
}

impl Default for PreparationCache {
    fn default() -> Self {
        Self::new(&motion_core::RenderCache::default())
    }
}

impl PreparationCache {
    pub(crate) fn new(cache: &motion_core::RenderCache) -> Self {
        Self {
            coverage: CoverageCache::new(cache),
            paints: TableCache::new(cache),
        }
    }

    pub(crate) fn retained_coverage_bytes(&self) -> u64 {
        self.coverage.retained_bytes()
    }

    pub(crate) fn retained_paint_bytes(&self) -> u64 {
        self.paints.retained_bytes()
    }
}
