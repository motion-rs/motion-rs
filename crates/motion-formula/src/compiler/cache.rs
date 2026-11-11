//! Bounded single-initialization formula cache, its keys and logical byte accounting.

use super::{
    FORMULA_BACKEND_VERSION, FormulaAsset, FormulaCompileError, FormulaCompiler, FormulaDiagnostic,
    FormulaOptions, MAX_SOURCE,
};
use motion_core::sync::MutexExt;
use std::collections::BTreeMap;
use std::sync::{Arc, OnceLock};

#[derive(Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub(super) struct Key {
    source: Arc<str>,
    size_bits: u64,
    family: Arc<str>,
    color_bits: [u64; 4],
    revision: u64,
    backend: &'static str,
    environment_id: u64,
}

type Compiled = Result<Arc<FormulaAsset>, FormulaCompileError>;

pub(super) struct Cached {
    value: OnceLock<Compiled>,
    bytes: OnceLock<usize>,
}

type Entry = Arc<Cached>;

#[derive(Default)]
pub(super) struct Cache {
    entries: BTreeMap<Key, Entry>,
    charged: std::collections::BTreeSet<Key>,
    bytes: usize,
}

/// Cache-owned logical totals; pinned external formulas/compiler internals are excluded.
#[derive(Clone, Copy, Debug)]
pub struct FormulaCacheStats {
    /// Currently retained ready or loading keys.
    pub entries: usize,
    /// Conservatively counted ready-result bytes, capped at 64 MiB.
    pub retained_bytes: usize,
}

impl FormulaCompiler {
    pub(crate) fn compile_cached(
        &self,
        math: impl Into<Arc<str>>,
        options: &FormulaOptions,
        shared: Option<&motion_core::SceneCache>,
    ) -> motion_core::Result<Arc<FormulaAsset>> {
        let math = math.into();
        let rgba = options.fill.to_rgba_float();
        if math.trim().is_empty()
            || math.len() > MAX_SOURCE
            || !options.size.is_finite()
            || !(1.0..=1024.0).contains(&options.size)
            || options.family.is_empty()
            || options.family.len() > 128
            || !options
                .family
                .bytes()
                .all(|byte| byte.is_ascii_alphanumeric() || b" -_".contains(&byte))
            || rgba.iter().any(|value| !value.is_finite())
        {
            return Err(FormulaCompileError::new(
                "Invalid source, point size, font family or fill",
            )
            .into());
        }
        let key = Key {
            source: math.clone(),
            size_bits: options.size.to_bits(),
            family: options.family.clone(),
            color_bits: rgba.map(f64::to_bits),
            revision: options.revision,
            backend: FORMULA_BACKEND_VERSION,
            environment_id: self.environment(),
        };
        if let Some(shared) = shared {
            let key_bytes = key.source.len() + key.family.len() + std::mem::size_of::<Key>();
            return shared
                .get_with_cost(
                    key,
                    || self.compile_once(math, options),
                    |result| {
                        key_bytes
                            + match result {
                                Ok(formula) => formula.retained_bytes(),
                                Err(error) => error
                                    .diagnostics
                                    .iter()
                                    .map(|diagnostic| {
                                        std::mem::size_of::<FormulaDiagnostic>()
                                            + diagnostic.message.len()
                                    })
                                    .sum(),
                            }
                    },
                )
                .as_ref()
                .clone()
                .map_err(Into::into);
        }
        let entry = {
            let mut cache = self.inner.cache.lock_unpoisoned();
            if let Some(entry) = cache.entries.get(&key) {
                entry.clone()
            } else {
                if cache.entries.len() >= 32 {
                    *cache = Cache::default();
                }
                let entry = Arc::new(Cached {
                    value: OnceLock::new(),
                    bytes: OnceLock::new(),
                });
                cache.entries.insert(key.clone(), entry.clone());
                entry
            }
        };
        let result = entry
            .value
            .get_or_init(|| self.compile_once(math, options))
            .clone();
        let bytes = *entry.bytes.get_or_init(|| match &result {
            Ok(formula) => formula.retained_bytes(),
            Err(error) => error
                .diagnostics
                .iter()
                .map(|diagnostic| {
                    std::mem::size_of::<FormulaDiagnostic>() + diagnostic.message.len()
                })
                .sum(),
        });
        {
            let mut cache = self.inner.cache.lock_unpoisoned();
            if cache
                .entries
                .get(&key)
                .is_some_and(|cached| Arc::ptr_eq(cached, &entry))
                && !cache.charged.contains(&key)
            {
                if bytes > 64 * 1024 * 1024 {
                    cache.entries.remove(&key);
                } else {
                    if cache.bytes + bytes > 64 * 1024 * 1024 {
                        *cache = Cache::default();
                        cache.entries.insert(key.clone(), entry.clone());
                    }
                    cache.bytes += bytes;
                    cache.charged.insert(key);
                }
            }
        }
        result.map_err(Into::into)
    }

    /// Reports bounded cache ownership without compiling or touching devices.
    pub fn cache_stats(&self) -> motion_core::Result<FormulaCacheStats> {
        let cache = self.inner.cache.lock_unpoisoned();
        Ok(FormulaCacheStats {
            entries: cache.entries.len(),
            retained_bytes: cache.bytes,
        })
    }
}
