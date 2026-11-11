//! Shared deferred-content pipeline: compile key, compiled vector tree, selection, reveal.

use super::{VectorTree, WriteOptions};
use motion_core::sync::MutexExt;
use motion_core::{Node, ValidationError};
use std::{
    collections::{BTreeMap, BTreeSet},
    sync::{Arc, Mutex},
};

/// Most results one cache retains before it starts over.
const MAX_SLOTS: usize = 8;

/// A compile cache created from one immutable key.
///
/// Compilation inputs live in the key, so changing one produces a new, empty
/// cache through [`CompileCache::map_key`]; nothing is ever cleared by hand.
/// Clones share their results. A variant distinguishes environments that are
/// not part of the key, such as the font snapshot a paragraph is shaped with.
pub struct CompileCache<K, V> {
    key: Arc<K>,
    slots: Arc<Mutex<BTreeMap<u64, V>>>,
}

impl<K, V> Clone for CompileCache<K, V> {
    fn clone(&self) -> Self {
        Self {
            key: self.key.clone(),
            slots: self.slots.clone(),
        }
    }
}

impl<K: std::fmt::Debug, V> std::fmt::Debug for CompileCache<K, V> {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("CompileCache")
            .field("key", &self.key)
            .finish_non_exhaustive()
    }
}

impl<K, V: Clone> CompileCache<K, V> {
    /// Creates an empty cache for a key.
    pub fn new(key: K) -> Self {
        Self {
            key: Arc::new(key),
            slots: Arc::default(),
        }
    }

    /// The immutable key every cached result was compiled from.
    pub fn key(&self) -> &K {
        &self.key
    }

    /// Whether two caches share their results, as clones and unchanged keys do.
    pub fn shares(&self, other: &Self) -> bool {
        Arc::ptr_eq(&self.slots, &other.slots)
    }

    /// Whether any result has been compiled yet.
    pub fn is_compiled(&self) -> bool {
        !self.slots.lock_unpoisoned().is_empty()
    }

    /// Returns the result for a variant, compiling it once from the key.
    /// The compiler runs while the cache is locked, so concurrent callers
    /// never compile the same variant twice.
    pub fn get_or_compile(&self, variant: u64, compile: impl FnOnce(&K) -> V) -> V {
        let mut slots = self.slots.lock_unpoisoned();
        if let Some(result) = slots.get(&variant) {
            return result.clone();
        }
        let result = compile(&self.key);
        if slots.len() >= MAX_SLOTS {
            slots.clear();
        }
        slots.insert(variant, result.clone());
        result
    }
}

impl<K: Clone + PartialEq, V: Clone> CompileCache<K, V> {
    /// Applies a change to a copy of the key. An unchanged key keeps sharing
    /// this cache; a changed key gets a new one.
    pub fn map_key(&self, change: impl FnOnce(&mut K)) -> Self {
        let mut key = (*self.key).clone();
        change(&mut key);
        if key == *self.key {
            self.clone()
        } else {
            Self::new(key)
        }
    }
}

type Modify = Arc<dyn Fn(Node) -> Node + Send + Sync>;

/// Authored modifier applied to the original vector groups at known addresses.
#[derive(Clone)]
pub struct VectorSelection {
    addresses: Arc<BTreeSet<Vec<usize>>>,
    modify: Modify,
}

impl VectorSelection {
    /// Selects original tree addresses and the ordinary modifier to apply to them.
    pub fn new(
        addresses: impl IntoIterator<Item = Vec<usize>>,
        modify: impl Fn(Node) -> Node + Send + Sync + 'static,
    ) -> Self {
        Self {
            addresses: Arc::new(addresses.into_iter().collect()),
            modify: Arc::new(modify),
        }
    }

    /// Selects addresses with an already shared modifier.
    pub fn shared(addresses: impl IntoIterator<Item = Vec<usize>>, modify: &Modify) -> Self {
        Self {
            addresses: Arc::new(addresses.into_iter().collect()),
            modify: modify.clone(),
        }
    }
}

impl VectorTree {
    /// Selection then reveal: applies each selection in declaration order to its
    /// original groups, and optionally writes the result by cumulative length.
    pub fn reveal(
        &self,
        selections: &[VectorSelection],
        write: Option<f64>,
    ) -> Result<Node, ValidationError> {
        let map = |address: &[usize], mut node: Node| {
            for selection in selections {
                if selection.addresses.contains(address) {
                    node = (selection.modify)(node);
                }
            }
            node
        };
        match write {
            Some(progress) => self.write_mapped(progress, WriteOptions::default(), &map),
            None => Ok(self.node_mapped(&map)),
        }
    }
}
