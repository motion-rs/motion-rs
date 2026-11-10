//! Bounded collection of errors that fluent builders report later.

use crate::Error;
use std::sync::Arc;

const MAX_DEFERRED_ERRORS: usize = 1024;

/// Ordered failures retained by a fluent builder until it is validated or built.
///
/// At most 1024 errors are kept; later ones are dropped. Nested
/// [`Error::BuildErrors`] are flattened so callers see individual failures.
///
/// ```
/// use motion_core::{Error, deferred::DeferredErrors};
/// let mut errors = DeferredErrors::default();
/// errors.record(Error::from(std::io::Error::other("disk failure")));
/// assert_eq!(errors.len(), 1);
/// ```
#[derive(Clone, Debug, Default)]
pub struct DeferredErrors {
    errors: Vec<Arc<Error>>,
}

impl DeferredErrors {
    /// Appends one failure, flattening build error lists, until the bound is reached.
    pub fn record(&mut self, error: Error) {
        let room = MAX_DEFERRED_ERRORS.saturating_sub(self.errors.len());
        match error {
            Error::BuildErrors(values) => self.errors.extend(values.into_iter().take(room)),
            error if room > 0 => self.errors.push(Arc::new(error)),
            _ => {}
        }
    }

    /// Retained failures in insertion order.
    pub fn as_slice(&self) -> &[Arc<Error>] {
        &self.errors
    }

    /// Number of retained failures.
    pub fn len(&self) -> usize {
        self.errors.len()
    }

    /// Whether no failure was recorded.
    pub fn is_empty(&self) -> bool {
        self.errors.is_empty()
    }

    /// Takes the retained failures.
    pub fn into_vec(self) -> Vec<Arc<Error>> {
        self.errors
    }
}
