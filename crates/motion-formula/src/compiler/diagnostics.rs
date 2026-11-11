//! Structured formula diagnostics addressed to original math source ranges.

use motion_core::ValidationError;
use std::ops::Range;
use std::sync::Arc;

/// A diagnostic addressed to original UTF-8 math source, never generated preamble.
#[derive(Clone, Debug)]
pub struct FormulaDiagnostic {
    /// Original source interval; None denotes an environment/output error.
    pub source: Option<Range<usize>>,
    /// Compiler or conversion explanation.
    pub message: Arc<str>,
    /// Nonfatal compiler warning instead of a rejected formula.
    pub warning: bool,
}

/// Recoverable structured formula failure; available through Error::downcast_ref.
#[derive(Clone, Debug)]
pub struct FormulaCompileError {
    pub(super) diagnostics: Arc<[FormulaDiagnostic]>,
}

impl FormulaCompileError {
    /// Original diagnostics including source ranges and severity.
    pub fn diagnostics(&self) -> &[FormulaDiagnostic] {
        &self.diagnostics
    }

    pub(crate) fn new(message: impl Into<Arc<str>>) -> Self {
        Self {
            diagnostics: Arc::from([FormulaDiagnostic {
                source: None,
                message: message.into(),
                warning: false,
            }]),
        }
    }
}

impl std::fmt::Display for FormulaCompileError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        for (index, diagnostic) in self.diagnostics.iter().enumerate() {
            if index > 0 {
                formatter.write_str("; ")?;
            }
            if let Some(source) = &diagnostic.source {
                write!(formatter, "{}..{}: ", source.start, source.end)?;
            }
            formatter.write_str(&diagnostic.message)?;
        }
        Ok(())
    }
}

impl std::error::Error for FormulaCompileError {}

impl From<FormulaCompileError> for motion_core::Error {
    fn from(error: FormulaCompileError) -> Self {
        Self::formula_with_source(error.to_string(), error)
    }
}

impl From<ValidationError> for FormulaCompileError {
    fn from(error: ValidationError) -> Self {
        Self::new(error.to_string())
    }
}
