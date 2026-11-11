//! Typst mathematics, semantic vectors and source-aware operations.
//!
//! ```
//! use motion_formula::prelude::*;
//! use motion_core::{PixelSize, RasterLimits, Size, ValidationLimits};
//! use motion_render::RenderPrepare;
//! let compiler = FormulaCompiler::new();
//! let formula = compiler.compile("x + frac(1, y)", &FormulaOptions::default())?;
//! let selected = formula.select_subexpression("frac(1, y)")?;
//! assert!(selected.symbols().iter().any(|symbol| symbol.structural));
//! let next = compiler.compile("y + x", &FormulaOptions::default())?;
//! let morph = formula.morph_to(&next)?;
//! let node = morph.sample(0.5)?;
//! let frame = node
//!     .compile(
//!         Size::new(192.0, 64.0),
//!         PixelSize::new(192, 64),
//!         ValidationLimits::default(),
//!     )?
//!     .prepare(ValidationLimits::default(), RasterLimits::default())?
//!     .render_linear(RasterLimits::default())?;
//! assert!(frame.linear_pixels().any(|pixel| pixel[3] > 0.0));
//! # Ok::<(), motion_core::Error>(())
//! ```

use motion_content::VectorTree;

use motion_core::{Color, Size};
use std::ops::Range;
use std::sync::{Arc, Mutex};
use typst::syntax::{Source, Span};

use crate::frame;
pub use crate::frame::FormulaSymbol;
pub use crate::operations::{FormulaMorph, FormulaSelection};

mod asset;
mod cache;
mod diagnostics;
#[cfg(test)]
mod tests;

pub use asset::FormulaAsset;
pub use cache::FormulaCacheStats;
pub use diagnostics::{FormulaCompileError, FormulaDiagnostic};

use cache::Cache;

const MAX_SOURCE: usize = 8192;

/// Version of the Typst-to-vector interpretation, included in cache keys.
pub const FORMULA_BACKEND_VERSION: &str = "typst-0.15.1/motion-v1";

#[cfg(test)]
fn failure(message: impl Into<String>) -> motion_core::Error {
    motion_core::Error::formula(message)
}

pub(super) struct Input {
    pub(super) source: Source,
    pub(super) body: Range<usize>,
}

impl Input {
    #[cfg(test)]
    fn new(math: &str, size: f64) -> motion_core::Result<Self> {
        if math.trim().is_empty()
            || math.len() > MAX_SOURCE
            || !size.is_finite()
            || !(1.0..=1024.0).contains(&size)
        {
            return Err(failure(
                "Require 1..=8192 source bytes and finite size in 1..=1024 points",
            ));
        }
        let prefix = format!(
            "#set page(width:auto,height:auto,margin:0pt,fill:none)\n#set text(font:\"New Computer Modern Math\",size:{size}pt)\n$"
        );
        let body = prefix.len()..prefix.len() + math.len();
        let source = Source::detached(format!("{prefix}{math}$"));
        Ok(Self { source, body })
    }

    pub(super) fn range(&self, span: Span) -> Option<Range<usize>> {
        use typst::syntax::SpanKind;
        let range = match span.get() {
            SpanKind::Detached => return None,
            SpanKind::Range { id, range } => {
                if id != self.source.id() {
                    return None;
                }
                range
            }
            SpanKind::Number { id, num } => {
                if id != self.source.id() {
                    return None;
                }
                self.source.range(num, None)?
            }
        };
        if range.start >= self.body.start && range.end <= self.body.end {
            Some(range.start - self.body.start..range.end - self.body.start)
        } else {
            None
        }
    }

    fn diagnostic_range(&self, span: typst::syntax::DiagSpan) -> Option<Range<usize>> {
        use typst::syntax::DiagSpanKind;
        let range = match span.get() {
            DiagSpanKind::Detached => return None,
            DiagSpanKind::Range { id, range } => {
                if id != self.source.id() {
                    return None;
                }
                range
            }
            DiagSpanKind::Number { id, num, sub_range } => {
                if id != self.source.id() {
                    return None;
                }
                self.source.range(num, sub_range)?
            }
        };
        if range.end < self.body.start || range.start > self.body.end {
            None
        } else {
            Some(
                range.start.max(self.body.start) - self.body.start
                    ..range.end.min(self.body.end) - self.body.start,
            )
        }
    }
}

/// Compiler options; math input is Typst syntax without surrounding dollar signs.
#[derive(Clone, Debug)]
pub struct FormulaOptions {
    /// Em size in Typst points; output coordinates use 96 pixels/inch.
    pub size: f64,
    /// Explicit mathematical font family in the compiler's frozen font book.
    pub family: Arc<str>,
    /// Default straight encoded-sRGB fill; per-expression Typst fills may override it.
    pub fill: Color,
    /// Caller-owned semantic revision, isolating cached resources after updates.
    pub revision: u64,
}

impl Default for FormulaOptions {
    fn default() -> Self {
        Self {
            size: 24.0,
            family: "New Computer Modern Math".into(),
            fill: Color::WHITE,
            revision: 0,
        }
    }
}

struct Compiler {
    typst: crate::TypstCompiler,
    environment: u64,
    cache: Mutex<Cache>,
}

/// Thread-safe frozen Typst environment and bounded single-initialization cache.
/// Compile trusted source outside frame callbacks: source/output bounds are not
/// a hard limit on Typst evaluation time, temporary memory or internal memoization.
#[derive(Clone)]
pub struct FormulaCompiler {
    inner: Arc<Compiler>,
}

impl Default for FormulaCompiler {
    fn default() -> Self {
        Self::new()
    }
}

impl FormulaCompiler {
    /// Loads bundled mathematical fonts without filesystem/system discovery.
    pub fn new() -> Self {
        Self::from_typst(crate::TypstCompiler::new())
    }

    /// Adds explicitly frozen font bytes to bundled math fonts in a new namespace.
    pub fn with_fonts(fonts: &motion_content::FontLibrary) -> motion_core::Result<Self> {
        Ok(Self::from_typst(
            crate::TypstCompiler::new().with_fonts(fonts)?,
        ))
    }

    /// Uses an explicitly configured frozen Typst environment; clones share cache.
    pub fn from_typst(typst: crate::TypstCompiler) -> Self {
        use std::sync::atomic::{AtomicU64, Ordering};

        static NEXT: AtomicU64 = AtomicU64::new(1);
        Self {
            inner: Arc::new(Compiler {
                typst,
                environment: NEXT.fetch_add(1, Ordering::Relaxed),
                cache: Mutex::new(Cache::default()),
            }),
        }
    }

    /// Unique process-local environment identity (not a content/randomness seed).
    pub fn environment(&self) -> u64 {
        self.inner.environment
    }

    /// Compiles Typst math syntax directly to vectors and original spans.
    /// At most 32 entries are retained; eviction never invalidates pinned results.
    pub fn compile(
        &self,
        math: impl Into<Arc<str>>,
        options: &FormulaOptions,
    ) -> motion_core::Result<Arc<FormulaAsset>> {
        self.compile_cached(math, options, None)
    }

    fn compile_once(
        &self,
        math: Arc<str>,
        options: &FormulaOptions,
    ) -> Result<Arc<FormulaAsset>, FormulaCompileError> {
        let rgba = options.fill.to_rgba_float();
        let prefix = format!(
            "#set page(width:auto,height:auto,margin:0pt,fill:none)\n#set text(font:\"{}\",size:{}pt,fill:rgb({}%,{}%,{}%,{}%))\n#show math.equation: set text(font:\"{}\")\n$",
            options.family,
            options.size,
            rgba[0] * 100.0,
            rgba[1] * 100.0,
            rgba[2] * 100.0,
            rgba[3] * 100.0,
            options.family
        );
        let body = prefix.len()..prefix.len() + math.len();
        let input = Input {
            source: Source::detached(format!("{prefix}{math}$")),
            body,
        };
        let compiled = self.inner.typst.layout(input.source.clone());
        let convert = |diagnostic: &typst::diag::SourceDiagnostic, warning| FormulaDiagnostic {
            source: input.diagnostic_range(diagnostic.span),
            message: diagnostic.message.to_string().into(),
            warning,
        };
        let document = compiled.output.map_err(|diagnostics| FormulaCompileError {
            diagnostics: diagnostics
                .iter()
                .map(|diagnostic| convert(diagnostic, false))
                .collect::<Vec<_>>()
                .into(),
        })?;
        if document.pages().len() != 1 {
            return Err(FormulaCompileError::new(
                "FormulaAsset must produce exactly one finite page",
            ));
        }
        let page = &document.pages()[0];
        let size = Size::new(
            page.frame.width().to_pt() * 4.0 / 3.0,
            page.frame.height().to_pt() * 4.0 / 3.0,
        );
        if !size.width.is_finite()
            || !size.height.is_finite()
            || size.width <= 0.0
            || size.height <= 0.0
            || size.width > 16384.0
            || size.height > 16384.0
        {
            return Err(FormulaCompileError::new(
                "FormulaAsset viewport exceeds finite bounds",
            ));
        }
        let (root, symbols, baseline) = frame::convert(&page.frame, &input)?;
        let mut tree = VectorTree::new(size.to_rect(), root)?;
        if let Some(baseline) = baseline {
            tree = tree.with_baseline(baseline);
        }
        Ok(Arc::new(FormulaAsset {
            source: math,
            tree: Arc::new(tree),
            symbols: symbols.into(),
            warnings: compiled
                .warnings
                .iter()
                .map(|diagnostic| convert(diagnostic, true))
                .collect::<Vec<_>>()
                .into(),
            environment: self.environment(),
            revision: options.revision,
            baseline,
        }))
    }
}
