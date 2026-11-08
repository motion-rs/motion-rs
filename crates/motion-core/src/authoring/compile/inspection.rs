//! Read-only node inspection for editor overlays, planned like compilation.

use super::*;

/// Read-only device-space layout metadata for editor overlays, in paint order.
/// This is not raster coverage; bounds picking is intentionally approximate.
#[derive(Clone, Debug)]
pub struct NodeInspection {
    /// Stable occurrence path, including authored names.
    pub path: String,
    /// Filter-expanded device-space visual bounds.
    pub bounds: crate::Rect,
    /// Composed local-to-device transform.
    pub transform: crate::Transform,
    /// Authored filter chain before device conversion.
    pub filters: Vec<crate::Filter>,
    /// Device-space clip bounding box, not exact clip coverage.
    pub clip_bounds: Option<crate::Rect>,
    /// Whether any local geometric/coverage clipping is present.
    pub clipped: bool,
    /// Whether the occurrence has an isolated content mask.
    pub masked: bool,
}

impl Node {
    /// Plans a bounded editor snapshot using the same layout, resources and z order
    /// as compilation. Drawables may resolve/record; no pixels or devices are used.
    /// Errors remain separate from normal rendering when used by an editor.
    pub fn inspect_with_context(
        &self,
        design_size: Size,
        output_size: PixelSize,
        limits: ValidationLimits,
        drawing: &DrawContext,
    ) -> Result<Vec<NodeInspection>, ValidationError> {
        let (root, scale) = output_size.mapping(design_size)?;
        let mut context = drawing.clone().with_validation_limits(limits);
        context.device_scale = scale;
        let mut state = ValidationState::new(limits)?;
        let planned = plan(self, "node", 1, &context, &mut state)?;
        let mut output = Vec::new();
        planned.inspect(root, &mut output)?;
        Ok(output)
    }
}

impl Plan {
    fn inspect(
        &self,
        parent: Transform,
        output: &mut Vec<NodeInspection>,
    ) -> Result<(), ValidationError> {
        let transform = self.transform.then(parent);
        let clip_bounds = self
            .rect_clip
            .map(|clip| mapped_bounds(clip.bounds, clip.transform.then(transform), &self.path))
            .transpose()?;
        output.push(NodeInspection {
            path: self.path.clone(),
            bounds: self.visual_bounds(parent)?.rect(&self.path)?,
            transform,
            filters: self.filters.clone(),
            clip_bounds,
            clipped: self.rect_clip.is_some() || self.clip.is_some() || self.shape_clip.is_some(),
            masked: self.mask.is_some(),
        });
        if let Content::Children(children) = &self.content {
            for child in children {
                child.inspect(transform, output)?;
            }
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{Color, NodeModifiers, Rect};

    #[test]
    fn inspection_uses_compiled_layout_names_mapping_and_order() {
        let node = Node::group([
            Node::from(Rect::new(0.0, 0.0, 10.0, 10.0).fill(Color::WHITE))
                .named("front")
                .z_index(2),
            Node::from(Rect::new(0.0, 0.0, 5.0, 5.0).fill(Color::WHITE)).named("back"),
        ])
        .translate(3.0, 4.0);
        let inspect = node
            .inspect_with_context(
                Size::new(20.0, 20.0),
                PixelSize::new(40, 40),
                ValidationLimits::default(),
                &DrawContext::default(),
            )
            .unwrap();
        assert_eq!(inspect.len(), 3);
        assert!(inspect[1].path.contains("back"));
        assert!(inspect[2].path.contains("front"));
        assert_eq!(inspect[2].bounds, Rect::new(6.0, 8.0, 26.0, 28.0));
        assert!(
            node.inspect_with_context(
                Size::new(20.0, 20.0),
                PixelSize::new(40, 40),
                ValidationLimits {
                    max_nodes: 1,
                    ..ValidationLimits::default()
                },
                &DrawContext::default()
            )
            .is_err()
        );
    }
}
