//! Shared lazy handles, lifecycle and sticky decoding results.

use super::*;

/// Observable shared lifecycle; loading starts only at the first get call.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AssetStatus {
    /// No read or decode has begun.
    Unloaded,
    /// One caller is reading/decoding; others join the same result.
    Loading,
    /// An immutable decoded resource is available.
    Ready,
    /// A sticky diagnostic is available; failures are not retried implicitly.
    Failed,
}

pub(super) struct Entry<A: Asset> {
    pub(super) pool: ResourcePool,
    pub(super) source: Source,
    pub(super) context: LoadContext,
    pub(super) result: OnceLock<Result<Arc<A>, Arc<crate::Error>>>,
    pub(super) loading: AtomicBool,
    pub(super) charged: AtomicUsize,
}

impl<A: Asset> Drop for Entry<A> {
    fn drop(&mut self) {
        self.pool
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
        let mut cache = self
            .pool
            .0
            .entries
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
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
            .field("status", &self.status())
            .finish()
    }
}

impl<A: Asset> Handle<A> {
    /// Current state without triggering a file read or decode.
    ///
    /// This is a non-authoritative snapshot for polling and diagnostics: another thread may change
    /// it immediately after the call. [`Handle::get`] is the only authoritative result.
    pub fn status(&self) -> AssetStatus {
        match self.entry.result.get() {
            Some(Ok(_)) => AssetStatus::Ready,
            Some(Err(_)) => AssetStatus::Failed,
            None if self.entry.loading.load(Ordering::Acquire) => AssetStatus::Loading,
            None => AssetStatus::Unloaded,
        }
    }

    /// Sticky original diagnostic, without triggering loading.
    pub fn error(&self) -> Option<Arc<crate::Error>> {
        self.entry
            .result
            .get()
            .and_then(|result| result.as_ref().err().cloned())
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
                        .pool
                        .0
                        .bytes
                        .try_update(Ordering::AcqRel, Ordering::Acquire, |current| {
                            current
                                .checked_add(additional)
                                .filter(|total| *total <= self.entry.pool.0.max_bytes)
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
