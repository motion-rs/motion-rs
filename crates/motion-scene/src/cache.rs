//! Caller-owned, bounded reuse of compiled file documents and dependency snapshots.

use crate::MAX_DOCUMENT_BYTES;
use crate::compile::{CompiledDocument, compile_source};
use motion_core::ValidationError;
use serde_json::Value;
use std::io::Read;
use std::path::{Path, PathBuf};
use std::sync::Arc;

/// Maximum retained source, parameter and dependency snapshot bytes per cache entry.
const MAX_CACHE_BYTES: usize = 16 * 1024 * 1024;

/// Maximum dependency snapshots retained for one compiled document.
const MAX_CACHED_RESOURCES: usize = 256;

/// A single-entry compilation cache with explicit file IO and no global state.
///
/// Successful loads reuse an immutable compiled document only when the absolute
/// path, source, overrides and dependency bytes match. Failed loads never replace
/// the last successful entry. Inputs exceeding 16 MiB of snapshot bytes or 256
/// dependencies compile normally without being cached. The compiled timeline's
/// own limits are separate from these retained input limits.
#[derive(Debug, Default)]
pub struct CompileCache {
    entry: Option<CacheEntry>,
}

impl CompileCache {
    /// Reads and compiles a file, reusing an unchanged successful load.
    pub fn load(&mut self, path: impl AsRef<Path>) -> motion_core::Result<Arc<CompiledDocument>> {
        self.load_with(path, None)
    }

    /// Loads a file with parameter overrides, checking dependency contents on hits.
    ///
    /// Relative resources resolve against the document directory as in [`crate::load_with`].
    /// No failure is cached; IO and validation errors propagate to the caller.
    pub fn load_with(
        &mut self,
        path: impl AsRef<Path>,
        overrides: Option<&Value>,
    ) -> motion_core::Result<Arc<CompiledDocument>> {
        let path = std::path::absolute(path.as_ref())?;
        let text = read_source(&path)?;
        if let Some(entry) = &self.entry
            && entry.path == path
            && entry.text == text
            && entry.overrides.as_ref() == overrides
            && resource_snapshots(&entry.compiled.resources()[1..], entry.remaining)
                .is_some_and(|snapshots| snapshots == entry.resources)
        {
            return Ok(entry.compiled.clone());
        }
        let compiled = Arc::new(compile_source(path.clone(), &text, overrides)?);
        let parameter_bytes = serde_json::to_vec(&overrides)
            .map_err(|error| ValidationError::invalid("params", "json", error.to_string()))?
            .len();
        self.entry = text
            .len()
            .checked_add(parameter_bytes)
            .and_then(|bytes| MAX_CACHE_BYTES.checked_sub(bytes))
            .and_then(|remaining| {
                let resources = resource_snapshots(&compiled.resources()[1..], remaining)?;
                Some(CacheEntry {
                    path,
                    text,
                    overrides: overrides.cloned(),
                    compiled: compiled.clone(),
                    remaining,
                    resources,
                })
            });
        Ok(compiled)
    }
}

#[derive(Debug)]
struct CacheEntry {
    path: PathBuf,
    text: String,
    overrides: Option<Value>,
    compiled: Arc<CompiledDocument>,
    remaining: usize,
    resources: Vec<Vec<u8>>,
}

pub(crate) fn read_source(path: &Path) -> motion_core::Result<String> {
    let mut text = String::new();
    std::fs::File::open(path)?
        .take(MAX_DOCUMENT_BYTES as u64 + 1)
        .read_to_string(&mut text)?;
    if text.len() > MAX_DOCUMENT_BYTES {
        return Err(ValidationError::limit("document", "bytes", "Document exceeds 16 MiB").into());
    }
    Ok(text)
}

fn resource_snapshots(paths: &[PathBuf], mut remaining: usize) -> Option<Vec<Vec<u8>>> {
    if paths.len() > MAX_CACHED_RESOURCES {
        return None;
    }
    let mut snapshots = Vec::with_capacity(paths.len());
    for path in paths {
        let file = std::fs::File::open(path).ok()?;
        if file.metadata().ok()?.len() > remaining as u64 {
            return None;
        }
        let mut bytes = Vec::new();
        file.take(remaining as u64 + 1)
            .read_to_end(&mut bytes)
            .ok()?;
        remaining = remaining.checked_sub(bytes.len())?;
        snapshots.push(bytes);
    }
    Some(snapshots)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn source() -> String {
        json!({"version":1,"size":[8,8],"fps":4,"params":{"seconds":1},
            "tracks":[{"items":[{"scene":{"duration":{"$param":"seconds"},
                "content":{"children":[]}}}]}], "audio":[{"source":"resource.wav"}]})
        .to_string()
    }

    #[test]
    fn compilation_cache_reuses_only_complete_matching_inputs() -> motion_core::Result<()> {
        let directory = tempfile::tempdir()?;
        let path = directory.path().join("film.json");
        let resource = directory.path().join("resource.wav");
        std::fs::write(&path, source())?;
        std::fs::write(&resource, "old")?;
        let mut cache = CompileCache::default();
        let first = cache.load(&path)?;
        assert!(Arc::ptr_eq(&first, &cache.load(&path)?));
        let modified = std::fs::metadata(&resource)?.modified()?;
        std::fs::write(&resource, "new")?;
        std::fs::File::options()
            .write(true)
            .open(&resource)?
            .set_modified(modified)?;
        let changed = cache.load(&path)?;
        assert!(!Arc::ptr_eq(&first, &changed));
        assert!(Arc::ptr_eq(&changed, &cache.load(&path)?));
        let overridden = cache.load_with(&path, Some(&json!({"seconds":2})))?;
        assert!(!Arc::ptr_eq(&changed, &overridden));
        assert_eq!(overridden.timeline().frame_count(), 8);
        std::fs::write(&path, "{")?;
        assert!(cache.load(&path).is_err());
        assert!(cache.load(&path).is_err());
        std::fs::write(&path, source())?;
        assert!(Arc::ptr_eq(
            &overridden,
            &cache.load_with(&path, Some(&json!({"seconds":2})))?
        ));
        let other = directory.path().join("other.json");
        std::fs::write(&other, source())?;
        assert!(!Arc::ptr_eq(
            &overridden,
            &cache.load_with(&other, Some(&json!({"seconds":2})))?
        ));
        Ok(())
    }

    #[test]
    fn compilation_cache_bypasses_oversized_or_missing_dependencies() -> motion_core::Result<()> {
        let directory = tempfile::tempdir()?;
        let path = directory.path().join("film.json");
        let resource = directory.path().join("resource.wav");
        std::fs::write(&path, source())?;
        std::fs::File::create(&resource)?.set_len(MAX_CACHE_BYTES as u64 + 1)?;
        let mut cache = CompileCache::default();
        let first = cache.load(&path)?;
        assert!(!Arc::ptr_eq(&first, &cache.load(&path)?));
        assert!(cache.entry.is_none());
        std::fs::remove_file(resource)?;
        let first = cache.load(&path)?;
        assert!(!Arc::ptr_eq(&first, &cache.load(&path)?));
        Ok(())
    }
}
