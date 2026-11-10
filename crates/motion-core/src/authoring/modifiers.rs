//! Immutable placement and compositing modifiers.

use super::*;

/// Snapshot modifiers shared by all values convertible into a [`Node`].
///
/// Placement, compositing, clipping and filters preserve immutable snapshots.
pub trait NodeModifiers: Into<Node> + Sized {
    /// Layers three soft silhouettes behind a single untouched source snapshot.
    fn layered_shadow(self, color: crate::Color, radius: f64, offset: crate::Vector2) -> Node {
        super::elevation::shadow_stack(
            self.into(),
            color,
            [(1.0, 0.2), (0.5, 0.3), (0.2, 0.5)]
                .map(|(scale, opacity)| (radius * scale, offset, opacity)),
        )
    }

    /// Layers contact, key and ambient soft shadows that scale with an [`Elevation`](super::elevation::Elevation).
    fn elevated(self, elevation: super::elevation::Elevation) -> Node {
        elevation.apply(self.into())
    }

    /// Repeatable physical gate drift, keyed only by an explicit frame and seed.
    fn gate_weave(self, seed: u64, frame: u64, amount: f64) -> Node {
        let random = super::sampling::shutter_random;
        self.into()
            .translate(
                (random(seed ^ frame.wrapping_mul(3)) * 2.0 - 1.0) * amount,
                (random(seed ^ frame.wrapping_mul(3).wrapping_add(1)) * 2.0 - 1.0) * amount,
            )
            .rotate(
                (random(seed ^ frame.wrapping_mul(3).wrapping_add(2)) * 2.0 - 1.0) * amount * 0.001,
            )
    }

    /// Places content next to the reference during compile-time layout.
    fn next_to(self, reference: &Node, side: Side, gap: f64) -> Node {
        self.into().next_to(reference, side, gap)
    }

    /// Aligns an edge with a compile-time measured reference snapshot.
    fn align_to(self, reference: &Node, edge: Side) -> Node {
        self.into().align_to(reference, edge)
    }

    /// Places the selected pivot at a parent-space point, before translation.
    fn at(self, position: impl Into<Point>) -> Node {
        let mut node = self.into();
        Arc::make_mut(&mut node.data).placement.position = Some(position.into());
        node
    }

    /// Adds parent-space translation, after scale and rotation.
    fn translate(self, horizontal: f64, vertical: f64) -> Node {
        let mut node = self.into();
        let translation = &mut Arc::make_mut(&mut node.data).placement.translation;
        translation.x += horizontal;
        translation.y += vertical;
        node
    }

    /// Adds clockwise radians about the pivot, after scale and before translation.
    fn rotate(self, radians: f64) -> Node {
        let mut node = self.into();
        Arc::make_mut(&mut node.data).placement.rotation += radians;
        node
    }

    /// Multiplies both scale factors; negative factors reflect and zero collapses.
    fn scale(self, factor: f64) -> Node {
        self.scale_xy(factor, factor)
    }

    /// Multiplies per-axis scale factors about the pivot.
    fn scale_xy(self, horizontal: f64, vertical: f64) -> Node {
        let mut node = self.into();
        let scale = &mut Arc::make_mut(&mut node.data).placement.scale;
        scale.x *= horizontal;
        scale.y *= vertical;
        node
    }

    /// Replaces the local-space pivot; defaults to the layout-bounds center.
    fn pivot(self, pivot: Pivot) -> Node {
        let mut node = self.into();
        Arc::make_mut(&mut node.data).placement.pivot = pivot;
        node
    }

    /// Replaces subtree opacity; invalid values are reported during conversion.
    fn opacity(self, opacity: f64) -> Node {
        let mut node = self.into();
        Arc::make_mut(&mut node.data).opacity = opacity;
        node
    }

    /// Blends the completed subtree once against its immediate parent backdrop.
    ///
    /// Modes use the selected blend space and force isolation even at opacity one.
    /// This does not change layout bounds.
    ///
    /// ```
    /// use motion_core::prelude::*;
    /// use motion_core::{
    ///     DisplayList, RasterLimits, ResourcePool, ValidatedDisplayList, MAX_PATH_VERBS,
    ///     MAX_STROKE_VERBS, MAX_SVG_PATH_BYTES, sample_offsets,
    /// };
    /// let tile = || Node::from(Rect::new(0.0, 0.0, 1.0, 1.0).fill(Color::WHITE)).opacity(0.5);
    /// let scene = Node::group([tile(), tile().blend_mode(BlendMode::Multiply)]);
    /// let prepared = scene.compile(
    ///     Size::new(1.0, 1.0),
    ///     PixelSize::new(1, 1),
    ///     ValidationLimits::default(),
    /// )?;
    /// # Ok::<(), motion_core::ValidationError>(())
    /// ```
    fn blend_mode(self, mode: BlendMode) -> Node {
        let mut node = self.into();
        Arc::make_mut(&mut node.data).blend_mode = mode;
        node
    }

    /// Appends a source-only filter, preserving declaration order.
    ///
    /// Filters process the completed subtree before this node's content mask,
    /// clips, opacity and blend. Child effects complete before parent effects.
    /// Layout bounds stay unchanged. Distances transform from local design units;
    /// CPU preparation retains offscreen inputs and derives capture bounds.
    ///
    /// ```
    /// use motion_core::prelude::*;
    /// use motion_core::{
    ///     DisplayList, RasterLimits, ResourcePool, ValidatedDisplayList, MAX_PATH_VERBS,
    ///     MAX_STROKE_VERBS, MAX_SVG_PATH_BYTES, sample_offsets,
    /// };
    /// let node = Node::from(Rect::new(0.0, 0.0, 2.0, 2.0).fill(Color::WHITE))
    ///     .filter(Filter::Blur { sigma: 0.5 });
    /// let output = node.compile(
    ///     Size::new(4.0, 4.0),
    ///     PixelSize::new(4, 4),
    ///     ValidationLimits::default(),
    /// )?;
    /// # Ok::<(), motion_core::ValidationError>(())
    /// ```
    fn filter(self, filter: crate::Filter) -> Node {
        let mut node = self.into();
        Arc::make_mut(&mut node.data).filters.push(filter);
        node
    }

    /// Replaces the subtree's local path-coverage clip without changing layout.
    ///
    /// Coverage multiplies the completed isolated subtree once, before group
    /// opacity and blending. Nested masks multiply; this is not geometric path
    /// intersection. Use a checked Path2D or a PathClip with an explicit rule.
    ///
    /// ```
    /// use motion_core::prelude::*;
    /// use motion_core::{
    ///     DisplayList, RasterLimits, ResourcePool, ValidatedDisplayList, MAX_PATH_VERBS,
    ///     MAX_STROKE_VERBS, MAX_SVG_PATH_BYTES, sample_offsets,
    /// };
    /// let tile = || Node::from(Rect::new(0.0, 0.0, 2.0, 1.0).fill(Color::WHITE));
    /// let path = Path2D::from_rect(Rect::new(0.5, 0.0, 2.0, 1.0))?;
    /// let node = Node::group([tile(), tile()]).clip(path);
    /// let prepared = node.compile(
    ///     Size::new(2.0, 1.0),
    ///     PixelSize::new(2, 1),
    ///     ValidationLimits::default(),
    /// )?;
    /// # Ok::<(), motion_core::ValidationError>(())
    /// ```
    fn clip(self, shape: impl crate::IntoShapeClip) -> Node {
        self.clip_with(shape, crate::ClipMode::Coverage)
    }

    /// Clips with explicit coverage multiplication or geometric intersection.
    fn clip_with(self, shape: impl crate::IntoShapeClip, mode: crate::ClipMode) -> Node {
        let shape = match shape.into_shape_clip() {
            Ok(shape) => shape,
            Err(error) => return super::transition::diagnostic(error),
        };
        let mut node = self.into();
        let data = Arc::make_mut(&mut node.data);
        match mode {
            crate::ClipMode::Coverage => {
                let path = match shape.geometry {
                    crate::drawing::ClipGeometry::Path(path) => path,
                    crate::drawing::ClipGeometry::Rectangle(bounds) => {
                        match crate::Path2D::from_rect(bounds) {
                            Ok(path) => Arc::new(path),
                            Err(error) => return super::transition::diagnostic(error),
                        }
                    }
                };
                data.clip = Some(crate::PathClip {
                    path,
                    transform: shape.transform,
                    rule: shape.rule,
                });
            }
            crate::ClipMode::Geometric => match shape.geometry {
                crate::drawing::ClipGeometry::Rectangle(bounds) => {
                    data.shape_clip = None;
                    data.rect_clip = Some(crate::RectClip {
                        bounds,
                        transform: shape.transform,
                    });
                }
                geometry => {
                    data.rect_clip = None;
                    data.shape_clip = Some(crate::ShapeClip {
                        geometry,
                        transform: shape.transform,
                        rule: shape.rule,
                    });
                }
            },
        }
        node
    }

    /// Replaces the local geometric rectangle clip without changing layout.
    ///
    /// Nested rectangles intersect in device space before coverage quantization,
    /// including rotated/reflected rectangles. Repeating an edge does not fade
    /// it again. The completed subtree is clipped once before opacity/blending.
    /// Path clips and content masks remain independent multiplicative coverage.
    /// RectClip allows a separate local transform. Invalid hidden clips fail
    /// validation; zero area is empty.
    ///
    /// ```
    /// use motion_core::prelude::*;
    /// use motion_core::{
    ///     DisplayList, RasterLimits, ResourcePool, ValidatedDisplayList, MAX_PATH_VERBS,
    ///     MAX_STROKE_VERBS, MAX_SVG_PATH_BYTES, sample_offsets,
    /// };
    /// let rect = Rect::new(0.5, 0.0, 2.0, 1.0);
    /// let inner = Node::from(Rect::new(0.0, 0.0, 2.0, 1.0).fill(Color::WHITE))
    ///     .clip_with(rect, motion_core::ClipMode::Geometric);
    /// let frame = Node::group([inner])
    ///     .clip_with(rect, motion_core::ClipMode::Geometric)
    ///     .compile(
    ///         Size::new(2.0, 1.0),
    ///         PixelSize::new(2, 1),
    ///         ValidationLimits::default(),
    ///     )?;
    /// # Ok::<(), motion_core::ValidationError>(())
    /// ```
    ///
    /// Replaces the geometric Shape snapshot without changing layout.
    ///
    /// Shapes and rectangles intersect before coverage quantization. NonZero
    /// is the default; ShapeClip exposes an independent rule and local transform.
    /// Path coverage masks and content masks still multiply independently.
    /// Curves use the preparation tolerance, not a pixel-area error guarantee.
    ///
    /// ```
    /// use motion_core::prelude::*;
    /// use motion_core::{
    ///     DisplayList, RasterLimits, ResourcePool, ValidatedDisplayList, MAX_PATH_VERBS,
    ///     MAX_STROKE_VERBS, MAX_SVG_PATH_BYTES, sample_offsets,
    /// };
    /// let triangle = Path2D::builder()
    ///     .move_to(Point::ORIGIN)
    ///     .line_to(Point::new(1.0, 0.0))
    ///     .line_to(Point::new(0.0, 1.0))
    ///     .build()?;
    /// let inner = Node::from(Rect::new(0.0, 0.0, 1.0, 1.0).fill(Color::WHITE))
    ///     .clip_with(triangle.clone(), motion_core::ClipMode::Geometric);
    /// let frame = Node::group([inner])
    ///     .clip_with(triangle, motion_core::ClipMode::Geometric)
    ///     .compile(
    ///         Size::new(1.0, 1.0),
    ///         PixelSize::new(1, 1),
    ///         ValidationLimits::default(),
    ///     )?;
    /// # Ok::<(), motion_core::ValidationError>(())
    /// ```
    ///
    /// Replaces the subtree's alpha or linear-luminance content mask.
    ///
    /// Mask artwork uses the source's local coordinate system and a transparent
    /// isolated backdrop. It does not contribute layout bounds. The completed
    /// mask scales content once, before path clipping, opacity and blending.
    /// Inverted modes complement clamped coverage, not color or alpha first.
    /// Empty or transparent mask regions then preserve existing source pixels;
    /// they do not create source artwork or enlarge its capture bounds.
    ///
    /// ```
    /// use motion_core::prelude::*;
    /// use motion_core::{
    ///     DisplayList, RasterLimits, ResourcePool, ValidatedDisplayList, MAX_PATH_VERBS,
    ///     MAX_STROKE_VERBS, MAX_SVG_PATH_BYTES, sample_offsets,
    /// };
    /// let tile = |color| Node::from(Rect::new(0.0, 0.0, 1.0, 1.0).fill(color));
    /// let node = tile(Color::WHITE).mask(
    ///     tile(Color::from_hex(0xff0000).with_alpha(0.5)),
    ///     MaskMode::Luminance,
    /// );
    /// let frame = node.compile(
    ///     Size::new(1.0, 1.0),
    ///     PixelSize::new(1, 1),
    ///     ValidationLimits::default(),
    /// )?;
    /// # Ok::<(), motion_core::ValidationError>(())
    /// ```
    ///
    /// ```
    /// use motion_core::prelude::*;
    /// use motion_core::{
    ///     DisplayList, RasterLimits, ResourcePool, ValidatedDisplayList, MAX_PATH_VERBS,
    ///     MAX_STROKE_VERBS, MAX_SVG_PATH_BYTES, sample_offsets,
    /// };
    /// let source = Node::from(Rect::new(1.0, 0.0, 3.0, 1.0).fill(Color::WHITE));
    /// let mask = Node::from(Rect::new(1.0, 0.0, 2.0, 1.0).fill(Color::WHITE));
    /// let frame = source.mask(mask, MaskMode::InvertedAlpha).compile(
    ///     Size::new(4.0, 1.0),
    ///     PixelSize::new(4, 1),
    ///     ValidationLimits::default(),
    /// )?;
    /// # Ok::<(), motion_core::ValidationError>(())
    /// ```
    fn mask(self, mask: impl Into<Node>, mode: crate::drawing::MaskMode) -> Node {
        let mut node = self.into();
        Arc::make_mut(&mut node.data).mask = Some(Box::new(NodeMask {
            node: mask.into(),
            mode,
        }));
        node
    }

    /// Replaces sibling-local stacking order; ties retain insertion order.
    fn z_index(self, index: i32) -> Node {
        let mut node = self.into();
        Arc::make_mut(&mut node.data).z_index = index;
        node
    }

    /// Removes the snapshot entirely when false, rather than evaluating it hidden.
    fn visible(self, visible: bool) -> Node {
        if visible { self.into() } else { Node::empty() }
    }

    /// Attaches a diagnostic name without changing artwork or draw order.
    fn named(self, name: &str) -> Node {
        let mut node = self.into();
        Arc::make_mut(&mut node.data).name = Some(Arc::from(name));
        node
    }
}

impl<Value: Into<Node>> NodeModifiers for Value {}
