//! Compiled formula assets: shared vector geometry, symbols, warnings and version identity.

use super::{
    FORMULA_BACKEND_VERSION, FormulaCompiler, FormulaDiagnostic, FormulaOptions, FormulaSymbol,
};
use motion_content::{VectorGroup, VectorItem, VectorTree};
use motion_core::{Node, ValidationError};
use std::sync::{Arc, OnceLock};

/// Typst formula, preserving glyph/source provenance and shared geometry.
/// Compiled formula geometry and source-aware operations.
#[derive(Clone, Debug)]
pub struct FormulaAsset {
    pub(crate) source: Arc<str>,
    pub(crate) tree: Arc<VectorTree>,
    pub(crate) symbols: Arc<[FormulaSymbol]>,
    pub(super) warnings: Arc<[FormulaDiagnostic]>,
    pub(super) environment: u64,
    pub(super) revision: u64,
    pub(crate) baseline: Option<f64>,
}

impl FormulaAsset {
    /// Compiles Typst math using the shared default frozen compiler.
    pub fn typst(math: impl Into<Arc<str>>) -> motion_core::Result<Arc<Self>> {
        static COMPILER: OnceLock<FormulaCompiler> = OnceLock::new();
        COMPILER
            .get_or_init(FormulaCompiler::new)
            .compile(math, &FormulaOptions::default())
    }

    /// Original math body, excluding generated compiler settings.
    pub fn source(&self) -> &str {
        &self.source
    }

    /// Known vector/metadata payload, conservatively charging shared paths per
    /// occurrence. Compiler temporaries, allocator capacity and external clones
    /// are not a process-wide memory bound.
    pub fn retained_bytes(&self) -> usize {
        fn group_bytes(group: &VectorGroup) -> usize {
            let mut bytes = std::mem::size_of::<VectorGroup>()
                + group.id.as_ref().map_or(0, |id| id.len())
                + group.children.len() * std::mem::size_of::<VectorItem>();
            for item in &group.children {
                bytes += match item {
                    VectorItem::Group(group) => group_bytes(group),
                    VectorItem::Path(path) => std::mem::size_of_val(path.path.verbs()),
                    VectorItem::Image(image) => image.image.retained_bytes(),
                };
            }
            bytes
        }
        std::mem::size_of::<Self>()
            + self.source.len()
            + group_bytes(self.tree.root())
            + self
                .symbols
                .iter()
                .map(|symbol| {
                    std::mem::size_of::<FormulaSymbol>()
                        + symbol.id.len()
                        + symbol.text.len()
                        + symbol.character.len()
                        + symbol.family.len()
                })
                .sum::<usize>()
            + self
                .warnings
                .iter()
                .map(|diagnostic| {
                    std::mem::size_of::<FormulaDiagnostic>() + diagnostic.message.len()
                })
                .sum::<usize>()
    }

    /// Canonical immutable vectors, without SVG or raster conversion.
    pub fn vector_tree(&self) -> Arc<VectorTree> {
        self.tree.clone()
    }

    /// Original-source ordered glyph/rule occurrences.
    pub fn symbols(&self) -> &[FormulaSymbol] {
        &self.symbols
    }

    /// Nonfatal warnings with original ranges when available.
    pub fn warnings(&self) -> &[FormulaDiagnostic] {
        &self.warnings
    }

    /// Frozen compiler identity/revision used for this resource.
    pub fn version(&self) -> (&'static str, u64, u64) {
        (FORMULA_BACKEND_VERSION, self.environment, self.revision)
    }

    /// Real recorded frame/text baseline when available, never inferred from ink.
    pub fn baseline(&self) -> Option<f64> {
        self.baseline
    }

    /// Ordinary content for all rendering consumers.
    pub fn node(&self) -> Node {
        self.tree.node()
    }

    /// Common cumulative vector handwriting effect.
    pub fn write(&self, progress: f64) -> Result<Node, ValidationError> {
        self.tree
            .write(progress, motion_content::WriteOptions::default())
    }
}
