//! Project-local decoded resource accounting and identity.

use super::*;

/// Explicit project cache and logical allowance; clones share identity and accounting.
/// Each new Assets owner starts isolated unless with_pool supplies a shared pool.
/// Decoded Arc values can outlive all owners/handles and are not then pool-accounted.
#[derive(Clone, Default)]
pub struct ResourcePool(pub(super) Arc<Pool>);

pub(super) struct Pool {
    pub(super) bytes: AtomicUsize,
    pub(super) entries: Mutex<BTreeMap<CacheKey, Weak<Erased>>>,
    pub(super) max_bytes: usize,
    pub(super) max_entries: usize,
}

impl Default for Pool {
    fn default() -> Self {
        Self {
            bytes: AtomicUsize::new(0),
            entries: Mutex::new(BTreeMap::new()),
            max_bytes: MAX_CACHE_BYTES,
            max_entries: MAX_ENTRIES,
        }
    }
}

impl ResourcePool {
    /// Creates an isolated 256 MiB / 4096-entry logical project pool.
    pub fn new() -> Self {
        Self::default()
    }

    /// Uses a positive payload allowance and 1..=4096 cached entries.
    pub fn with_limits(limits: crate::CacheLimits) -> crate::Result<Self> {
        let (max_bytes, max_entries) = (limits.bytes, limits.entries);
        if max_bytes == 0 || !(1..=MAX_ENTRIES).contains(&max_entries) {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "Resource limits require positive bytes and 1..=4096 entries",
            )
            .into());
        }
        Ok(Self(Arc::new(Pool {
            max_bytes,
            max_entries,
            ..Pool::default()
        })))
    }

    /// Configured logical payload allowance.
    pub fn max_bytes(&self) -> usize {
        self.0.max_bytes
    }

    /// Configured cached-entry allowance.
    pub fn max_entries(&self) -> usize {
        self.0.max_entries
    }

    /// Logical payload retained by this project, not process RSS.
    pub fn retained_bytes(&self) -> usize {
        self.0.bytes.load(Ordering::Acquire)
    }

    /// Whether two owners charge the same project pool.
    pub fn same_pool(&self, other: &Self) -> bool {
        Arc::ptr_eq(&self.0, &other.0)
    }
}
