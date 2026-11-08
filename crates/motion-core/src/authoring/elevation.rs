//! Height-driven layered soft shadows built from existing filter primitives.

use crate::{Color, Filter, FilterOperation, Node, NodeModifiers, ValidationError, Vector2};

/// One soft shadow silhouette: blur sigma, offset and opacity multiplier.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ShadowLayer {
    /// Gaussian standard deviation in local units.
    pub sigma: f64,
    /// Local displacement of the silhouette.
    pub offset: Vector2,
    /// Multiplier on the shadow color's alpha, in 0..=1.
    pub opacity: f64,
}

/// Height above the surface, expanded into contact, key and ambient shadows.
///
/// A small dark contact shadow anchors the object, a medium key shadow follows the
/// light direction and a wide faint ambient shadow suggests occlusion. Sigmas and
/// offsets grow linearly with `height`, while opacity falls off smoothly.
///
/// ```
/// use motion_core::{Color, Elevation, NodeModifiers, Rect};
/// let card = Rect::new(0.0, 0.0, 80.0, 50.0).fill(Color::WHITE);
/// let raised = card.elevated(Elevation::shadow(12.0)?);
/// # let _ = raised;
/// # Ok::<(), motion_core::ValidationError>(())
/// ```
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Elevation {
    height: f64,
    direction: Vector2,
    color: Color,
}

impl Elevation {
    /// Largest accepted height in local units.
    pub const MAX_HEIGHT: f64 = 1024.0;

    /// Creates a downward-falling black shadow stack for a finite height in `0..=1024`.
    pub fn shadow(height: f64) -> Result<Self, ValidationError> {
        if !height.is_finite() || !(0.0..=Self::MAX_HEIGHT).contains(&height) {
            return Err(ValidationError::invalid(
                "elevation",
                "height",
                "Require a finite height in 0..=1024",
            ));
        }
        Ok(Self {
            height,
            direction: Vector2::new(0.0, 1.0),
            color: Color::BLACK,
        })
    }

    /// Sets the direction the shadow falls; the vector is normalized.
    pub fn direction(mut self, direction: Vector2) -> Result<Self, ValidationError> {
        let length = libm::hypot(direction.x, direction.y);
        if !length.is_finite() || length <= 0.0 {
            return Err(ValidationError::invalid(
                "elevation",
                "direction",
                "Require a finite nonzero direction",
            ));
        }
        self.direction = Vector2::new(direction.x / length, direction.y / length);
        Ok(self)
    }

    /// Sets the encoded-sRGB shadow color; its alpha scales every layer.
    pub fn color(mut self, color: Color) -> Self {
        self.color = color;
        self
    }

    /// The elevation in local units.
    pub fn height(self) -> f64 {
        self.height
    }

    /// Contact, key and ambient layers; empty at zero height.
    pub fn layers(self) -> Vec<ShadowLayer> {
        let height = self.height;
        if height == 0.0 {
            return Vec::new();
        }
        // Opacity fades as the object lifts away, but never vanishes within the bounded range.
        let fade = 1.0 / (1.0 + height / 64.0);
        [
            (0.1 * height + 0.5, 0.06 * height, 0.55 * fade),
            (0.5 * height, 0.4 * height, 0.32 * fade),
            (1.0 * height + 1.0, 0.8 * height, 0.2 * fade),
        ]
        .map(|(sigma, distance, opacity)| ShadowLayer {
            sigma,
            offset: Vector2::new(self.direction.x * distance, self.direction.y * distance),
            opacity,
        })
        .into()
    }

    /// Draws the layers behind a single untouched source snapshot.
    pub fn apply(self, source: Node) -> Node {
        let layers = self.layers();
        if layers.is_empty() {
            return source;
        }
        shadow_stack(
            source,
            self.color,
            layers
                .into_iter()
                .map(|layer| (layer.sigma, layer.offset, layer.opacity)),
        )
    }
}

/// Colorizes the source alpha, blurs and offsets one clone per layer, then draws the source.
pub(super) fn shadow_stack(
    source: Node,
    color: Color,
    layers: impl IntoIterator<Item = (f64, Vector2, f64)>,
) -> Node {
    let linear = color.to_linear_premultiplied();
    let straight = if linear[3] > 0.0 {
        [
            linear[0] / linear[3],
            linear[1] / linear[3],
            linear[2] / linear[3],
        ]
    } else {
        [0.0; 3]
    };
    let mut nodes = Vec::new();
    for (sigma, offset, opacity) in layers {
        let mut matrix = [0.0; 20];
        matrix[4] = straight[0];
        matrix[9] = straight[1];
        matrix[14] = straight[2];
        matrix[18] = linear[3] * opacity;
        nodes.push(source.clone().filter(Filter::Custom(vec![
            FilterOperation::ColorMatrix(matrix),
            FilterOperation::Blur { sigma },
            FilterOperation::Offset {
                x: offset.x,
                y: offset.y,
            },
        ])));
    }
    nodes.push(source);
    Node::group(nodes)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::Rect;

    #[test]
    fn layers_grow_with_height_and_fade_in_opacity() {
        assert!(Elevation::shadow(0.0).unwrap().layers().is_empty());
        let low = Elevation::shadow(4.0).unwrap().layers();
        let high = Elevation::shadow(40.0).unwrap().layers();
        assert_eq!(low.len(), 3);
        for (low, high) in low.iter().zip(&high) {
            assert!(high.sigma > low.sigma);
            assert!(high.offset.y > low.offset.y);
            assert!(high.opacity < low.opacity);
        }
        // Contact is tight and dark, ambient is wide and faint.
        assert!(low[0].sigma < low[2].sigma && low[0].opacity > low[2].opacity);
    }

    #[test]
    fn direction_is_normalized_and_inputs_are_validated() {
        let layers = Elevation::shadow(10.0)
            .unwrap()
            .direction(Vector2::new(3.0, 4.0))
            .unwrap()
            .layers();
        let ratio = layers[1].offset.x / layers[1].offset.y;
        assert!((ratio - 0.75).abs() < 1e-12);
        for height in [-1.0, f64::NAN, f64::INFINITY, 2000.0] {
            assert!(Elevation::shadow(height).is_err());
        }
        let elevation = Elevation::shadow(1.0).unwrap();
        assert!(elevation.direction(Vector2::new(0.0, 0.0)).is_err());
        assert!(elevation.direction(Vector2::new(f64::NAN, 1.0)).is_err());
    }

    #[test]
    fn apply_builds_layer_clones_before_the_untouched_source() {
        let card: Node = Rect::new(0.0, 0.0, 8.0, 8.0).fill(Color::WHITE).into();
        let raised = card.clone().elevated(Elevation::shadow(8.0).unwrap());
        let children = |node: &Node| match &node.data.kind {
            super::super::NodeKind::Group(children) => Some(children.len()),
            _ => None,
        };
        assert_eq!(children(&raised), Some(4));
        let flat = card.elevated(Elevation::shadow(0.0).unwrap());
        assert_eq!(children(&flat), None);
    }
}
