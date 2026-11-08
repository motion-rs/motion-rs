//! Handwriting-style write-on effects for vector trees.

use super::*;

/// Shared cumulative-length handwriting and fill handoff settings.
#[derive(Clone, Copy, Debug)]
pub struct WriteOptions {
    /// Temporary outline thickness in tree coordinates (not font units).
    pub width: f64,
    /// Fraction of progress reserved for drawing contours; the remainder hands
    /// over to the original fill/stroke/images. Must be strictly between 0 and 1.
    pub outline_fraction: f64,
}

impl Default for WriteOptions {
    fn default() -> Self {
        Self {
            width: 1.0,
            outline_fraction: 0.8,
        }
    }
}

impl VectorTree {
    /// Draws paths in source reading order by their cumulative transformed arc
    /// length, then fades temporary contours into the original artwork. Endpoints
    /// are exact; finite progress clamps to 0..=1. Noninvertible writing matrices
    /// and existing curve-stroke budget violations fail explicitly.
    pub fn write(&self, progress: f64, options: WriteOptions) -> Result<Node, ValidationError> {
        self.write_mapped(progress, options, &|_, node| node)
    }

    /// Applies handwriting while mapping original vector addresses to authored nodes.
    pub fn write_mapped(
        &self,
        progress: f64,
        options: WriteOptions,
        map: &impl Fn(&[usize], Node) -> Node,
    ) -> Result<Node, ValidationError> {
        if !progress.is_finite()
            || !options.width.is_finite()
            || options.width <= 0.0
            || !options.outline_fraction.is_finite()
            || !(0.0..1.0).contains(&options.outline_fraction)
            || options.outline_fraction == 0.0
        {
            return Err(invalid(
                "write",
                "Require finite progress, positive width and outline fraction in (0,1)",
            ));
        }
        let progress = progress.clamp(0.0, 1.0);
        if progress == 1.0 {
            return Ok(self.node_mapped(map));
        }
        let handoff = ((progress - options.outline_fraction) / (1.0 - options.outline_fraction))
            .clamp(0.0, 1.0);
        let mut paths = Vec::new();
        let mut total = 0.0;
        for leaf in self.leaves() {
            if let VectorItem::Path(path) = leaf.item
                && (path.fill.is_some() || path.stroke.is_some())
            {
                let world = path.path.transformed(&leaf.transform)?;
                let length = world.try_length()?;
                total += length;
                if !total.is_finite() {
                    return Err(invalid("length", "Accumulated vector length overflow"));
                }
                paths.push((leaf, world, length));
            }
        }
        let mut remaining = total * (progress / options.outline_fraction).min(1.0);
        let mut drawings = BTreeMap::new();
        for (leaf, world, length) in paths {
            let amount = if length == 0.0 {
                0.0
            } else {
                (remaining / length).clamp(0.0, 1.0)
            };
            remaining = (remaining - length).max(0.0);
            let VectorItem::Path(path) = leaf.item else {
                unreachable!()
            };
            let mut children = Vec::new();
            if amount > 0.0 && handoff < 1.0 {
                let inverse = leaf.transform.inverse().ok_or_else(|| {
                    invalid(
                        "transform",
                        "Writing requires an invertible local-to-tree matrix",
                    )
                })?;
                let outline = StrokeStyle {
                    width: options.width,
                    cap: motion_core::LineCap::Round,
                    join: motion_core::LineJoin::Round,
                }
                .outline(&world.try_trim(0.0, amount)?, "vector_tree.write")?
                .transformed(&inverse)?;
                let paint = path
                    .stroke
                    .as_ref()
                    .map(|(_, paint)| paint)
                    .or_else(|| path.fill.as_ref().map(|(paint, _)| paint))
                    .expect("painted path")
                    .clone();
                children
                    .push(Node::from(VectorPath::filled(outline, paint)).opacity(1.0 - handoff));
            }
            if handoff > 0.0 {
                children.push(Node::from(path.clone()).opacity(handoff));
            }
            drawings.insert(leaf.address, Node::group(children));
        }

        fn build(
            group: &VectorGroup,
            address: &mut Vec<usize>,
            drawings: &BTreeMap<Vec<usize>, Node>,
            handoff: f64,
            map: &impl Fn(&[usize], Node) -> Node,
        ) -> Node {
            let children = group
                .children
                .iter()
                .enumerate()
                .map(|(index, item)| {
                    address.push(index);
                    let node = match item {
                        VectorItem::Group(child) => build(child, address, drawings, handoff, map),
                        VectorItem::Path(_) => {
                            drawings.get(address).cloned().unwrap_or_else(Node::empty)
                        }
                        VectorItem::Image(_) => item_node(item).opacity(handoff),
                    };
                    let node = if matches!(item, VectorItem::Group(_)) {
                        node
                    } else {
                        map(address, node)
                    };
                    address.pop();
                    node
                })
                .collect::<Vec<_>>();
            map(address, group.decorate(children))
        }
        let mut node = build(self.root(), &mut Vec::new(), &drawings, handoff, map);
        if let Some(baseline) = self.data.baseline {
            node = Node::group([node, Node::from(TextBounds(self.view_box()))])
                .with_text_metrics(baseline, self.view_box());
        }
        Ok(node.pivot(motion_core::Pivot::Point(
            self.node().measure(ValidationLimits::default())?.center(),
        )))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use motion_core::{Color, PixelSize, RasterLimits, Size};
    use motion_render::RenderPrepare;

    fn pixels(node: Node) -> motion_core::Result<motion_render::LinearFrame> {
        Ok(node
            .compile(
                Size::new(16.0, 8.0),
                PixelSize::new(16, 8),
                ValidationLimits::default(),
            )?
            .prepare(ValidationLimits::default(), RasterLimits::default())?
            .render_linear(RasterLimits::default())?)
    }

    #[test]
    fn writing_uses_total_world_length_then_hands_off_to_exact_original() -> motion_core::Result<()>
    {
        let tree = VectorTree::new(
            Rect::new(0.0, 0.0, 16.0, 8.0),
            VectorGroup {
                children: vec![
                    VectorItem::Path(VectorPath::filled(
                        Path2D::from_rect(Rect::new(1.0, 1.0, 5.0, 5.0))?,
                        Color::WHITE,
                    )),
                    VectorItem::Path(VectorPath::filled(
                        Path2D::from_rect(Rect::new(7.0, 1.0, 15.0, 5.0))?,
                        Color::rgb(255, 0, 0),
                    )),
                ],
                ..Default::default()
            },
        )?;
        let options = WriteOptions::default();
        let early = pixels(tree.write(0.8 * 16.0 / 40.0, options)?)?;
        assert!(early.pixel(1, 1).unwrap()[3] > 0.0);
        assert_eq!(early.pixel(3, 3), Some([0.0; 4]));
        assert_eq!(early.pixel(7, 1), Some([0.0; 4]));
        assert!(pixels(tree.write(0.9, options)?)?.pixel(3, 3).unwrap()[3] > 0.0);
        assert_eq!(
            pixels(tree.write(1.0, options)?)?.pixels(),
            pixels(tree.node())?.pixels()
        );
        assert!(
            pixels(tree.write(0.0, options)?)?
                .pixels()
                .iter()
                .all(|pixel| *pixel == [0.0; 4])
        );
        assert!(tree.write(f64::NAN, options).is_err());
        Ok(())
    }
}
