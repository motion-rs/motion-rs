//! Bounded error report that collects every failure of one authoring step.

use crate::{BoxedError, Error};
use std::{error::Error as StdError, fmt, sync::Arc};

const MAX_REPORTED_ERRORS: usize = 1024;

/// Ordered failures collected while authoring or compiling one value.
///
/// A report is the fatal counterpart of [`Diagnostics`](crate::Diagnostics), which carries the
/// non-fatal warnings of the same step. [`Error`] always describes exactly one failure; fluent
/// builders retain their deferred failures here and return the whole report from `build`.
///
/// At most 1024 errors are kept; later ones are dropped. Reports passed to
/// [`ErrorReport::record`] are flattened so callers see individual failures. Converting a report
/// with `?` into [`Error`] keeps it reachable through [`Error::report`].
///
/// ```
/// use motion_core::{Error, ErrorReport};
/// let mut report = ErrorReport::default();
/// report.record(Error::from(std::io::Error::other("disk failure")));
/// assert_eq!(report.len(), 1);
/// ```
#[derive(Clone, Debug, Default)]
pub struct ErrorReport {
    errors: Vec<Arc<Error>>,
}

impl ErrorReport {
    /// Appends one failure, flattening nested reports, until the bound is reached.
    pub fn record(&mut self, error: Error) {
        let room = MAX_REPORTED_ERRORS.saturating_sub(self.errors.len());
        match error {
            Error::Other(cause) => match cause.downcast::<ErrorReport>() {
                Ok(report) => self.errors.extend(report.errors.into_iter().take(room)),
                Err(cause) if room > 0 => self.errors.push(Arc::new(Error::Other(cause))),
                Err(_) => {}
            },
            error if room > 0 => self.errors.push(Arc::new(error)),
            _ => {}
        }
    }

    /// Appends an already shared failure without flattening it.
    pub fn record_shared(&mut self, error: Arc<Error>) {
        if self.errors.len() < MAX_REPORTED_ERRORS {
            self.errors.push(error);
        }
    }

    /// Retained failures in insertion order.
    pub fn errors(&self) -> &[Arc<Error>] {
        &self.errors
    }

    /// First retained failure, which classifies the report.
    pub fn first(&self) -> Option<&Arc<Error>> {
        self.errors.first()
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

    /// Replaces every retained failure with `map` of it, preserving order.
    pub fn map(self, map: impl FnMut(Arc<Error>) -> Arc<Error>) -> Self {
        Self {
            errors: self.errors.into_iter().map(map).collect(),
        }
    }
}

impl From<Error> for ErrorReport {
    fn from(error: Error) -> Self {
        let mut report = Self::default();
        report.record(error);
        report
    }
}

impl From<crate::ValidationError> for ErrorReport {
    fn from(error: crate::ValidationError) -> Self {
        Error::from(error).into()
    }
}

impl From<std::io::Error> for ErrorReport {
    fn from(error: std::io::Error) -> Self {
        Error::from(error).into()
    }
}

impl From<BoxedError> for ErrorReport {
    fn from(error: BoxedError) -> Self {
        Error::from(error).into()
    }
}

impl From<ErrorReport> for Error {
    fn from(report: ErrorReport) -> Self {
        let mut errors = report.errors;
        if errors.len() == 1 {
            match Arc::try_unwrap(errors.remove(0)) {
                Ok(error) => return error,
                Err(shared) => errors.push(shared),
            }
        }
        Self::Other(Box::new(ErrorReport { errors }))
    }
}

impl fmt::Display for ErrorReport {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "{} authoring error(s)", self.errors.len())?;
        for error in &self.errors {
            write!(formatter, "\n  {error}")?;
        }
        Ok(())
    }
}

impl StdError for ErrorReport {
    fn source(&self) -> Option<&(dyn StdError + 'static)> {
        self.errors
            .first()
            .map(|error| error.as_ref() as &(dyn StdError + 'static))
    }
}

impl IntoIterator for ErrorReport {
    type Item = Arc<Error>;

    type IntoIter = std::vec::IntoIter<Arc<Error>>;

    fn into_iter(self) -> Self::IntoIter {
        self.errors.into_iter()
    }
}

impl<'a> IntoIterator for &'a ErrorReport {
    type Item = &'a Arc<Error>;

    type IntoIter = std::slice::Iter<'a, Arc<Error>>;

    fn into_iter(self) -> Self::IntoIter {
        self.errors.iter()
    }
}
