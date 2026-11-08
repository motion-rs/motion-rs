//! Backend-independent scene diagnostics and bounded structural traversal.

use std::fmt;

/// A machine-readable reason for scene preflight failure.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub enum ValidationErrorKind {
    /// A property is nonfinite, out of range, or otherwise invalid.
    InvalidValue,
    /// The scene exceeds its node or nesting-depth allowance.
    StructuralLimit,
    /// A required rendering capability is unavailable.
    Unsupported,
}

/// A property-level diagnostic, independent of fonts, GPU initialization and I/O.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ValidationError {
    /// Category suitable for programmatic handling.
    pub kind: ValidationErrorKind,
    /// Structural path, such as `layers[2].mask.group.layers[0]`.
    pub path: String,
    /// The invalid property or exhausted allowance.
    pub field: String,
    /// The specific constraint that failed.
    pub message: String,
    /// The requested frame index, when supplied by composition preflight or rendering.
    pub frame: Option<u32>,
}

impl ValidationError {
    /// Creates a property diagnostic without binding it to an output frame.
    pub fn new(
        kind: ValidationErrorKind,
        path: impl Into<String>,
        field: impl Into<String>,
        message: impl Into<String>,
    ) -> Self {
        Self {
            kind,
            path: path.into(),
            field: field.into(),
            message: message.into(),
            frame: None,
        }
    }

    /// Creates an invalid-value diagnostic for a rejected property.
    pub fn invalid(
        path: impl Into<String>,
        field: impl Into<String>,
        message: impl Into<String>,
    ) -> Self {
        Self::new(ValidationErrorKind::InvalidValue, path, field, message)
    }

    /// Creates a structural, work or storage allowance diagnostic.
    pub fn limit(
        path: impl Into<String>,
        field: impl Into<String>,
        message: impl Into<String>,
    ) -> Self {
        Self::new(ValidationErrorKind::StructuralLimit, path, field, message)
    }

    /// Creates a diagnostic for input that is valid but not supported.
    pub fn unsupported(
        path: impl Into<String>,
        field: impl Into<String>,
        message: impl Into<String>,
    ) -> Self {
        Self::new(ValidationErrorKind::Unsupported, path, field, message)
    }
}

impl fmt::Display for ValidationError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        if let Some(frame) = self.frame {
            write!(formatter, "frame {frame}: ")?;
        }
        write!(formatter, "{}.{}: {}", self.path, self.field, self.message)
    }
}

impl std::error::Error for ValidationError {}

/// Scene structure allowances, separate from pixel and kernel-work budgets.
///
/// Defaults allow 100,000 layers and 64 levels. All content branches and masks
/// count, including invisible layers and inactive transition endpoints. The root
/// layers have depth one. Custom limits must be positive, with at most 1,000,000
/// nodes and 128 levels, so preflight itself always has bounded recursion.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ValidationLimits {
    /// Maximum total layer count across all branches.
    pub max_nodes: usize,
    /// Maximum layer nesting depth, including masks.
    pub max_depth: usize,
}

impl Default for ValidationLimits {
    fn default() -> Self {
        Self {
            max_nodes: 100_000,
            max_depth: 64,
        }
    }
}

/// Structural requirements measured without initializing a renderer.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct SceneStats {
    /// Number of logical layers, including masks, all temporal samples and the
    /// wrapper/primitive charges of retained particle batches.
    pub nodes: usize,
    /// Deepest layer, or zero for an empty scene.
    pub max_depth: usize,
}

/// A bounded structural traversal shared by compilation and preparation.
pub struct ValidationState {
    limits: ValidationLimits,
    /// Cumulative node/depth statistics for the currently validated structure.
    pub stats: SceneStats,
}

impl ValidationState {
    /// Validates allowances and starts an empty traversal.
    pub fn new(limits: ValidationLimits) -> Result<Self, ValidationError> {
        if limits.max_nodes == 0
            || limits.max_nodes > 1_000_000
            || limits.max_depth == 0
            || limits.max_depth > 128
        {
            return Err(ValidationError::new(
                ValidationErrorKind::InvalidValue,
                "scene",
                "limits",
                "Require 1..=1000000 nodes and 1..=128 levels",
            ));
        }
        Ok(Self {
            limits,
            stats: SceneStats::default(),
        })
    }

    /// Charges one structural node before descending into its children.
    pub fn enter(&mut self, path: &str, depth: usize) -> Result<(), ValidationError> {
        self.enter_with(|| path.to_owned(), depth)
    }

    /// Like [`Self::enter`], but formats the path only when a limit is exceeded.
    pub fn enter_with(
        &mut self,
        path: impl FnOnce() -> String,
        depth: usize,
    ) -> Result<(), ValidationError> {
        if depth > self.limits.max_depth || self.stats.nodes >= self.limits.max_nodes {
            let (field, allowed) = if depth > self.limits.max_depth {
                ("max_depth", self.limits.max_depth)
            } else {
                ("max_nodes", self.limits.max_nodes)
            };
            return Err(ValidationError::limit(
                path(),
                field,
                format!("Scene exceeds {field}={allowed}"),
            ));
        }
        self.stats.nodes += 1;
        self.stats.max_depth = self.stats.max_depth.max(depth);
        Ok(())
    }
}
