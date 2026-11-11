//! Shared lazy handles, lifecycle and sticky decoding results.

use super::*;
use crate::sync::MutexExt;

/// Observable shared lifecycle with its sticky result; observing never starts loading.
pub enum AssetState<A> {
    /// No read or decode has begun.
    Unloaded,
    /// One caller is reading/decoding; others join the same result.
    Loading,
    /// An immutable decoded resource is available.
    Ready(Arc<A>),
    /// A sticky diagnostic is available; failures are not retried implicitly.
    Failed(Arc<crate::Error>),
}

impl<A> Clone for AssetState<A> {
    fn clone(&self) -> Self {
        match self {
            Self::Unloaded => Self::Unloaded,
            Self::Loading => Self::Loading,
            Self::Ready(asset) => Self::Ready(asset.clone()),
            Self::Failed(error) => Self::Failed(error.clone()),
        }
    }
}

impl<A> std::fmt::Debug for AssetState<A> {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Unloaded => formatter.write_str("Unloaded"),
            Self::Loading => formatter.write_str("Loading"),
            Self::Ready(_) => formatter.write_str("Ready"),
            Self::Failed(error) => formatter.debug_tuple("Failed").field(error).finish(),
        }
    }
}

impl<A> AssetState<A> {
    /// Whether no read or decode has begun.
    pub fn is_unloaded(&self) -> bool {
        matches!(self, Self::Unloaded)
    }

    /// Whether a decode is in progress.
    pub fn is_loading(&self) -> bool {
        matches!(self, Self::Loading)
    }

    /// Whether the decoded resource is available.
    pub fn is_ready(&self) -> bool {
        matches!(self, Self::Ready(_))
    }

    /// Whether loading failed.
    pub fn is_failed(&self) -> bool {
        matches!(self, Self::Failed(_))
    }

    /// Sticky original diagnostic of a failed load.
    pub fn failure(&self) -> Option<&Arc<crate::Error>> {
        match self {
            Self::Failed(error) => Some(error),
            _ => None,
        }
    }
}

pub(super) struct Entry<A: Asset> {
    pub(super) store: ResourceStore,
    pub(super) source: Source,
    pub(super) context: LoadContext,
    pub(super) result: OnceLock<Result<Arc<A>, Arc<crate::Error>>>,
    pub(super) loading: AtomicBool,
    pub(super) charged: AtomicUsize,
}

impl<A: Asset> Drop for Entry<A> {
    fn drop(&mut self) {
        self.store
            .0
            .bytes
            .fetch_sub(self.charged.load(Ordering::Relaxed), Ordering::Relaxed);
        let key = (
            TypeId::of::<A>(),
            self.context.root.as_ref().clone(),
            self.context
                .fonts
                .as_ref()
                .map_or(0, crate::asset::environment::FontEnvironment::snapshot_id),
            self.context.namespace,
            self.source.clone(),
        );
        let mut cache = self.store.0.entries.lock_unpoisoned();
        if cache
            .get(&key)
            .is_some_and(|entry| entry.strong_count() == 0)
        {
            cache.remove(&key);
        }
    }
}

/// Cheap shared reference to a lazy resource; clones share success or failure.
pub struct Handle<A: Asset> {
    pub(super) entry: Arc<Entry<A>>,
    pub(super) diagnostics: Option<RequestDiagnostics>,
}

impl<A: Asset> Clone for Handle<A> {
    fn clone(&self) -> Self {
        Self {
            entry: self.entry.clone(),
            diagnostics: self.diagnostics.clone(),
        }
    }
}

impl<A: Asset> std::fmt::Debug for Handle<A> {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("Handle")
            .field("kind", &A::KIND)
            .field("source", &self.entry.source)
            .field("state", &self.state())
            .finish()
    }
}

impl<A: Asset> Handle<A> {
    /// Current state without triggering a file read or decode.
    ///
    /// This is a non-authoritative snapshot for polling and diagnostics: another thread may change
    /// it immediately after the call. [`Handle::get`] is the only call that loads, and a `Ready`
    /// or `Failed` state is sticky.
    pub fn state(&self) -> AssetState<A> {
        match self.entry.result.get() {
            Some(Ok(asset)) => AssetState::Ready(asset.clone()),
            Some(Err(error)) => AssetState::Failed(error.clone()),
            None if self.entry.loading.load(Ordering::Acquire) => AssetState::Loading,
            None => AssetState::Unloaded,
        }
    }

    /// Resolves this handle synchronously, joining one concurrent decode.
    /// User decoder panics become sticky asset errors. Decoders must not
    /// recursively resolve the same handle. Returned resources are immutable.
    pub fn get(&self) -> crate::Result<Arc<A>> {
        match &self.diagnostics {
            Some(diagnostics) => self.get_with(diagnostics),
            None => self.get_with(&RequestDiagnostics::default()),
        }
    }

    /// Resolves into an explicit request sink, including when called on another thread.
    pub fn get_with(&self, diagnostics: &RequestDiagnostics) -> crate::Result<Arc<A>> {
        let _guard = if self.entry.result.get().is_none() {
            let identity = Arc::as_ptr(&self.entry) as usize;
            let accepted = RESOLVING.with(|entries| {
                let mut entries = entries.borrow_mut();
                if entries.len() >= 64 || entries.contains(&identity) {
                    false
                } else {
                    entries.push(identity);
                    true
                }
            });
            if !accepted {
                return Err(crate::Error::asset(
                    A::KIND,
                    self.source_name(),
                    io::Error::other("Recursive resource resolution or load depth exceeds 64"),
                ));
            }
            Some(ResolveGuard)
        } else {
            None
        };
        let result = self.entry.result.get_or_init(|| {
            self.entry.loading.store(true, Ordering::Release);
            let decoded =
                std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| -> crate::Result<A> {
                    let bytes: Arc<[u8]> = match &self.entry.source {
                        Source::Bytes { data, .. } => data.clone(),
                        Source::File(path) => {
                            let metadata = std::fs::metadata(path)?;
                            if metadata.len() > MAX_ENCODED as u64 {
                                return Err(io::Error::new(
                                    io::ErrorKind::InvalidData,
                                    "Asset source exceeds 64 MiB",
                                )
                                .into());
                            }
                            if !metadata.is_file() {
                                return Err(io::Error::new(
                                    io::ErrorKind::InvalidInput,
                                    "Asset source must be a regular file",
                                )
                                .into());
                            }
                            let file = std::fs::File::open(path)?;
                            if !file.metadata()?.is_file() {
                                return Err(io::Error::new(
                                    io::ErrorKind::InvalidInput,
                                    "Asset source must be a regular file",
                                )
                                .into());
                            }
                            let mut bytes = Vec::new();
                            file.take(MAX_ENCODED as u64 + 1).read_to_end(&mut bytes)?;
                            bytes.into()
                        }
                    };
                    if bytes.len() > MAX_ENCODED {
                        return Err(io::Error::new(
                            io::ErrorKind::InvalidData,
                            "Asset source exceeds 64 MiB",
                        )
                        .into());
                    }
                    let asset = A::decode(&bytes, &self.entry.context)?;
                    let charge = asset
                        .retained_bytes()
                        .checked_add(if matches!(self.entry.source, Source::Bytes { .. }) {
                            bytes.len()
                        } else {
                            0
                        })
                        .ok_or_else(|| {
                            io::Error::new(
                                io::ErrorKind::OutOfMemory,
                                "Asset payload accounting overflow",
                            )
                        })?
                        .max(bytes.len());
                    let additional = charge - self.entry.charged.load(Ordering::Acquire);
                    self.entry
                        .store
                        .0
                        .bytes
                        .try_update(Ordering::AcqRel, Ordering::Acquire, |current| {
                            current
                                .checked_add(additional)
                                .filter(|total| *total <= self.entry.store.0.limits.max_bytes)
                        })
                        .map_err(|_| {
                            io::Error::new(
                                io::ErrorKind::OutOfMemory,
                                "Shared asset-cache logical payload exceeds its allowance",
                            )
                        })?;
                    self.entry.charged.store(charge, Ordering::Release);
                    Ok(asset)
                }));
            let decoded = decoded.unwrap_or_else(|payload| {
                let message = payload
                    .downcast_ref::<String>()
                    .cloned()
                    .or_else(|| {
                        payload
                            .downcast_ref::<&str>()
                            .map(|message| (*message).to_owned())
                    })
                    .unwrap_or_else(|| "non-string decoder panic".into());
                Err(io::Error::other(format!("Asset decoder panicked: {message}")).into())
            });

            decoded
                .map(Arc::new)
                .map_err(|error| Arc::new(crate::Error::asset(A::KIND, self.source_name(), error)))
        });
        if let Err(error) = result {
            diagnostics.record(error.clone());
        }
        result
            .as_ref()
            .map(Arc::clone)
            .map_err(|error| crate::Error::asset(A::KIND, self.source_name(), error.clone()))
    }

    /// Resource path or embedded key, suitable for diagnostic display.
    pub fn source_name(&self) -> String {
        match &self.entry.source {
            Source::File(path) => path.display().to_string(),
            Source::Bytes { key, .. } => key.to_string(),
        }
    }
}
