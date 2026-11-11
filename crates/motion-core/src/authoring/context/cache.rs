//! Shared LRU storage for pure authoring computations with optional byte accounting.

use crate::sync::MutexExt;
use std::any::{Any, TypeId};
use std::collections::{VecDeque, hash_map::DefaultHasher};
use std::hash::{Hash, Hasher};
use std::panic::Location;
use std::sync::{Arc, Mutex};

use super::CacheLimits;

/// Shared LRU compute cache for pure authoring computations, with optional byte accounting.
///
/// Entries can be dropped and recomputed at any time; decoded resources that must keep their
/// identity live in a [`crate::ResourceStore`] instead.
///
/// Values must depend only on their keys and immutable resources. Resource
/// revisions belong in the key. The default retains 256 entries without a byte cap.
/// with_limits adds a caller-declared logical byte allowance for cache_with_cost.
/// Ordinary cache() is entry-bounded only and reports no retained-byte charge.
/// Outstanding Arc pins and allocator overhead are not bounded.
/// Concurrent misses may compute more than once; builders and cost callbacks run
/// outside the lock. Zero entries disable retention; zero bytes disable costed values.
/// Original keys are retained and compared by type and equality after hashing.
pub struct SceneCache {
    capacity: usize,
    max_bytes: usize,
    pub(super) entries: Mutex<VecDeque<CacheEntry>>,
}

impl std::fmt::Debug for SceneCache {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("SceneCache")
            .field("capacity", &self.capacity)
            .field("max_bytes", &self.max_bytes)
            .finish_non_exhaustive()
    }
}

pub(super) struct CacheEntry {
    key: CacheKey,
    input: Box<dyn Any + Send + Sync>,
    value: Arc<dyn Any + Send + Sync>,
    bytes: usize,
}

#[derive(PartialEq, Eq)]
struct CacheKey {
    location: &'static Location<'static>,
    value_type: TypeId,
    hash: u64,
}

impl CacheEntry {
    fn matches<Key: Eq + 'static>(&self, key: &CacheKey, input: &Key) -> bool {
        self.key == *key && self.input.downcast_ref::<Key>() == Some(input)
    }
}

impl Default for SceneCache {
    fn default() -> Self {
        Self::new(256)
    }
}

impl SceneCache {
    /// Creates a cache with at most this many retained entries; zero disables it.
    pub fn new(capacity: usize) -> Self {
        Self::with_limits(CacheLimits {
            max_entries: capacity,
            max_bytes: usize::MAX,
        })
    }

    /// Bounds entries and caller-declared retained payload bytes.
    /// Heap-owning keys and values need SceneContext::cache_with_cost for payload charges.
    /// Neither allocator overhead nor externally pinned Arc values are bounded.
    pub fn with_limits(limits: CacheLimits) -> Self {
        Self {
            capacity: limits.max_entries,
            max_bytes: limits.max_bytes,
            entries: Mutex::new(VecDeque::new()),
        }
    }

    /// Logical payload allowance; usize::MAX means no configured byte bound.
    pub fn max_bytes(&self) -> usize {
        self.max_bytes
    }

    /// Sum of the declared charges for currently retained entries.
    pub fn retained_bytes(&self) -> usize {
        self.entries
            .lock_unpoisoned()
            .iter()
            .map(|entry| entry.bytes)
            .sum()
    }

    /// Maximum number of retained entries, not a byte budget.
    pub fn capacity(&self) -> usize {
        self.capacity
    }

    /// Number of currently retained entries.
    pub fn len(&self) -> usize {
        self.entries.lock_unpoisoned().len()
    }

    /// Whether no computed values are currently retained.
    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    /// Removes retained entries without invalidating existing returned handles.
    pub fn clear(&self) {
        let entries = std::mem::take(&mut *self.entries.lock_unpoisoned());
        drop(entries);
    }

    #[track_caller]
    pub(super) fn get<Key: Eq + Hash + Send + Sync + 'static, Value: Send + Sync + 'static>(
        &self,
        key: Key,
        build: impl FnOnce() -> Value,
    ) -> Arc<Value> {
        match self.get_cached(
            key,
            || Ok::<_, std::convert::Infallible>(build()),
            |_| 0,
            false,
            Location::caller(),
        ) {
            Ok(value) => value,
            Err(never) => match never {},
        }
    }

    /// Like [`get`](Self::get) for a fallible builder; failures are returned and never cached.
    #[track_caller]
    pub(super) fn try_get<
        Key: Eq + Hash + Send + Sync + 'static,
        Value: Send + Sync + 'static,
        Error,
    >(
        &self,
        key: Key,
        build: impl FnOnce() -> Result<Value, Error>,
    ) -> Result<Arc<Value>, Error> {
        self.get_cached(key, build, |_| 0, false, Location::caller())
    }

    /// Gets a pure computed value with a caller-declared retained payload charge.
    #[track_caller]
    pub fn get_with_cost<Key: Eq + Hash + Send + Sync + 'static, Value: Send + Sync + 'static>(
        &self,
        key: Key,
        build: impl FnOnce() -> Value,
        cost: impl FnOnce(&Value) -> usize,
    ) -> Arc<Value> {
        match self.get_cached(
            key,
            || Ok::<_, std::convert::Infallible>(build()),
            cost,
            true,
            Location::caller(),
        ) {
            Ok(value) => value,
            Err(never) => match never {},
        }
    }

    fn content_location() -> &'static Location<'static> {
        Location::caller()
    }

    /// Looks up a renderer content key independently of the caller's source location.
    pub fn lookup<Key: Eq + Hash + Send + Sync + 'static, Value: Send + Sync + 'static>(
        &self,
        key: &Key,
    ) -> Option<Arc<Value>> {
        let mut hasher = DefaultHasher::new();
        key.hash(&mut hasher);
        self.find(
            &CacheKey {
                location: Self::content_location(),
                value_type: TypeId::of::<Value>(),
                hash: hasher.finish(),
            },
            key,
        )
    }

    /// Inserts checked pure content, converging concurrent inserts to the retained value.
    pub fn insert_with_cost<
        Key: Eq + Hash + Send + Sync + 'static,
        Value: Send + Sync + 'static,
    >(
        &self,
        key: Key,
        value: Value,
        cost: impl FnOnce(&Value) -> usize,
    ) -> Arc<Value> {
        match self.get_cached(
            key,
            || Ok::<_, std::convert::Infallible>(value),
            cost,
            true,
            Self::content_location(),
        ) {
            Ok(value) => value,
            Err(never) => match never {},
        }
    }

    #[track_caller]
    fn get_cached<Key: Eq + Hash + Send + Sync + 'static, Value: Send + Sync + 'static, Error>(
        &self,
        key: Key,
        build: impl FnOnce() -> Result<Value, Error>,
        cost: impl FnOnce(&Value) -> usize,
        accounted: bool,
        location: &'static Location<'static>,
    ) -> Result<Arc<Value>, Error> {
        if self.capacity == 0 || (accounted && self.max_bytes == 0) {
            return build().map(Arc::new);
        }
        let mut hasher = DefaultHasher::new();
        key.hash(&mut hasher);
        let cache_key = CacheKey {
            location,
            value_type: TypeId::of::<Value>(),
            hash: hasher.finish(),
        };
        if let Some(value) = self.find(&cache_key, &key) {
            return Ok(value);
        }
        let value = Arc::new(build()?);
        let bytes = if accounted {
            cost(value.as_ref())
                .max(std::mem::size_of::<Value>().saturating_add(std::mem::size_of::<Key>()))
        } else {
            0
        };
        if bytes > self.max_bytes {
            return Ok(value);
        }
        let mut entries = self.entries.lock_unpoisoned();
        if let Some(index) = entries
            .iter()
            .position(|entry| entry.matches(&cache_key, &key))
        {
            let entry = entries.remove(index).expect("located cache entry");
            let stored = entry.value.clone();
            entries.push_back(entry);
            drop(entries);
            return Ok(stored
                .downcast()
                .unwrap_or_else(|_| unreachable!("the cache key encodes the value type")));
        }
        let mut retained: usize = entries.iter().map(|entry| entry.bytes).sum();
        let mut evicted = Vec::new();
        while entries.len() >= self.capacity || bytes > self.max_bytes.saturating_sub(retained) {
            let Some(entry) = entries.pop_front() else {
                break;
            };
            retained -= entry.bytes;
            evicted.push(entry);
        }
        entries.push_back(CacheEntry {
            key: cache_key,
            input: Box::new(key),
            value: value.clone(),
            bytes,
        });
        drop(entries);
        drop(evicted);
        Ok(value)
    }

    fn find<Key: Eq + 'static, Value: Send + Sync + 'static>(
        &self,
        key: &CacheKey,
        input: &Key,
    ) -> Option<Arc<Value>> {
        let mut entries = self.entries.lock_unpoisoned();
        let index = entries.iter().position(|entry| entry.matches(key, input))?;
        let entry = entries.remove(index).expect("located cache entry");
        let value = entry.value.clone();
        entries.push_back(entry);
        drop(entries);
        Some(
            value
                .downcast()
                .unwrap_or_else(|_| unreachable!("the cache key encodes the value type")),
        )
    }
}
