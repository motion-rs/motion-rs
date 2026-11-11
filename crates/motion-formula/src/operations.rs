//! Formula symbol selection and semantic morphing between compiled formulas.

#[cfg(test)]
use crate::{FormulaCompiler, FormulaOptions};

use crate::{FormulaAsset, FormulaCompileError, FormulaSymbol};

use motion_content::{CompileCache, VectorGroup, VectorItem, VectorTree};

use motion_core::{Canvas, DrawContext, Drawable, Node, NodeModifiers, Rect, ValidationError};
use std::collections::{BTreeMap, BTreeSet};
use std::{ops::Range, sync::Arc};

/// A checked immutable formula subset; original geometry, clipping, viewport and
/// baseline remain unchanged. Glyphs sharing a Typst cluster span stay together.
#[derive(Clone, Debug)]
pub struct FormulaSelection {
    subset: CompileCache<Subset, Result<Arc<VectorTree>, ValidationError>>,
}

/// The immutable key of a selected subtree: an asset and the symbols kept.
#[derive(Clone, Debug)]
struct Subset {
    formula: FormulaAsset,
    indices: Arc<[usize]>,
}

impl FormulaAsset {
    pub(crate) fn selection(&self, indices: Vec<usize>) -> motion_core::Result<FormulaSelection> {
        if indices.is_empty() {
            return Err(FormulaCompileError::new(
                "FormulaAsset selection matched no glyph or rule",
            )
            .into());
        }
        Ok(FormulaSelection {
            subset: CompileCache::new(Subset {
                formula: self.clone(),
                indices: indices.into(),
            }),
        })
    }

    /// Selects glyph/rule occurrences intersecting an original UTF-8 byte range.
    /// Ranges must be nonempty and lie on the input's character boundaries.
    pub fn select_source(&self, range: Range<usize>) -> motion_core::Result<FormulaSelection> {
        if range.start >= range.end
            || range.end > self.source.len()
            || !self.source.is_char_boundary(range.start)
            || !self.source.is_char_boundary(range.end)
        {
            return Err(FormulaCompileError::new(
                "Selection requires a nonempty UTF-8 source interval",
            )
            .into());
        }
        self.select_ranges(std::slice::from_ref(&range))
    }

    /// Selects an actual rendered symbol or its Typst source alias, including all
    /// repeated occurrences (for example alpha can select the rendered alpha).
    pub fn select_symbol(&self, text: &str) -> motion_core::Result<FormulaSelection> {
        if text.is_empty() {
            return Err(FormulaCompileError::new("Symbol selection cannot be empty").into());
        }
        self.selection(
            self.symbols
                .iter()
                .enumerate()
                .filter(|(_, symbol)| {
                    !symbol.structural
                        && (symbol.text.as_ref() == text
                            || symbol.character.as_ref() == text
                            || symbol
                                .source
                                .as_ref()
                                .and_then(|range| self.source.get(range.clone()))
                                == Some(text))
                })
                .map(|(index, _)| index)
                .collect(),
        )
    }

    /// Selects every occurrence of a literal subexpression in original math input,
    /// including structural rules whose compiler span intersects the expression.
    pub fn select_subexpression(&self, text: &str) -> motion_core::Result<FormulaSelection> {
        if text.is_empty() {
            return Err(FormulaCompileError::new("Subexpression selection cannot be empty").into());
        }
        let ranges = self
            .source
            .match_indices(text)
            .map(|(start, _)| start..start + text.len())
            .collect::<Vec<_>>();
        self.select_ranges(&ranges)
    }

    /// Structural rules need the whole span inside a range; rendered glyphs only need to overlap one.
    fn select_ranges(&self, ranges: &[Range<usize>]) -> motion_core::Result<FormulaSelection> {
        self.selection(
            self.symbols
                .iter()
                .enumerate()
                .filter(|(_, symbol)| {
                    symbol.source.as_ref().is_some_and(|source| {
                        ranges.iter().any(|range| {
                            if symbol.structural {
                                range.start <= source.start && source.end <= range.end
                            } else {
                                source.start < range.end && range.start < source.end
                            }
                        })
                    })
                })
                .map(|(index, _)| index)
                .collect(),
        )
    }

    /// Creates a prepared character-first morph. Repeated characters pair
    /// in original source order before remaining position/area matching.
    pub fn morph_to(&self, target: &FormulaAsset) -> Result<FormulaMorph, ValidationError> {
        FormulaMorph::new(self, target)
    }
}

impl FormulaSelection {
    /// Selected source-order occurrences (not a reshaped substring).
    pub fn symbols(&self) -> Vec<&FormulaSymbol> {
        let subset = self.subset.key();
        subset
            .indices
            .iter()
            .map(|index| &subset.formula.symbols[*index])
            .collect()
    }

    pub(crate) fn addresses(&self) -> Vec<Vec<usize>> {
        let subset = self.subset.key();
        subset
            .indices
            .iter()
            .filter_map(|index| {
                subset
                    .formula
                    .tree
                    .ids()
                    .get(&subset.formula.symbols[*index].id)
                    .cloned()
            })
            .collect()
    }

    /// Canonical selected vector subtree, without reshaping or changing glyph
    /// positions. Clones share a single bounded subset preparation.
    pub fn vector_tree(&self) -> Result<Arc<VectorTree>, ValidationError> {
        self.subset.get_or_compile(0, |subset| {
            let selected = self.addresses();

            fn prune(
                group: &VectorGroup,
                address: &mut Vec<usize>,
                selected: &[Vec<usize>],
            ) -> VectorGroup {
                let mut result = group.clone();
                result.children = group
                    .children
                    .iter()
                    .enumerate()
                    .filter_map(|(index, item)| {
                        address.push(index);
                        let kept = if selected
                            .iter()
                            .any(|selected| address.starts_with(selected))
                        {
                            Some(item.clone())
                        } else if selected
                            .iter()
                            .any(|selected| selected.starts_with(address))
                        {
                            match item {
                                VectorItem::Group(group) => {
                                    Some(VectorItem::Group(prune(group, address, selected)))
                                }
                                _ => None,
                            }
                        } else {
                            None
                        };
                        address.pop();
                        kept
                    })
                    .collect();
                result
            }
            let root = prune(subset.formula.tree.root(), &mut Vec::new(), &selected);
            let mut tree = VectorTree::new(subset.formula.tree.view_box(), root)?;
            if let Some(baseline) = subset.formula.baseline {
                tree = tree.with_baseline(baseline);
            }
            Ok(Arc::new(tree))
        })
    }

    /// Ordinary subset anchored in the original formula viewport.
    pub fn node(&self) -> Node {
        match self.vector_tree() {
            Ok(tree) => Node::group([
                tree.node(),
                Node::from(Bounds(self.subset.key().formula.tree.view_box())),
            ])
            .pivot(motion_core::Pivot::Point(
                self.subset.key().formula.tree.view_box().center(),
            )),
            Err(error) => motion_core::authoring::transition::diagnostic(error),
        }
    }

    /// Writes only selected original occurrences by their own cumulative length,
    /// without recompilation or changing retained source/viewport metrics.
    pub fn write(&self, progress: f64) -> Result<Node, ValidationError> {
        let tree = self.vector_tree()?;
        Ok(Node::group([
            tree.write(progress, motion_content::WriteOptions::default())?,
            Node::from(Bounds(self.subset.key().formula.tree.view_box())),
        ])
        .pivot(motion_core::Pivot::Point(
            self.subset.key().formula.tree.view_box().center(),
        )))
    }

    /// Applies a normal modifier callback to selected occurrence groups
    /// while leaving all other original formula content intact.
    pub fn apply(&self, callback: impl Fn(Node) -> Node) -> Node {
        let addresses = self.addresses().into_iter().collect::<BTreeSet<_>>();
        self.subset
            .key()
            .formula
            .tree
            .node_mapped(&|address, node| {
                if addresses.contains(address) {
                    callback(node)
                } else {
                    node
                }
            })
    }
}

#[derive(Clone)]
struct Bounds(Rect);

impl Drawable for Bounds {
    fn bounds(&self, _: &DrawContext) -> Result<Rect, ValidationError> {
        Ok(self.0)
    }

    fn draw(&self, _: &DrawContext, _: &mut Canvas<'_>) -> Result<(), ValidationError> {
        Ok(())
    }
}

/// Prepared character-first formula morph over the common Morph engine.
#[derive(Clone, Debug)]
pub struct FormulaMorph {
    from: FormulaAsset,
    to: FormulaAsset,
    geometry: motion_content::Morph,
    matches: Arc<[(Arc<str>, Arc<str>)]>,
}

type SemanticTree = (VectorTree, BTreeMap<Arc<str>, Arc<str>>);

fn semantic_tree(formula: &FormulaAsset) -> Result<SemanticTree, ValidationError> {
    let mut occurrences = BTreeMap::<Arc<str>, usize>::new();
    let mut names = BTreeMap::<Arc<str>, Option<Arc<str>>>::new();
    let mut keys = BTreeMap::new();
    for symbol in formula.symbols.iter() {
        if symbol.structural {
            names.insert(symbol.id.clone(), None);
            continue;
        }
        let occurrence = occurrences.entry(symbol.character.clone()).or_default();
        let encoded = symbol
            .character
            .as_bytes()
            .iter()
            .map(|byte| format!("{byte:02x}"))
            .collect::<String>();
        let key: Arc<str> = format!("character-{encoded}-{occurrence}").into();
        *occurrence += 1;
        names.insert(symbol.id.clone(), Some(key.clone()));
        keys.insert(key, symbol.id.clone());
    }

    fn rename(group: &mut VectorGroup, names: &BTreeMap<Arc<str>, Option<Arc<str>>>) {
        if let Some(id) = &group.id
            && let Some(replacement) = names.get(id)
        {
            group.id = replacement.clone();
        }
        for item in &mut group.children {
            if let VectorItem::Group(group) = item {
                rename(group, names);
            }
        }
    }
    let mut root = formula.tree.root().clone();
    rename(&mut root, &names);
    let mut tree = VectorTree::new(formula.tree.view_box(), root)?;
    if let Some(baseline) = formula.baseline {
        tree = tree.with_baseline(baseline);
    }
    Ok((tree, keys))
}

impl FormulaMorph {
    fn new(from: &FormulaAsset, to: &FormulaAsset) -> Result<Self, ValidationError> {
        let (source, source_keys) = semantic_tree(from)?;
        let (target, target_keys) = semantic_tree(to)?;
        let matches = source_keys
            .iter()
            .filter_map(|(key, source)| {
                target_keys
                    .get(key)
                    .map(|target| (source.clone(), target.clone()))
            })
            .collect::<Vec<_>>();
        Ok(Self {
            from: from.clone(),
            to: to.clone(),
            geometry: motion_content::Morph::new(&source, &target)?,
            matches: matches.into(),
        })
    }

    /// Exact rendered-character ID pairs reserved before geometric fallback.
    pub fn character_matches(&self) -> &[(Arc<str>, Arc<str>)] {
        &self.matches
    }

    /// Finite progress clamps to 0..=1; endpoints preserve the original formulas.
    pub fn sample(&self, progress: f64) -> Result<Node, ValidationError> {
        if !progress.is_finite() {
            return self.geometry.sample(progress);
        }
        if progress <= 0.0 {
            return Ok(self.from.node());
        }
        if progress >= 1.0 {
            return Ok(self.to.node());
        }
        self.geometry.sample(progress)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use motion_core::Size;
    use motion_render::RenderPrepare;

    fn pixels(node: Node) -> motion_core::Result<motion_render::FrameBuffer> {
        Ok(node
            .compile(
                Size::new(192.0, 64.0),
                motion_core::PixelSize::new(192, 64),
                motion_core::ValidationLimits::default(),
            )?
            .prepare(
                motion_core::ValidationLimits::default(),
                motion_core::RasterLimits::default(),
            )?
            .render_linear(motion_core::RasterLimits::default())?)
    }

    #[test]
    fn formula_selections_writing_and_character_first_morph_share_original_vectors()
    -> motion_core::Result<()> {
        let compiler = FormulaCompiler::new();
        let options = FormulaOptions::default();
        let from = compiler.compile("x + y + x + frac(1, y)", &options)?;
        let to = compiler.compile("y + x + z + x", &options)?;
        let selected = from.select_symbol("x")?;
        assert_eq!(selected.symbols().len(), 2);
        let subset = pixels(selected.node())?;
        let full = pixels(from.node())?;
        assert!(
            subset
                .linear_pixels()
                .filter(|pixel| pixel[3] > 0.0)
                .count()
                < full.linear_pixels().filter(|pixel| pixel[3] > 0.0).count()
        );
        assert!(from.select_subexpression("frac(1, y)")?.symbols().len() >= 2);
        assert!(
            from.select_source(0..1)?
                .symbols()
                .iter()
                .any(|symbol| symbol.character.as_ref() == "x")
        );
        assert!(from.select_symbol("not_present").is_err());
        let hidden = pixels(selected.apply(|node| node.opacity(0.0)))?;
        assert!(
            hidden
                .linear_pixels()
                .filter(|pixel| pixel[3] > 0.0)
                .count()
                < full.linear_pixels().filter(|pixel| pixel[3] > 0.0).count()
        );
        assert_eq!(pixels(from.write(1.0)?)?, full);
        assert!(
            pixels(from.write(0.0)?)?
                .linear_pixels()
                .all(|pixel| pixel == [0.0; 4])
        );
        let fraction = from.select_subexpression("frac(1, y)")?;
        assert!(fraction.symbols().iter().any(|symbol| symbol.structural));
        assert_eq!(pixels(fraction.write(1.0)?)?, pixels(fraction.node())?);
        assert!(
            pixels(selected.write(0.3)?)?
                .linear_pixels()
                .any(|pixel| pixel[3] > 0.0)
        );
        let morph = from.morph_to(&to)?;
        for (source, target) in morph.character_matches() {
            let source = from
                .symbols
                .iter()
                .find(|symbol| &symbol.id == source)
                .unwrap();
            let target = to
                .symbols
                .iter()
                .find(|symbol| &symbol.id == target)
                .unwrap();
            assert_eq!(source.character, target.character);
        }
        assert_eq!(
            morph
                .character_matches()
                .iter()
                .filter(|(id, _)| from
                    .symbols
                    .iter()
                    .find(|symbol| &symbol.id == id)
                    .unwrap()
                    .character
                    .as_ref()
                    == "x")
                .count(),
            2
        );
        assert_eq!(pixels(morph.sample(0.0)?)?, full);
        assert_eq!(pixels(morph.sample(1.0)?)?, pixels(to.node())?);
        assert_eq!(pixels(morph.sample(0.5)?)?, pixels(morph.sample(0.5)?)?);
        Ok(())
    }
}
