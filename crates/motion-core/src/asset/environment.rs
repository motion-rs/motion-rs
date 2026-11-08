//! Immutable, type-erased content environments carried by core scene contracts.

use std::{
    any::Any,
    fmt,
    sync::{Arc, Mutex},
};

/// Explicit font-source observations shared by rendering workers for one export.
#[derive(Clone, Default, Debug)]
pub struct FontUsage(Arc<Mutex<Vec<FontIdentity>>>);

impl FontUsage {
    /// Records a selected face once, including when its outline came from a cache.
    pub fn record(&self, font: FontIdentity) {
        let mut fonts = self.0.lock().unwrap_or_else(|error| error.into_inner());
        if !fonts.contains(&font) {
            fonts.push(font);
        }
    }

    /// Returns stable face ordering independent of worker completion order.
    pub fn fonts(&self) -> Vec<FontIdentity> {
        let mut fonts = self
            .0
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            .clone();
        fonts.sort_by(|left, right| {
            (&left.name, left.file_hash).cmp(&(&right.name, right.file_hash))
        });
        fonts
    }
}

/// A frozen content environment with an explicit cache identity.
/// The content package owns its concrete font implementation; core only carries
/// its immutable snapshot and never initializes a font engine.
#[derive(Clone)]
pub struct FontEnvironment {
    value: Arc<dyn Any + Send + Sync>,
    identity: u64,
    usage: Option<Arc<dyn Fn() -> Vec<FontIdentity> + Send + Sync>>,
}

impl std::panic::RefUnwindSafe for FontEnvironment {}

impl std::panic::UnwindSafe for FontEnvironment {}

impl From<&FontEnvironment> for FontEnvironment {
    fn from(value: &FontEnvironment) -> Self {
        value.clone()
    }
}

impl FontEnvironment {
    /// Captures an immutable environment under its owning implementation's identity.
    pub fn new<T: Any + Send + Sync>(value: T, identity: u64) -> Self {
        Self {
            value: Arc::new(value),
            identity,
            usage: None,
        }
    }

    /// Stable identity of this immutable environment snapshot.
    pub fn snapshot_id(&self) -> u64 {
        self.identity
    }

    /// Attaches explicit usage reporting without exposing the content engine to core.
    pub fn with_usage(
        mut self,
        usage: impl Fn() -> Vec<FontIdentity> + Send + Sync + 'static,
    ) -> Self {
        self.usage = Some(Arc::new(usage));
        self
    }

    /// Faces actually selected from this shared environment, including source hashes.
    pub fn used_fonts(&self) -> Vec<FontIdentity> {
        self.usage.as_ref().map_or_else(Vec::new, |usage| usage())
    }

    /// Resolves an environment only in the package that owns its concrete type.
    pub fn downcast_ref<T: Any>(&self) -> Option<&T> {
        self.value.downcast_ref()
    }
}

impl fmt::Debug for FontEnvironment {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("FontEnvironment")
            .field("snapshot", &self.identity)
            .finish_non_exhaustive()
    }
}

/// Identity of one parsed font face, in primary-then-fallback order.
/// Hashes identify the font source, not a cryptographic checksum or a promise of
/// identical rasterization across different text-engine versions/platforms.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FontIdentity {
    /// Face name when present in the font's metadata.
    pub name: Option<String>,
    /// Non-cryptographic font-source hash supplied by the text engine.
    pub file_hash: u64,
}
