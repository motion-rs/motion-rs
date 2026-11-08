//! Shape morphing between vector trees by matching filled parts.

use super::*;
use crate::MorphPath;
use motion_core::Shape;

#[derive(Clone, Debug)]
struct Part {
    address: Vec<usize>,
    world: Path2D,
    inverse: Transform,
    paint: Paint,
    rule: FillRule,
    key: Option<String>,
    role: u8,
    area: f64,
}

#[derive(Clone, Debug)]
struct Pair {
    source: Option<Part>,
    target: Option<Part>,
    from: MorphPath,
    to: MorphPath,
}

/// Prepared, replayable semantic tree morph. Leaf IDs take priority, followed by
/// position and signed-curve-area magnitude. Unmatched parts grow/shrink at their
/// own center. Fill/stroke/image geometry shares MorphPath's original curves.
/// Spatial paints and differing clipping/isolation crossfade the two retained
/// hierarchies; solid colors interpolate in Oklab on the morphed geometry.
#[derive(Clone, Debug)]
pub struct Morph {
    source: VectorTree,
    target: VectorTree,
    pairs: Arc<[Pair]>,
}

fn curve_area(path: &Path2D) -> f64 {
    let mut curve = kurbo::BezPath::new();
    for verb in path.verbs() {
        match *verb {
            motion_core::PathVerb::MoveTo(point) => curve.move_to((point.x, point.y)),
            motion_core::PathVerb::LineTo(point) => curve.line_to((point.x, point.y)),
            motion_core::PathVerb::QuadraticTo(control, end) => {
                curve.quad_to((control.x, control.y), (end.x, end.y))
            }
            motion_core::PathVerb::CubicTo(first, second, end) => {
                curve.curve_to((first.x, first.y), (second.x, second.y), (end.x, end.y))
            }
            motion_core::PathVerb::Close => curve.close_path(),
        }
    }
    kurbo::Shape::area(&curve).abs()
}

fn parts(tree: &VectorTree) -> Result<Vec<Part>, ValidationError> {
    let mut result = Vec::new();
    for leaf in tree.paint_leaves() {
        let inverse = leaf.transform.inverse().ok_or_else(|| {
            invalid(
                "transform",
                "Tree morph requires invertible local-to-tree matrices",
            )
        })?;
        let id = tree
            .ids()
            .iter()
            .filter(|(_, address)| !address.is_empty() && leaf.address.starts_with(address))
            .max_by_key(|(_, address)| address.len())
            .map(|(id, address)| format!("{id}/{:?}", &leaf.address[address.len()..]));
        let mut append = |geometry: Path2D,
                          paint: Paint,
                          rule: FillRule,
                          role: u8|
         -> Result<(), ValidationError> {
            let world = geometry.transformed(&leaf.transform)?;
            let area = curve_area(&world);
            if !area.is_finite() {
                return Err(invalid("area", "Curve area overflow"));
            }
            result.push(Part {
                address: leaf.address.clone(),
                world,
                inverse,
                paint,
                rule,
                key: id.as_ref().map(|id| format!("{id}:{role}")),
                role,
                area,
            });
            if result.len() > 1024 {
                return Err(limited("Morph admits at most 1024 painted parts per tree"));
            }
            Ok(())
        };
        match leaf.item {
            VectorItem::Path(path) => {
                if let Some((paint, rule)) = &path.fill {
                    append(path.path.as_ref().clone(), paint.clone(), *rule, 0)?;
                }
                if let Some((style, paint)) = &path.stroke {
                    append(
                        style.outline(&path.path, "vector_tree.morph.stroke")?,
                        paint.clone(),
                        FillRule::NonZero,
                        1,
                    )?;
                }
            }
            VectorItem::Image(image) => {
                if image.rect.width() > 0.0 && image.rect.height() > 0.0 {
                    append(
                        Path2D::from_rect(image.rect)?,
                        image.pattern().into(),
                        FillRule::NonZero,
                        2,
                    )?;
                }
            }
            VectorItem::Group(_) => unreachable!(),
        }
    }
    Ok(result)
}

impl Morph {
    /// Prepares bounded leaf matching and exact cubic path pairs once. Input
    /// trees remain immutable. Up to 1024 painted parts per side are admitted.
    pub fn new(source: &VectorTree, target: &VectorTree) -> Result<Self, ValidationError> {
        let sources = parts(source)?;
        let targets = parts(target)?;
        let mut used = vec![false; targets.len()];
        let mut pairs = Vec::new();
        let reserved = sources
            .iter()
            .filter_map(|part| part.key.clone())
            .collect::<std::collections::BTreeSet<_>>();
        let scale = source
            .view_box()
            .width()
            .max(source.view_box().height())
            .max(target.view_box().width())
            .max(target.view_box().height())
            .max(1.0);
        for source in sources {
            let exact = source.key.as_ref().and_then(|key| {
                targets
                    .iter()
                    .enumerate()
                    .find(|(index, target)| !used[*index] && target.key.as_ref() == Some(key))
                    .map(|(index, _)| index)
            });
            let target = exact.or_else(|| {
                targets
                    .iter()
                    .enumerate()
                    .filter(|(index, target)| {
                        !used[*index]
                            && !target
                                .key
                                .as_ref()
                                .is_some_and(|key| reserved.contains(key))
                    })
                    .min_by(|(_, first), (_, second)| {
                        let cost = |target: &Part| {
                            let source_center = source.world.bounding_box().center();
                            let target_center = target.world.bounding_box().center();
                            let horizontal = (source_center.x - target_center.x) / scale;
                            let vertical = (source_center.y - target_center.y) / scale;
                            horizontal * horizontal
                                + vertical * vertical
                                + (source.area - target.area).abs()
                                    / (source.area + target.area).max(f64::MIN_POSITIVE)
                                + if source.role == target.role { 0.0 } else { 2.0 }
                        };
                        cost(first).total_cmp(&cost(second))
                    })
                    .map(|(index, _)| index)
            });
            let target = target.map(|index| {
                used[index] = true;
                targets[index].clone()
            });
            let (from, to) = MorphPath::pair(
                source.world.clone(),
                target
                    .as_ref()
                    .map_or_else(Path2D::new, |part| part.world.clone()),
            )?;
            pairs.push(Pair {
                source: Some(source),
                target,
                from,
                to,
            });
        }
        for (index, target) in targets.into_iter().enumerate() {
            if !used[index] {
                let (from, to) = MorphPath::pair(Path2D::new(), target.world.clone())?;
                pairs.push(Pair {
                    source: None,
                    target: Some(target),
                    from,
                    to,
                });
            }
        }
        Ok(Self {
            source: source.clone(),
            target: target.clone(),
            pairs: pairs.into(),
        })
    }

    /// Evaluates without pairing again, I/O or history. Exact endpoints preserve
    /// the original trees; finite progress clamps to 0..=1. Group opacity is not
    /// multiplied independently into overlapping leaves.
    pub fn sample(&self, progress: f64) -> Result<Node, ValidationError> {
        if !progress.is_finite() {
            return Err(invalid("progress", "Morph progress must be finite"));
        }
        let progress = progress.clamp(0.0, 1.0);
        if progress == 0.0 || Arc::ptr_eq(&self.source.data, &self.target.data) {
            return Ok(self.source.node());
        }
        if progress == 1.0 {
            return Ok(self.target.node());
        }
        let mut source = BTreeMap::<Vec<usize>, Vec<Node>>::new();
        let mut target = BTreeMap::<Vec<usize>, Vec<Node>>::new();
        for pair in self.pairs.iter() {
            let geometry = pair.from.try_interpolate(&pair.to, progress)?.path();
            let color = match (&pair.source, &pair.target) {
                (Some(source), Some(target)) => match (&source.paint, &target.paint) {
                    (Paint::Solid(source), Paint::Solid(target)) => {
                        Some(Paint::Solid(source.interpolate(target, progress)))
                    }
                    _ => None,
                },
                _ => None,
            };
            for (part, output) in [(&pair.source, &mut source), (&pair.target, &mut target)] {
                if let Some(part) = part {
                    let path = VectorPath {
                        id: None,
                        path: Arc::new(geometry.transformed(&part.inverse)?),
                        fill: Some((color.as_ref().unwrap_or(&part.paint).clone(), part.rule)),
                        stroke: None,
                        source: None,
                    };
                    output
                        .entry(part.address.clone())
                        .or_default()
                        .push(Node::from(path));
                }
            }
        }

        fn build(
            group: &VectorGroup,
            address: &mut Vec<usize>,
            parts: &BTreeMap<Vec<usize>, Vec<Node>>,
        ) -> Node {
            let children = group
                .children
                .iter()
                .enumerate()
                .map(|(index, item)| {
                    address.push(index);
                    let node = match item {
                        VectorItem::Group(child) => build(child, address, parts),
                        _ => Node::group(parts.get(address).cloned().unwrap_or_default()),
                    };
                    address.pop();
                    node
                })
                .collect::<Vec<_>>();
            group.decorate(children)
        }
        Node::weighted_sum([
            (
                build(self.source.root(), &mut Vec::new(), &source),
                1.0 - progress,
            ),
            (
                build(self.target.root(), &mut Vec::new(), &target),
                progress,
            ),
        ])
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use motion_core::{Color, PixelSize, RasterLimits, Size};
    use motion_render::RenderPrepare;

    fn tree(colors: &[Color], reverse: bool) -> Result<VectorTree, ValidationError> {
        let mut children = colors
            .iter()
            .enumerate()
            .map(|(index, color)| {
                let mut path = VectorPath::filled(
                    Path2D::from_rect(Rect::new(
                        1.0 + index as f64 * 5.0,
                        1.0,
                        5.0 + index as f64 * 5.0,
                        5.0,
                    ))
                    .unwrap(),
                    *color,
                );
                path.id = Some(format!("part-{index}").into());
                VectorItem::Path(path)
            })
            .collect::<Vec<_>>();
        if reverse {
            children.reverse();
        }
        VectorTree::new(
            Rect::new(0.0, 0.0, 20.0, 8.0),
            VectorGroup {
                opacity: 0.5,
                children,
                ..Default::default()
            },
        )
    }

    fn pixels(node: Node) -> motion_core::Result<motion_render::LinearFrame> {
        Ok(node
            .compile(
                Size::new(20.0, 8.0),
                PixelSize::new(20, 8),
                ValidationLimits::default(),
            )?
            .prepare(ValidationLimits::default(), RasterLimits::default())?
            .render_linear(RasterLimits::default())?)
    }

    #[test]
    fn tree_morph_pairs_ids_grows_missing_leaves_and_preserves_group_alpha()
    -> motion_core::Result<()> {
        let from = tree(&[Color::rgb(255, 0, 0), Color::WHITE], false)?;
        let to = tree(&[Color::rgb(0, 0, 255), Color::WHITE, Color::WHITE], true)?;
        let morph = Morph::new(&from, &to)?;
        assert_eq!(
            pixels(morph.sample(0.0)?)?.pixels(),
            pixels(from.node())?.pixels()
        );
        assert_eq!(
            pixels(morph.sample(1.0)?)?.pixels(),
            pixels(to.node())?.pixels()
        );
        let middle = pixels(morph.sample(0.5)?)?;
        assert_eq!(middle.pixel(2, 2).unwrap()[3], 0.5);
        assert!(middle.pixel(2, 2).unwrap()[0] > 0.0 && middle.pixel(2, 2).unwrap()[2] > 0.0);
        assert!(middle.pixel(12, 2).unwrap()[3] > 0.0);
        assert_eq!(middle.pixels(), pixels(morph.sample(0.5)?)?.pixels());
        let empty = VectorTree::new(from.view_box(), VectorGroup::default())?;
        assert!(
            pixels(Morph::new(&from, &empty)?.sample(1.0)?)?
                .pixels()
                .iter()
                .all(|pixel| *pixel == [0.0; 4])
        );
        assert!(morph.sample(f64::NAN).is_err());
        Ok(())
    }

    #[test]
    fn exact_ids_are_reserved_before_geometry_and_overlapping_groups_isolate_once()
    -> motion_core::Result<()> {
        let base = tree(&[Color::WHITE, Color::WHITE], false)?;
        let mut source = base.root().clone();
        if let VectorItem::Path(path) = &mut source.children[0] {
            path.id = Some("missing".into());
        }
        let source = VectorTree::new(base.view_box(), source)?;
        let mut target = base.root().clone();
        target.children.remove(0);
        let target = VectorTree::new(base.view_box(), target)?;
        let morph = Morph::new(&source, &target)?;
        assert!(morph.pairs[0].target.is_none());
        assert!(morph.pairs[1].target.is_some());
        let mut source = base.root().clone();
        source.children[1] = source.children[0].clone();
        if let VectorItem::Path(path) = &mut source.children[1] {
            path.id = Some("overlap".into());
        }
        let source = VectorTree::new(base.view_box(), source)?;
        let mut target = source.root().clone();
        let mut extra = target.children[0].clone();
        if let VectorItem::Path(path) = &mut extra {
            path.id = Some("extra".into());
        }
        target.children.push(extra);
        let target = VectorTree::new(base.view_box(), target)?;
        assert_eq!(
            pixels(Morph::new(&source, &target)?.sample(0.5)?)?
                .pixel(2, 2)
                .unwrap()[3],
            0.5
        );
        Ok(())
    }
}
