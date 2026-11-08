//! Trusted Typst markup and math compiled directly into shared vector content.

#![forbid(unsafe_code)]

#[cfg(feature = "formula")]
mod compiler;
#[cfg(feature = "formula")]
mod lazy;

#[cfg(feature = "formula")]
pub use lazy::Formula;

#[cfg(feature = "formula")]
mod frame;
#[cfg(feature = "formula")]
mod operations;
mod typst_asset;

#[cfg(feature = "formula")]
pub use compiler::{
    FORMULA_BACKEND_VERSION, FormulaAsset, FormulaCacheStats, FormulaCompileError, FormulaCompiler,
    FormulaDiagnostic, FormulaMorph, FormulaOptions, FormulaSelection, FormulaSymbol,
};
pub use typst_asset::{TypstCompiler, TypstDocument};

/// Shared content and formula authoring operations.
pub mod prelude {
    #[cfg(feature = "formula")]
    pub use crate::Formula;
    #[cfg(feature = "formula")]
    pub use crate::{FormulaAsset, FormulaCompiler, FormulaOptions};
    pub use crate::{TypstCompiler, TypstDocument};
}
