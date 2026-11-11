//! Lazy typed assets with explicit bounded project resource stores and request diagnostics.
//!
//! ```
//! use motion_core::{Asset, Assets, LoadContext, Source};
//! use std::sync::Arc;
//! struct Label(String);
//! impl Asset for Label {
//!     const KIND: &'static str = "label";
//!     fn decode(bytes: &[u8], _: &LoadContext) -> motion_core::Result<Self> {
//!         Ok(Self(std::str::from_utf8(bytes)?.to_owned()))
//!     }
//!     fn retained_bytes(&self) -> usize {
//!         self.0.len()
//!     }
//! }
//! let assets = Assets::new("assets")?;
//! let handle = assets.load::<Label>(Source::Bytes {
//!     key: "title".into(),
//!     data: Arc::from(&b"Native resources"[..]),
//! });
//! assert!(handle.state().is_unloaded());
//! assert_eq!(handle.get()?.0, "Native resources");
//! # Ok::<(), motion_core::Error>(())
//! ```

use crate::sync::MutexExt;
use std::{
    any::{Any, TypeId},
    collections::BTreeMap,
    io::{self, Read},
    path::{Path, PathBuf},
    sync::{
        Arc, Mutex, OnceLock, Weak,
        atomic::{AtomicBool, AtomicUsize, Ordering},
    },
};

use handle::Entry;

pub mod environment;
mod handle;
mod store;
#[cfg(test)]
mod tests;

pub use environment::{FontEnvironment, FontIdentity};
pub use handle::{AssetState, Handle};
pub use store::{ResourceLimits, ResourceStore};

const MAX_ENCODED: usize = 64 * 1024 * 1024;

const MAX_STORE_BYTES: usize = 256 * 1024 * 1024;

const MAX_ENTRIES: usize = 4096;

thread_local! {
    static RESOLVING: std::cell::RefCell<Vec<usize>> = const { std::cell::RefCell::new(Vec::new()) };
}

/// Explicit request-owned diagnostic sink, safely shared with background tasks.
/// Clones append to the same sink. Assets views and handles carry it explicitly.
/// Resource load failures and scene-authoring failures are kept apart.
#[derive(Clone, Default, Debug)]
pub struct RequestDiagnostics(Arc<Mutex<RequestFailures>>);

#[derive(Default, Debug)]
struct RequestFailures {
    loads: Vec<Arc<crate::Error>>,
    scene: crate::ErrorReport,
}

impl RequestDiagnostics {
    fn locked(&self) -> std::sync::MutexGuard<'_, RequestFailures> {
        self.0.lock_unpoisoned()
    }

    /// Captures a resource failure once by Arc identity, not by its text or semantic equality.
    pub fn record(&self, error: Arc<crate::Error>) {
        let mut failures = self.locked();
        if !failures
            .loads
            .iter()
            .any(|existing| Arc::ptr_eq(existing, &error))
        {
            failures.loads.push(error);
        }
    }

    /// Captures a scene-authoring failure once by Arc identity.
    pub fn record_scene(&self, error: Arc<crate::Error>) {
        let mut failures = self.locked();
        if !failures
            .scene
            .errors()
            .iter()
            .any(|existing| Arc::ptr_eq(existing, &error))
        {
            failures.scene.record_shared(error);
        }
    }

    /// Snapshot of resource failures belonging to this request only.
    pub fn errors(&self) -> Vec<Arc<crate::Error>> {
        self.locked().loads.clone()
    }

    /// Snapshot of scene-authoring failures belonging to this request only.
    pub fn scene_errors(&self) -> crate::ErrorReport {
        self.locked().scene.clone()
    }
}

struct ResolveGuard;

impl Drop for ResolveGuard {
    fn drop(&mut self) {
        RESOLVING.with(|entries| {
            entries.borrow_mut().pop();
        });
    }
}

/// A custom typed resource decoded once for a shared source and project context.
pub trait Asset: Sized + Send + Sync + 'static {
    /// Human-readable resource category retained by load diagnostics.
    const KIND: &'static str;

    /// Decodes bounded input without requiring a rendering device.
    /// Implementations must be deterministic for these bytes/context; recursive
    /// resource dependency graphs are not a supported decoder contract.
    fn decode(bytes: &[u8], context: &LoadContext) -> crate::Result<Self>;

    /// Logical decoded payload charge; custom heap-owning assets should override.
    /// At least the encoded input length is charged regardless of this value.
    fn retained_bytes(&self) -> usize {
        std::mem::size_of::<Self>()
    }
}

/// Local file or immutable named bytes. Byte identity includes the content,
/// so reusing a name with different bytes cannot return the wrong cached asset.
#[derive(Clone, PartialEq, Eq, PartialOrd, Ord)]
pub enum Source {
    /// Relative to the Assets root, or an explicitly supplied absolute path.
    File(PathBuf),
    /// Embedded bytes with a diagnostic identifier.
    Bytes {
        /// Stable user-facing resource name.
        key: Arc<str>,
        /// Immutable encoded content.
        data: Arc<[u8]>,
    },
}

impl std::fmt::Debug for Source {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::File(path) => formatter.debug_tuple("File").field(path).finish(),
            Self::Bytes { key, data } => formatter
                .debug_struct("Bytes")
                .field("key", key)
                .field("length", &data.len())
                .finish(),
        }
    }
}

impl From<PathBuf> for Source {
    fn from(path: PathBuf) -> Self {
        Self::File(path)
    }
}

impl From<&Path> for Source {
    fn from(path: &Path) -> Self {
        Self::File(path.to_path_buf())
    }
}

impl From<&str> for Source {
    fn from(path: &str) -> Self {
        Self::File(path.into())
    }
}

impl From<String> for Source {
    fn from(path: String) -> Self {
        Self::File(path.into())
    }
}

/// Immutable loader configuration; no font discovery, GPU or encoder is created.
#[derive(Clone, Debug)]
pub struct LoadContext {
    root: Arc<PathBuf>,
    fonts: Option<crate::asset::environment::FontEnvironment>,
    namespace: u64,
}

impl LoadContext {
    /// Immutable font environment supplied by this resource owner, if any.
    pub fn fonts(&self) -> Option<&crate::asset::environment::FontEnvironment> {
        self.fonts.as_ref()
    }

    /// Absolute project asset directory captured when Assets is created.
    pub fn root(&self) -> &Path {
        &self.root
    }

    /// Maximum encoded bytes accepted by a single resource.
    pub fn max_encoded_bytes(&self) -> usize {
        MAX_ENCODED
    }
}

type CacheKey = (TypeId, PathBuf, u64, u64, Source);

type Erased = dyn Any + Send + Sync;

struct OwnedEntry {
    value: Arc<Erased>,
    initialized: Box<dyn Fn() -> bool + Send + Sync>,
}

impl OwnedEntry {
    fn new<AssetValue: Asset>(entry: Arc<Entry<AssetValue>>) -> Arc<Self> {
        let weak = Arc::downgrade(&entry);
        Arc::new(Self {
            value: entry,
            initialized: Box::new(move || {
                weak.upgrade()
                    .is_some_and(|entry| entry.result.get().is_some())
            }),
        })
    }
}

/// Bounded project resource owner. Owners/handles retain project weak cache
/// entries; dropping them releases cache payload charges. This is not a hard
/// process-memory cap or an automatic file-change watcher.
#[derive(Clone)]
pub struct Assets {
    context: LoadContext,
    entries: Arc<Mutex<BTreeMap<CacheKey, Arc<OwnedEntry>>>>,
    store: ResourceStore,
    diagnostics: Option<RequestDiagnostics>,
}

impl std::fmt::Debug for Assets {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("Assets")
            .field("root", &self.context.root)
            .finish_non_exhaustive()
    }
}

impl Default for Assets {
    fn default() -> Self {
        Self {
            context: LoadContext {
                root: Arc::new(PathBuf::new()),
                fonts: None,
                namespace: 0,
            },
            entries: Arc::new(Mutex::new(BTreeMap::new())),
            store: ResourceStore::new(),
            diagnostics: None,
        }
    }
}

impl Assets {
    /// Derives a request-local view without changing resource identity or ownership.
    /// Handles created from this view keep the explicit sink across threads.
    pub fn with_diagnostics(mut self, diagnostics: RequestDiagnostics) -> Self {
        self.diagnostics = Some(diagnostics);
        self
    }

    /// Freezes existing entries while isolating future registrations from old clones.
    /// Snapshot versions still share project accounting and existing decoded payloads.
    pub fn snapshot(&self) -> Self {
        static NEXT: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(1);
        let mut snapshot = self.clone();
        snapshot.context.namespace = NEXT.fetch_add(1, Ordering::Relaxed);
        let entries = self
            .entries
            .lock_unpoisoned()
            .iter()
            .filter(|(_, entry)| (entry.initialized)())
            .map(|(key, entry)| {
                let mut key = key.clone();
                key.3 = snapshot.context.namespace;
                (key, entry.clone())
            })
            .collect();
        snapshot.entries = Arc::new(Mutex::new(entries));
        snapshot
    }

    /// Registers decoded data in the same bounded entry used by lazy handles.
    /// Performs no file read; an already resolved source is never replaced.
    pub fn preload<A: Asset>(
        &self,
        source: impl Into<Source>,
        value: Arc<A>,
    ) -> crate::Result<Handle<A>> {
        let handle = self.load::<A>(source);
        let inserted = std::cell::Cell::new(false);
        let result = handle.entry.result.get_or_init(|| {
            inserted.set(true);
            let charge = value
                .retained_bytes()
                .checked_add(handle.entry.charged.load(Ordering::Acquire));
            let result = charge
                .ok_or_else(|| {
                    io::Error::new(
                        io::ErrorKind::OutOfMemory,
                        "Asset payload accounting overflow",
                    )
                })
                .and_then(|charge| {
                    let additional = charge - handle.entry.charged.load(Ordering::Acquire);
                    self.store
                        .0
                        .bytes
                        .try_update(Ordering::AcqRel, Ordering::Acquire, |current| {
                            current
                                .checked_add(additional)
                                .filter(|total| *total <= self.store.0.limits.max_bytes)
                        })
                        .map_err(|_| {
                            io::Error::new(
                                io::ErrorKind::OutOfMemory,
                                "Shared asset-cache logical payload exceeds its allowance",
                            )
                        })?;
                    handle.entry.charged.store(charge, Ordering::Release);
                    Ok(value)
                });
            result.map_err(|error| {
                Arc::new(crate::Error::asset(A::KIND, handle.source_name(), error))
            })
        });
        if let Err(error) = result {
            return Err(crate::Error::asset(
                A::KIND,
                handle.source_name(),
                error.clone(),
            ));
        }
        if !inserted.get() {
            return Err(io::Error::new(
                io::ErrorKind::AlreadyExists,
                "Asset source is already resolved",
            )
            .into());
        }
        Ok(handle)
    }

    /// Uses an explicit project store and drops this value's retained entry table.
    /// Existing owner clones, handles and decoded values keep their original ownership.
    pub fn with_store(mut self, store: ResourceStore) -> Self {
        self.store = store;
        self.entries = Arc::new(Mutex::new(BTreeMap::new()));
        self
    }

    /// Project cache identity and accounting shared by this owner's clones.
    pub fn store(&self) -> &ResourceStore {
        &self.store
    }

    /// Unregisters one typed source from this owner without invalidating handles.
    /// Clones share registrations; use snapshot() before independent edits.
    /// Retained snapshots and handles keep their payload charged until dropped.
    pub fn remove<A: Asset>(&self, source: impl Into<Source>) -> bool {
        let mut source = source.into();
        if let Source::File(path) = &mut source
            && !path.is_absolute()
        {
            *path = self.context.root.join(&*path);
        }
        let key = (
            TypeId::of::<A>(),
            self.context.root.as_ref().clone(),
            self.context
                .fonts
                .as_ref()
                .map_or(0, crate::asset::environment::FontEnvironment::snapshot_id),
            self.context.namespace,
            source,
        );
        let mut entries = self.entries.lock_unpoisoned();
        let Some(entry) = entries.remove(&key) else {
            return false;
        };
        let mut global = self.store.0.entries.lock_unpoisoned();
        if global
            .get(&key)
            .and_then(Weak::upgrade)
            .is_some_and(|value| Arc::ptr_eq(&value, &entry.value))
        {
            global.remove(&key);
        }
        true
    }

    /// Derives an independent resource view with an immutable font environment.
    /// Cache identity includes its snapshot; existing owners/handles stay valid.
    pub fn with_fonts(
        mut self,
        fonts: impl Into<crate::asset::environment::FontEnvironment>,
    ) -> Self {
        self.context.fonts = Some(fonts.into());
        self.entries = Arc::new(Mutex::new(BTreeMap::new()));
        self
    }

    /// Font snapshot shared by layout and explicit SVG parsing.
    pub fn fonts(&self) -> Option<&crate::asset::environment::FontEnvironment> {
        self.context.fonts()
    }

    /// Captures a project asset directory without reading/decode/device work.
    pub fn new(root: impl AsRef<Path>) -> crate::Result<Self> {
        Ok(Self {
            context: LoadContext {
                root: Arc::new(std::path::absolute(root)?),
                fonts: None,
                namespace: 0,
            },
            entries: Arc::new(Mutex::new(BTreeMap::new())),
            store: ResourceStore::new(),
            diagnostics: None,
        })
    }

    /// Requests a lazy typed handle. Matching type/root/font snapshot/source shares
    /// one entry only among owners attached to the same ResourceStore. Creating a
    /// handle performs no file decode. Relative parent traversal is rejected.
    pub fn load<A: Asset>(&self, source: impl Into<Source>) -> Handle<A> {
        let mut source = source.into();
        let failure = if let Source::File(path) = &mut source {
            if self.context.root.as_os_str().is_empty() && !path.is_absolute() {
                Some("Relative file assets require an explicit project root")
            } else if path
                .components()
                .any(|component| component == std::path::Component::ParentDir)
            {
                Some("Asset paths must not contain parent traversal")
            } else {
                if !path.is_absolute() {
                    *path = self.context.root.join(&*path);
                }
                None
            }
        } else if let Source::Bytes { key, data } = &source {
            if data.len() > MAX_ENCODED {
                Some("Asset source exceeds 64 MiB")
            } else if key.is_empty() || key.len() > 1024 || key.chars().any(char::is_control) {
                Some("Asset byte keys require 1..=1024 bytes without control characters")
            } else {
                None
            }
        } else {
            None
        };
        let key = (
            TypeId::of::<A>(),
            self.context.root.as_ref().clone(),
            self.context
                .fonts
                .as_ref()
                .map_or(0, crate::asset::environment::FontEnvironment::snapshot_id),
            self.context.namespace,
            source.clone(),
        );
        let mut entries = self.entries.lock_unpoisoned();
        if failure.is_none()
            && let Some(entry) = entries.get(&key)
        {
            return Handle {
                entry: entry
                    .value
                    .clone()
                    .downcast::<Entry<A>>()
                    .expect("typed resource key"),
                diagnostics: self.diagnostics.clone(),
            };
        }
        let mut global = self.store.0.entries.lock_unpoisoned();
        global.retain(|_, entry| entry.strong_count() > 0);
        let failure = failure.or_else(|| {
            (entries.len() >= self.store.0.limits.max_entries)
                .then_some("Resource cache exceeds its entry allowance")
        });
        if failure.is_none()
            && let Some(entry) = global.get(&key).and_then(Weak::upgrade)
        {
            let entry = entry.downcast::<Entry<A>>().expect("typed global key");
            entries.insert(key, OwnedEntry::new(entry.clone()));
            return Handle {
                entry,
                diagnostics: self.diagnostics.clone(),
            };
        }
        let mut failure = failure.or_else(|| {
            (global.len() >= self.store.0.limits.max_entries)
                .then_some("Resource cache exceeds its entry allowance")
        });
        let mut charged = 0;
        if failure.is_none()
            && let Source::Bytes { data, .. } = &source
        {
            if self
                .store
                .0
                .bytes
                .try_update(Ordering::AcqRel, Ordering::Acquire, |current| {
                    current
                        .checked_add(data.len())
                        .filter(|total| *total <= self.store.0.limits.max_bytes)
                })
                .is_ok()
            {
                charged = data.len();
            } else {
                failure = Some("Shared asset-cache logical payload exceeds its allowance");
            }
        }
        if failure.is_some()
            && let Source::Bytes { data, .. } = &mut source
        {
            *data = Arc::from([]);
        }
        let entry = Arc::new(Entry {
            store: self.store.clone(),
            source,
            context: self.context.clone(),
            result: OnceLock::new(),
            loading: AtomicBool::new(false),
            charged: AtomicUsize::new(charged),
        });
        if let Some(message) = failure {
            let source_name = match &entry.source {
                Source::File(path) => path.display().to_string(),
                Source::Bytes { key, .. } => key.to_string(),
            };
            let _result = entry.result.set(Err(Arc::new(crate::Error::asset(
                A::KIND,
                source_name,
                io::Error::new(io::ErrorKind::InvalidInput, message),
            ))));
        } else {
            let erased: Arc<Erased> = entry.clone();
            global.insert(key.clone(), Arc::downgrade(&erased));
            entries.insert(key, OwnedEntry::new(entry.clone()));
        }
        Handle {
            entry,
            diagnostics: self.diagnostics.clone(),
        }
    }
}
