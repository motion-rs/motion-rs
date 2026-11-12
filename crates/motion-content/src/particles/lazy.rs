//! Particle instances that are either validated eagerly or materialized on first CPU use.

use super::batch::{Instance, SpatialNode};
use super::*;
use std::sync::OnceLock;

/// Validated instances with the spatial index built over them.
#[derive(Clone, Debug)]
pub(super) struct Materialized {
    pub(super) instances: Arc<[Instance]>,
    pub(super) index: Arc<[SpatialNode]>,
}

type Source = Arc<dyn Fn() -> Result<Materialized, ValidationError> + Send + Sync>;

/// A batch whose positions the GPU evaluates never needs CPU instances; the CPU reference
/// path materializes them once, on first use, and shares the result between clones.
#[derive(Clone)]
pub(super) enum Instances {
    Ready(Materialized),
    Deferred {
        len: usize,
        cell: Arc<OnceLock<Result<Materialized, ValidationError>>>,
        source: Source,
    },
}

impl Instances {
    pub(super) fn ready(instances: Arc<[Instance]>, index: Arc<[SpatialNode]>) -> Self {
        Self::Ready(Materialized { instances, index })
    }

    pub(super) fn deferred(
        len: usize,
        source: impl Fn() -> Result<Materialized, ValidationError> + Send + Sync + 'static,
    ) -> Self {
        Self::Deferred {
            len,
            cell: Arc::new(OnceLock::new()),
            source: Arc::new(source),
        }
    }

    pub(super) fn len(&self) -> usize {
        match self {
            Self::Ready(ready) => ready.instances.len(),
            Self::Deferred { len, .. } => *len,
        }
    }

    /// Spatial nodes charged to the batch; a deferred batch charges none until used.
    pub(super) fn index_len(&self) -> usize {
        match self {
            Self::Ready(ready) => ready.index.len(),
            Self::Deferred { .. } => 0,
        }
    }

    pub(super) fn is_empty(&self) -> bool {
        self.len() == 0
    }

    pub(super) fn is_deferred(&self) -> bool {
        matches!(self, Self::Deferred { .. })
    }

    pub(super) fn get(&self) -> Result<&Materialized, ValidationError> {
        match self {
            Self::Ready(ready) => Ok(ready),
            Self::Deferred { cell, source, .. } => {
                cell.get_or_init(|| source()).as_ref().map_err(Clone::clone)
            }
        }
    }
}

impl std::fmt::Debug for Instances {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Ready(ready) => formatter.debug_tuple("Ready").field(ready).finish(),
            Self::Deferred { len, .. } => formatter
                .debug_struct("Deferred")
                .field("len", len)
                .finish_non_exhaustive(),
        }
    }
}

/// Deferred batches are equal when their analytic program (compared by the batch) and count
/// match; they never equal a materialized batch.
impl PartialEq for Instances {
    fn eq(&self, other: &Self) -> bool {
        match (self, other) {
            (Self::Ready(first), Self::Ready(second)) => first.instances == second.instances,
            (Self::Deferred { len: first, .. }, Self::Deferred { len: second, .. }) => {
                first == second
            }
            _ => false,
        }
    }
}

#[cfg(test)]
impl Instances {
    pub(super) fn is_materialized(&self) -> bool {
        match self {
            Self::Ready(_) => true,
            Self::Deferred { cell, .. } => cell.get().is_some(),
        }
    }

    pub(super) fn slice(&self) -> &[Instance] {
        &self.get().expect("test instances materialize").instances
    }
}

#[cfg(test)]
impl From<Vec<Instance>> for Instances {
    fn from(instances: Vec<Instance>) -> Self {
        Self::ready(instances.into(), Arc::from([]))
    }
}
