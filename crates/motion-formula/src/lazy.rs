//! Deferred mathematical content using the shared frozen formula compiler.

use crate::{FormulaAsset, FormulaCompileError, FormulaCompiler, FormulaOptions};
use motion_content::{CompileCache, VectorSelection};
use motion_core::{Color, DeferredContent, DrawContext, Node, ValidationError};
use std::{
    ops::Range,
    sync::{Arc, OnceLock},
};

#[derive(Clone)]
enum Selection {
    Source(Arc<str>, Arc<dyn Fn(Node) -> Node + Send + Sync>),
    Symbols(Range<usize>, Arc<dyn Fn(Node) -> Node + Send + Sync>),
}

/// Everything that determines a formula compilation.
#[derive(Clone, Debug, PartialEq)]
struct CompileKey {
    source: Arc<str>,
    options: FormulaOptions,
}

type Compilation = CompileCache<CompileKey, Result<Arc<FormulaAsset>, FormulaCompileError>>;

fn compile_key(key: &CompileKey) -> Result<Arc<FormulaAsset>, FormulaCompileError> {
    static COMPILER: OnceLock<FormulaCompiler> = OnceLock::new();
    COMPILER
        .get_or_init(FormulaCompiler::new)
        .compile(key.source.clone(), &key.options)
        .map_err(|error| {
            error
                .downcast_ref::<FormulaCompileError>()
                .cloned()
                .unwrap_or_else(|| FormulaCompileError::new(error.to_string()))
        })
}

/// Lazy Typst formula; compilation and selection errors occur at scene compilation.
#[derive(Clone)]
pub struct Formula {
    compilation: Compilation,
    progress: Option<f64>,
    selections: Vec<Selection>,
}

impl std::fmt::Debug for Formula {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("Formula")
            .field("source", &self.compilation.key().source)
            .field("options", &self.compilation.key().options)
            .field("progress", &self.progress)
            .field("selections", &self.selections.len())
            .finish()
    }
}

fn invalid(message: impl Into<String>) -> ValidationError {
    ValidationError::invalid("formula", "formula", message)
}

impl Formula {
    /// Retains Typst math without initializing the compiler or reading fonts.
    pub fn typst(source: impl Into<Arc<str>>) -> Self {
        Self {
            compilation: Compilation::new(CompileKey {
                source: source.into(),
                options: FormulaOptions::default(),
            }),
            progress: None,
            selections: Vec::new(),
        }
    }

    /// Sets logical Typst point size, validated when compiled.
    pub fn size(mut self, size: f64) -> Self {
        self.compilation = self.compilation.map_key(|key| key.options.size = size);
        self
    }

    /// Sets the default source fill, preserving selected authored transforms.
    pub fn fill(mut self, fill: Color) -> Self {
        self.compilation = self.compilation.map_key(|key| key.options.fill = fill);
        self
    }

    /// Defers cumulative vector handwriting; progress is validated when compiled.
    pub fn write(mut self, progress: f64) -> Self {
        self.progress = Some(progress);
        self
    }

    /// Maps all original Typst occurrences of a literal subexpression.
    pub fn select_subexpression(
        mut self,
        source: impl Into<Arc<str>>,
        map: impl Fn(Node) -> Node + Send + Sync + 'static,
    ) -> Self {
        self.selections
            .push(Selection::Source(source.into(), Arc::new(map)));
        self
    }

    /// Maps a half-open range of source-ordered rendered symbol occurrences.
    pub fn select_symbols(
        mut self,
        range: Range<usize>,
        map: impl Fn(Node) -> Node + Send + Sync + 'static,
    ) -> Self {
        self.selections
            .push(Selection::Symbols(range, Arc::new(map)));
        self
    }

    /// Explicitly requests cached compiled metadata without rendering a frame.
    pub fn asset(&self) -> motion_core::Result<Arc<FormulaAsset>> {
        self.compiled().map_err(Into::into)
    }

    fn compiled(&self) -> Result<Arc<FormulaAsset>, FormulaCompileError> {
        self.compilation.get_or_compile(0, compile_key)
    }

    fn node(&self, context: &DrawContext) -> Result<Node, ValidationError> {
        if self.selections.len() > 1024 {
            return Err(invalid("Formula exceeds 1024 authored selections"));
        }
        let compiled = if let Some(cache) = context.render_cache() {
            static COMPILER: OnceLock<FormulaCompiler> = OnceLock::new();
            COMPILER
                .get_or_init(FormulaCompiler::new)
                .compile_cached(
                    self.compilation.key().source.clone(),
                    &self.compilation.key().options,
                    Some(cache.partition(motion_core::CachePartition::Formula)),
                )
                .map_err(|error| invalid(error.to_string()))?
        } else {
            self.compiled()
                .map_err(|error| invalid(error.to_string()))?
        };
        let mut selections = Vec::new();
        for selected in &self.selections {
            let (selection, modify) = match selected {
                Selection::Source(source, modify) => (
                    compiled
                        .select_subexpression(source)
                        .map_err(|error| invalid(error.to_string()))?,
                    modify,
                ),
                Selection::Symbols(range, modify) => {
                    if range.start >= range.end || range.end > compiled.symbols().len() {
                        return Err(invalid("Formula symbol range is out of bounds"));
                    }
                    (
                        compiled
                            .selection(range.clone().collect())
                            .map_err(|error| invalid(error.to_string()))?,
                        modify,
                    )
                }
            };
            selections.push(VectorSelection::shared(selection.addresses(), modify));
        }
        compiled.vector_tree().reveal(&selections, self.progress)
    }
}

impl DeferredContent for Formula {
    fn resolve(&self, context: &DrawContext) -> Result<Node, ValidationError> {
        self.node(context)
    }

    fn baseline(&self, _: &DrawContext) -> Option<f64> {
        self.compiled().ok().and_then(|formula| formula.baseline())
    }
}

impl From<Formula> for Node {
    fn from(formula: Formula) -> Node {
        Node::deferred(formula)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use motion_core::{NodeModifiers, PixelSize, Size, ValidationLimits};

    #[test]
    fn formula_builders_compile_lazily_and_keep_cached_provenance_and_selection() {
        let formula = Formula::typst("x^2 + x")
            .size(24.0)
            .fill(Color::WHITE)
            .write(0.5)
            .select_subexpression("x", |node| node.opacity(0.5));
        assert!(!formula.compilation.is_compiled());
        let first = formula.asset().unwrap();
        assert!(Arc::ptr_eq(&first, &formula.asset().unwrap()));
        let list = Node::from(formula)
            .named("equation")
            .compile(
                Size::new(160.0, 64.0),
                PixelSize::new(160, 64),
                ValidationLimits::default(),
            )
            .unwrap();
        assert!(!list.commands.is_empty());
        let invalid = Formula::typst("");
        assert!(!invalid.compilation.is_compiled());
        let error = Node::from(invalid)
            .named("bad-equation")
            .compile(
                Size::new(160.0, 64.0),
                PixelSize::new(160, 64),
                ValidationLimits::default(),
            )
            .unwrap_err();
        assert!(error.path.contains("bad-equation"));
        assert_eq!(error.field, "formula");
    }

    #[test]
    fn lazy_formula_output_errors_keep_formula_kind_node_name_and_frame() {
        use motion_core::{Scene, SceneContext, Time, TimeLiteral, Timeline};
        let timeline = Timeline::new(Size::new(160.0, 64.0), motion_core::fps!(30))
            .track(|track| {
                track.push(
                    (|_: &SceneContext<'_>| Node::from(Formula::typst("")).named("invalid-math"))
                        .with_duration(1.seconds()),
                )
            })
            .build()
            .unwrap();
        let error = motion_render::render_still(
            &timeline,
            Time::ZERO,
            &motion_render::RenderOptions::default(),
        )
        .err()
        .unwrap();
        assert_eq!(error.kind(), motion_core::ErrorKind::Formula);
        assert_eq!(error.frame(), Some(0));
        assert!(error.to_string().contains("invalid-math"));
    }
}
