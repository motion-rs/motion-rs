//! Project-local decoded resource accounting and identity.

use super::*;

/// Byte and entry allowance for a [`ResourceStore`].
///
/// This is independent of the compute-cache [`crate::CacheLimits`]: a store
/// retains decoded resources, while caches retain recomputable values.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ResourceLimits {
    /// Logical decoded payload allowance; must be positive.
    pub max_bytes: usize,
    /// Cached-entry allowance, in `1..=4096`.
    pub max_entries: usize,
}

impl Default for ResourceLimits {
    fn default() -> Self {
        Self {
            max_bytes: MAX_STORE_BYTES,
            max_entries: MAX_ENTRIES,
        }
    }
}

/// Explicit project resource store and logical allowance; clones share identity and accounting.
/// Each new Assets owner starts isolated unless with_store supplies a shared store.
/// Decoded Arc values can outlive all owners/handles and are not then store-accounted.
///
/// It holds decoded resources that are loaded on demand, unlike the compute caches
/// ([`crate::SceneCache`], [`crate::RenderCache`]) that retain recomputable values.
#[derive(Clone, Default)]
pub struct ResourceStore(pub(super) Arc<Store>);

pub(super) struct Store {
    pub(super) bytes: AtomicUsize,
    pub(super) entries: Mutex<BTreeMap<CacheKey, Weak<Erased>>>,
    pub(super) limits: ResourceLimits,
}

impl Default for Store {
    fn default() -> Self {
        Self {
            bytes: AtomicUsize::new(0),
            entries: Mutex::new(BTreeMap::new()),
            limits: ResourceLimits::default(),
        }
    }
}

impl ResourceStore {
    /// Creates an isolated 256 MiB / 4096-entry logical project store.
    pub fn new() -> Self {
        Self::default()
    }

    /// Uses a positive payload allowance and 1..=4096 cached entries.
    pub fn with_limits(limits: ResourceLimits) -> crate::Result<Self> {
        if limits.max_bytes == 0 || !(1..=MAX_ENTRIES).contains(&limits.max_entries) {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "Resource limits require positive bytes and 1..=4096 entries",
            )
            .into());
        }
        Ok(Self(Arc::new(Store {
            limits,
            ..Store::default()
        })))
    }

    /// Configured logical payload allowance.
    pub fn max_bytes(&self) -> usize {
        self.0.limits.max_bytes
    }

    /// Configured cached-entry allowance.
    pub fn max_entries(&self) -> usize {
        self.0.limits.max_entries
    }

    /// Logical payload retained by this project, not process RSS.
    pub fn retained_bytes(&self) -> usize {
        self.0.bytes.load(Ordering::Acquire)
    }

    /// Whether two owners charge the same project store.
    pub fn same_store(&self, other: &Self) -> bool {
        Arc::ptr_eq(&self.0, &other.0)
    }
}
