//! Immutable scene nodes and bounded layout traversal.

use super::*;

/// A cheaply cloned snapshot of authored grouping and compositing choices.
///
/// [`Drawable`] values convert directly into leaves with content bounds.
/// Grouping and modifiers retain geometry and explicit drawing policy.
///
/// ```
/// use motion_core::prelude::*;
/// use motion_core::{
///     DisplayList, RasterLimits, ResourcePool, ValidatedDisplayList, MAX_PATH_VERBS,
///     MAX_STROKE_VERBS, MAX_SVG_PATH_BYTES, sample_offsets,
/// };
/// let artwork = Rect::new(0.0, 0.0, 32.0, 24.0).fill(Color::rgb(20, 150, 125));
/// let node = Node::group([Node::from(artwork).z_index(2)])
///     .opacity(0.5)
///     .named("overlay");
/// node.compile(
///     Size::new(32.0, 24.0),
///     PixelSize::new(32, 24),
///     ValidationLimits::default(),
/// )?;
/// # Ok::<(), motion_core::ValidationError>(())
/// ```
#[derive(Clone, Debug)]
pub struct Node {
    pub(super) data: Arc<NodeData>,
}

#[derive(Clone, Debug)]
pub(super) struct NodeData {
    pub(super) time: Option<Time>,
    pub(super) kind: NodeKind,
    pub(super) placement: Placement,
    pub(super) text_metrics: Option<(f64, Rect)>,
    pub(super) filters: Vec<crate::Filter>,
    pub(super) opacity: f64,
    pub(super) blend_mode: BlendMode,
    pub(super) clip: Option<crate::PathClip>,
    pub(super) rect_clip: Option<crate::RectClip>,
    pub(super) shape_clip: Option<crate::ShapeClip>,
    pub(super) mask: Option<Box<NodeMask>>,
    pub(super) z_index: i32,
    pub(super) name: Option<Arc<str>>,

    pub(super) shutter_sampled: bool,
}

#[derive(Clone, Debug)]
pub(super) struct NodeMask {
    pub(super) node: Node,
    pub(super) mode: crate::drawing::MaskMode,
}

#[derive(Clone)]
pub(super) enum NodeKind {
    Empty,
    Leaf(Arc<dyn Drawable>),
    Group(Vec<Node>),
    Accumulate(Vec<Node>),
    Layout(Layout, Vec<Node>),
    Relative(super::spatial::Relative),
}

impl std::fmt::Debug for NodeKind {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Empty => formatter.write_str("Empty"),
            Self::Leaf(_) => formatter.write_str("Leaf(..)"),
            Self::Relative(relative) => formatter.debug_tuple("Relative").field(relative).finish(),
            Self::Group(children) => formatter.debug_tuple("Group").field(children).finish(),
            Self::Accumulate(children) => {
                formatter.debug_tuple("Accumulate").field(children).finish()
            }
            Self::Layout(layout, children) => formatter
                .debug_tuple("Layout")
                .field(layout)
                .field(children)
                .finish(),
        }
    }
}

impl Default for Node {
    fn default() -> Self {
        Self::empty()
    }
}

/// Iterable or heterogeneous tuple children accepted by `Node::group`.
pub trait IntoNodeGroup<Kind> {
    /// Freezes child snapshots in source order.
    fn into_nodes(self) -> Vec<Node>;
}

/// Distinguishes iterable children from tuple children during type inference.
pub struct IteratorGroup<Child>(std::marker::PhantomData<Child>);

/// Distinguishes heterogeneous tuple children from iterable children.
pub struct TupleGroup;

impl<Child: Into<Node>, Children: IntoIterator<Item = Child>> IntoNodeGroup<IteratorGroup<Child>>
    for Children
{
    fn into_nodes(self) -> Vec<Node> {
        self.into_iter().map(Into::into).collect()
    }
}

impl IntoNodeGroup<TupleGroup> for () {
    fn into_nodes(self) -> Vec<Node> {
        Vec::new()
    }
}

impl From<()> for Node {
    fn from(_: ()) -> Self {
        Self::empty()
    }
}

macro_rules! tuple_nodes {
    (@impl $($kind:ident:$value:ident,)+) => {
        impl<$($kind: Into<Node>),+> IntoNodeGroup<TupleGroup> for ($($kind,)+) {
            fn into_nodes(self) -> Vec<Node> {
                let ($($value,)+) = self;
                vec![$($value.into()),+]
            }
        }
        impl<$($kind: Into<Node>),+> From<($($kind,)+)> for Node {
            fn from(values: ($($kind,)+)) -> Self {
                Self::new(NodeKind::Group(IntoNodeGroup::<TupleGroup>::into_nodes(values)))
            }
        }
    };
    (@grow [$($kind:ident:$value:ident,)+] $next:ident:$next_value:ident $(,$tail:ident:$tail_value:ident)*) => {
        tuple_nodes!(@impl $($kind:$value,)+ $next:$next_value,);
        tuple_nodes!(@grow [$($kind:$value,)+ $next:$next_value,] $($tail:$tail_value),*);
    };
    (@grow [$($kind:ident:$value:ident,)+]) => {};
    ($first:ident:$first_value:ident $(,$tail:ident:$tail_value:ident)*) => {
        tuple_nodes!(@impl $first:$first_value,);
        tuple_nodes!(@grow [$first:$first_value,] $($tail:$tail_value),*);
    };
}

tuple_nodes!(First:first, Second:second, Third:third, Fourth:fourth, Fifth:fifth, Sixth:sixth, Seventh:seventh, Eighth:eighth, Ninth:ninth, Tenth:tenth, Eleventh:eleventh, Twelfth:twelfth, Thirteenth:thirteenth, Fourteenth:fourteenth, Fifteenth:fifteenth, Sixteenth:sixteenth);

impl Node {
    pub(crate) fn shutter_sampled(mut self) -> Self {
        Arc::make_mut(&mut self.data).shutter_sampled = true;
        self
    }

    pub(crate) fn contains_shutter(&self, depth: usize) -> bool {
        if depth > 256 || self.data.shutter_sampled {
            return true;
        }
        if self
            .data
            .mask
            .as_ref()
            .is_some_and(|mask| mask.node.contains_shutter(depth + 1))
        {
            return true;
        }
        match &self.data.kind {
            NodeKind::Group(children)
            | NodeKind::Accumulate(children)
            | NodeKind::Layout(_, children) => children
                .iter()
                .any(|child| child.contains_shutter(depth + 1)),
            NodeKind::Relative(relative) => {
                relative.node.contains_shutter(depth + 1)
                    || relative.reference.contains_shutter(depth + 1)
            }
            _ => false,
        }
    }

    /// Associates scene-local time with deferred content in this subtree.
    pub fn with_scene_time(mut self, time: Time) -> Self {
        Arc::make_mut(&mut self.data).time = Some(time);
        self
    }

    pub(super) fn new(kind: NodeKind) -> Self {
        Self {
            data: Arc::new(NodeData {
                time: None,
                kind,
                placement: Placement::default(),
                text_metrics: None,
                filters: Vec::new(),
                opacity: 1.0,
                blend_mode: BlendMode::SourceOver,
                clip: None,
                rect_clip: None,
                shape_clip: None,
                mask: None,
                z_index: 0,
                name: None,
                shutter_sampled: false,
            }),
        }
    }

    /// Creates a node with no artwork.
    pub fn empty() -> Self {
        Self::new(NodeKind::Empty)
    }

    /// Positions a snapshot relative to compile-time measured reference bounds.
    pub fn next_to(self, reference: &Node, side: Side, gap: f64) -> Self {
        spatial::relative(self, reference.clone(), side, Some(gap))
    }

    /// Aligns an edge with a compile-time measured reference snapshot.
    pub fn align_to(self, reference: &Node, edge: Side) -> Self {
        spatial::relative(self, reference.clone(), edge, None)
    }

    /// Averages 1..=64 independently rendered samples in linear premultiplied RGBA.
    /// Empty samples retain their weight; sibling z order cannot reorder samples.
    pub fn accumulate(samples: impl IntoIterator<Item = Node>) -> Result<Self, ValidationError> {
        let samples = samples.into_iter().take(65).collect::<Vec<_>>();
        let count = samples.len();
        Self::weighted_sum(samples.into_iter().map(|node| (node, 1.0 / count as f64)))
    }

    /// Composites 1..=64 isolated samples with explicit linear-light weights.
    /// Each weight must be finite in `0..=1`; weights are not normalized.
    pub fn weighted_sum(
        samples: impl IntoIterator<Item = (Node, f64)>,
    ) -> Result<Self, ValidationError> {
        let samples = samples.into_iter().take(65).collect::<Vec<_>>();
        if samples.is_empty()
            || samples.len() > 64
            || samples
                .iter()
                .any(|(_, weight)| !weight.is_finite() || !(0.0..=1.0).contains(weight))
        {
            return Err(ValidationError::invalid(
                "accumulate",
                "samples",
                "Require 1..=64 samples and finite weights in 0..=1",
            ));
        }
        Ok(Self::new(NodeKind::Accumulate(
            samples
                .into_iter()
                .map(|(node, weight)| {
                    Node::group([node])
                        .opacity(weight)
                        .blend_mode(BlendMode::LinearPlus)
                })
                .collect(),
        )))
    }

    /// Attaches measured paragraph metrics for horizontal baseline layout.
    /// Content implementations must provide conservative local bounds and a real baseline.
    pub fn with_text_metrics(mut self, baseline: f64, bounds: Rect) -> Self {
        Arc::make_mut(&mut self.data).text_metrics = Some((baseline, bounds));
        self
    }

    /// Applies a full local affine matrix in a new group, before subsequent
    /// placement modifiers.
    pub fn transformed(self, transform: crate::Transform) -> Self {
        let mut group = Self::group([self]);
        Arc::make_mut(&mut group.data).placement.affine = transform;
        group
    }

    /// Groups snapshots; sibling z indices are stably sorted during conversion.
    pub fn group<Kind>(children: impl IntoNodeGroup<Kind>) -> Self {
        Self::new(NodeKind::Group(children.into_nodes()))
    }

    /// Appends a drawable snapshot; nongroup nodes retain their original subtree.
    pub fn push(&mut self, child: impl Into<Node>) {
        let child = child.into();
        if let NodeKind::Group(children) = &mut Arc::make_mut(&mut self.data).kind {
            children.push(child);
        } else {
            *self = Self::group([self.clone(), child]);
        }
    }

    pub(super) fn layout<Child: Into<Node>>(
        layout: Layout,
        children: impl IntoIterator<Item = Child>,
    ) -> Self {
        Self::new(NodeKind::Layout(
            layout,
            children.into_iter().map(Into::into).collect(),
        ))
    }

    /// Measures content or declared layout geometry in parent coordinates.
    ///
    /// Uses f64 throughout, includes zero-area declared bounds and ignores empty
    /// nodes. Structure and parameters validate without pixel narrowing.
    /// Opacity does not change layout bounds. leaves call
    /// Drawable::bounds, not draw. Uses default drawing inputs at device scale one.
    pub fn measure(&self, limits: ValidationLimits) -> Result<Rect, ValidationError> {
        self.measure_with(&DrawContext::default(), limits)
    }

    /// Validates structure and measured geometry without recording draw commands.
    /// Fonts, when required by content, must be supplied through the drawing context.
    pub fn preflight_with(
        &self,
        context: &DrawContext,
        limits: ValidationLimits,
    ) -> Result<crate::SceneStats, ValidationError> {
        let context = &context.clone().with_validation_limits(limits);
        let mut state = ValidationState::new(limits)?;
        self.measure_bounds("node", 1, &mut state, context)?;
        Ok(state.stats)
    }

    /// Measures with explicit, time-independent drawing inputs.
    ///
    /// Device scale is metadata for content; this adapter does not apply the
    /// future scene-to-device root transform or infer resource environments.
    pub fn measure_with(
        &self,
        context: &DrawContext,
        limits: ValidationLimits,
    ) -> Result<Rect, ValidationError> {
        let context = &context.clone().with_validation_limits(limits);
        let mut state = ValidationState::new(limits)?;
        self.measure_bounds("node", 1, &mut state, context)?
            .rect("node")
    }

    pub(super) fn measure_bounds(
        &self,
        path: &str,
        depth: usize,
        state: &mut ValidationState,
        context: &DrawContext,
    ) -> Result<Bounds, ValidationError> {
        let context = &context
            .clone()
            .with_time(self.data.time.unwrap_or(context.time()));
        let path = self
            .data
            .name
            .as_ref()
            .map_or_else(|| path.to_owned(), |name| format!("{path}(name={name})"));
        state.enter(&path, depth)?;
        self.validate_clip(&path, depth, state)?;
        if !self.data.opacity.is_finite() || !(0.0..=1.0).contains(&self.data.opacity) {
            return Err(ValidationError::invalid(
                &path,
                "opacity",
                "Require finite opacity in 0..=1",
            ));
        }
        if let Some(mask) = &self.data.mask {
            mask.node
                .measure_bounds(&format!("{path}.mask"), depth + 1, state, context)?;
        }
        let bounds = match &self.data.kind {
            NodeKind::Empty => Bounds::Empty,
            NodeKind::Relative(relative) => {
                let subject = relative.node.measure_bounds(
                    &format!("{path}.subject"),
                    depth + 1,
                    state,
                    context,
                )?;
                let reference = relative.reference.measure_bounds(
                    &format!("{path}.reference"),
                    depth + 1,
                    state,
                    context,
                )?;
                let translation =
                    relative.translation(subject.rect(&path)?, reference.rect(&path)?)?;
                subject.transformed(translation, &path)?
            }
            NodeKind::Leaf(drawable) => {
                if let Some(resolved) = drawable.resolve_node(context)? {
                    resolved.measure_bounds(&path, depth + 1, state, context)?
                } else {
                    let bounds = drawable.try_bounds(context)?;
                    crate::drawing::validate_rect(bounds, &path, "bounds")?;
                    Bounds::Known(bounds)
                }
            }
            NodeKind::Group(children)
            | NodeKind::Layout(_, children)
            | NodeKind::Accumulate(children) => {
                let mut geometry = Vec::new();
                for (index, child) in children.iter().enumerate() {
                    geometry.push((
                        index,
                        child.measure_bounds(
                            &format!("{path}.children[{index}]"),
                            depth + 1,
                            state,
                            context,
                        )?,
                    ));
                }
                if let NodeKind::Layout(layout, _) = self.data.kind {
                    let baselines = layout.baselines(children, context, &path)?;
                    let placements = layout.placements(&geometry, &baselines, &path)?;
                    for ((_, bounds), placement) in geometry.iter_mut().zip(placements) {
                        if let Some((_, placed)) = placement {
                            *bounds = placed;
                        }
                    }
                }
                geometry
                    .into_iter()
                    .fold(Bounds::Empty, |bounds, (_, child)| bounds.union(child))
            }
        };
        let placement = self.data.placement.resolve(bounds, &path)?;
        bounds.transformed(placement.transform, &path)
    }

    pub(super) fn validate_clip(
        &self,
        path: &str,
        depth: usize,
        state: &mut ValidationState,
    ) -> Result<(), ValidationError> {
        for (index, filter) in self.data.filters.iter().enumerate() {
            let path = format!("{path}.filters[{index}]");
            filter.validate(&path)?;
            for operation in 0..filter.structural_cost() {
                state.enter_with(|| format!("{path}[{operation}]"), depth + 1)?;
            }
        }
        if let Some(clip) = self.data.rect_clip {
            let path = format!("{path}.rect_clip");
            crate::display_list::mapped_bounds(clip.bounds, clip.transform, &path)?;
            state.enter(&path, depth + 1)?;
        }
        if let Some(clip) = &self.data.shape_clip {
            let path = format!("{path}.shape_clip");
            let verbs = clip.validate(&path)?;
            state.enter(&path, depth + 1)?;
            for index in 0..verbs {
                state.enter_with(|| format!("{path}.verbs[{index}]"), depth + 1)?;
            }
        }
        if let Some(clip) = &self.data.clip {
            let path = format!("{path}.clip");
            crate::display_list::validate_path(&clip.path, clip.transform, &path)?;
            for index in 0..clip.path.verbs().len() {
                state.enter_with(|| format!("{path}.verbs[{index}]"), depth + 1)?;
            }
        }
        Ok(())
    }

    pub(super) fn baseline(
        &self,
        context: &DrawContext,
        path: &str,
    ) -> Result<Option<f64>, ValidationError> {
        let (baseline, local_bounds) = if let Some(metrics) = self.data.text_metrics {
            metrics
        } else {
            let NodeKind::Leaf(drawable) = &self.data.kind else {
                return Ok(None);
            };
            let Some(baseline) = drawable.baseline(context) else {
                return Ok(None);
            };
            (baseline, drawable.try_bounds(context)?)
        };
        if !baseline.is_finite() {
            return Err(ValidationError::invalid(
                path,
                "baseline",
                "Text baseline must be finite",
            ));
        }
        if self.data.placement.rotation != 0.0 || self.data.placement.scale.y <= 0.0 {
            return Err(ValidationError::unsupported(
                path,
                "baseline",
                "Baseline rows require horizontal, upright text",
            ));
        }
        let bounds = Bounds::declared(local_bounds, path)?;
        let placement = self.data.placement.resolve(bounds, path)?;
        let baseline = placement.transform.apply(Point::new(0.0, baseline)).y;
        if !baseline.is_finite() {
            return Err(ValidationError::invalid(
                path,
                "baseline",
                "Transformed baseline must be finite",
            ));
        }
        Ok(Some(baseline))
    }
}

impl<Content: Drawable + 'static> From<Content> for Node {
    fn from(content: Content) -> Self {
        Self::new(NodeKind::Leaf(Arc::new(content)))
    }
}

impl From<Arc<dyn Drawable>> for Node {
    fn from(content: Arc<dyn Drawable>) -> Self {
        Self::new(NodeKind::Leaf(content))
    }
}

impl<Value: Into<Node>> From<Option<Value>> for Node {
    fn from(value: Option<Value>) -> Self {
        value.map_or_else(Self::empty, Into::into)
    }
}

impl FromIterator<Node> for Node {
    fn from_iter<Iter: IntoIterator<Item = Node>>(iter: Iter) -> Self {
        Self::group(iter)
    }
}
