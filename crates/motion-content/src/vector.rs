//! Shared immutable semantic vectors for text, SVG, formulas and vector effects.
//!
//! ```
//! use motion_content::prelude::*;
//! use motion_core::prelude::*;
//! use motion_core::{
//!     DisplayList, RasterLimits, ResourceStore, ValidatedDisplayList, MAX_PATH_VERBS,
//!     MAX_STROKE_VERBS, MAX_SVG_PATH_BYTES, sample_offsets,
//! };
//! use motion_render::{Composition, RenderPrepare, RenderOptions, BackendKind, render_still};
//! let tree = VectorTree::new(
//!     Rect::new(0.0, 0.0, 16.0, 16.0),
//!     VectorGroup {
//!         children: vec![VectorItem::Path(VectorPath::filled(
//!             Path2D::from_rect(Rect::new(2.0, 2.0, 14.0, 14.0))?,
//!             Color::WHITE,
//!         ))],
//!         ..VectorGroup::default()
//!     },
//! )?;
//! assert_eq!(tree.surface()?.area(), 144.0);
//! assert_eq!(tree.sample_points(32, 42)?, tree.sample_points(32, 42)?);
//! let ink = tree.write(0.5, WriteOptions::default())?;
//! let morph = Morph::new(&tree, &tree)?;
//! let frame = morph
//!     .sample(0.5)?
//!     .compile(
//!         Size::new(16.0, 16.0),
//!         PixelSize::new(16, 16),
//!         ValidationLimits::default(),
//!     )?
//!     .prepare(ValidationLimits::default(), RasterLimits::default())?
//!     .render_linear(RasterLimits::default())?;
//! assert_eq!(frame.linear_pixel(8, 8), Some([1.0; 4]));
//! # Ok::<(), motion_core::Error>(())
//! ```

use motion_core::{
    DrawContext, Drawable, FillRule, Node, NodeModifiers, Paint, Path2D, Rect, StrokeStyle,
    Transform, ValidationError, ValidationLimits,
};
use std::{collections::BTreeMap, ops::Range, sync::Arc};

mod deferred;
mod effects;
#[cfg(feature = "ink")]
pub mod ink;
mod morph;
mod surface;
#[cfg(test)]
mod tests;

pub use deferred::{CompileCache, VectorSelection};
pub use effects::WriteOptions;
pub use morph::Morph;
pub use surface::{SurfaceTriangle, VectorSurface};

fn invalid(field: &str, message: &str) -> ValidationError {
    ValidationError::invalid("vector_tree", field, message)
}

fn limited(message: &str) -> ValidationError {
    ValidationError::limit("vector_tree", "structure", message)
}

/// A painted path in its containing group's local coordinates.
#[derive(Clone, Debug)]
pub struct VectorPath {
    /// Optional unique selection ID.
    pub id: Option<Arc<str>>,
    /// Original curves, shared without flattening.
    pub path: Arc<Path2D>,
    /// Fill paint and contour interpretation; None is genuinely unfilled.
    pub fill: Option<(Paint, FillRule)>,
    /// Local stroke style/paint, drawn after fill.
    pub stroke: Option<(StrokeStyle, Paint)>,
    /// Optional original UTF-8 source interval for reading-order operations.
    pub source: Option<Range<usize>>,
}

impl VectorPath {
    /// Creates a nonzero solid/spatial filled path without an ID or stroke.
    pub fn filled(path: Path2D, paint: impl Into<Paint>) -> Self {
        Self {
            id: None,
            path: Arc::new(path),
            fill: Some((paint.into(), FillRule::NonZero)),
            stroke: None,
            source: None,
        }
    }
}

impl Drawable for VectorPath {
    fn bounds(&self, _: &DrawContext) -> Result<Rect, ValidationError> {
        let mut bounds = self.path.bounding_box();
        if let Some((style, _)) = &self.stroke {
            bounds = bounds.union(
                style
                    .outline(&self.path, "vector_tree.stroke")?
                    .bounding_box(),
            );
        }
        Ok(bounds)
    }

    fn draw(
        &self,
        _: &DrawContext,
        canvas: &mut motion_core::Canvas<'_>,
    ) -> Result<(), ValidationError> {
        if let Some((paint, rule)) = &self.fill {
            canvas.fill_path(self.path.clone(), Transform::IDENTITY, paint.clone(), *rule)?;
        }
        if let Some((style, paint)) = &self.stroke {
            canvas.stroke(self.path.as_ref(), style, paint.clone())?;
        }
        Ok(())
    }
}

/// A bitmap leaf; its surrounding group supplies any affine placement.
#[derive(Clone, Debug)]
pub struct VectorImage {
    /// Optional unique selection ID.
    pub id: Option<Arc<str>>,
    /// Immutable linear-premultiplied mip data.
    pub image: Arc<crate::ImageAsset>,
    /// Local destination rectangle; pixels stretch to this rectangle.
    pub rect: Rect,
    /// Source-pixel-to-local mapping, independent of the filled rectangle.
    pub pixel_transform: Transform,
    /// Shared reconstruction rule.
    pub sampling: crate::Sampling,
    /// Sampling outside source coordinates.
    pub extend: motion_core::GradientExtend,
    /// Whether source-pixel bounds discard samples before reconstruction.
    pub decal: bool,
    /// Optional source provenance.
    pub source: Option<Range<usize>>,
}

impl VectorImage {
    /// Creates a image stretched to a local rectangle, with trilinear/Pad
    /// sampling and a source decal. Fonts/importers may retain a distinct mapping.
    pub fn new(image: Arc<crate::ImageAsset>, rect: Rect) -> Self {
        let pixel_transform = Transform::scale_non_uniform(
            rect.width() / f64::from(image.width()),
            rect.height() / f64::from(image.height()),
        )
        .then(Transform::translate(rect.left, rect.top));
        Self {
            id: None,
            image,
            rect,
            pixel_transform,
            sampling: crate::Sampling::Trilinear,
            extend: motion_core::GradientExtend::Pad,
            decal: true,
            source: None,
        }
    }

    pub(crate) fn pattern(&self) -> motion_core::ImagePattern {
        let mut pattern = motion_core::ImagePattern::from_asset(self.image.clone())
            .transform(self.pixel_transform)
            .sampling(self.sampling)
            .extend(self.extend);
        pattern.decal = self.decal;
        pattern
    }
}

impl Drawable for VectorImage {
    fn bounds(&self, _: &DrawContext) -> Result<Rect, ValidationError> {
        Ok(self.rect)
    }

    fn draw(
        &self,
        _: &DrawContext,
        canvas: &mut motion_core::Canvas<'_>,
    ) -> Result<(), ValidationError> {
        if self.rect.width() == 0.0 || self.rect.height() == 0.0 {
            return Ok(());
        }
        canvas.fill_path(
            Arc::new(Path2D::from_rect(self.rect)?),
            Transform::IDENTITY,
            self.pattern(),
            FillRule::NonZero,
        )
    }
}

/// A semantic leaf or group, retained in paint order.
#[derive(Clone, Debug)]
pub enum VectorItem {
    /// Nested isolation/coordinate system.
    Group(VectorGroup),
    /// Painted curves.
    Path(VectorPath),
    /// immutable image.
    Image(VectorImage),
}

/// A shared coordinate/isolation group, including source provenance.
#[derive(Clone, Debug)]
pub struct VectorMask {
    /// Immutable local mask artwork; its IDs are scoped to this resource.
    pub tree: Arc<VectorTree>,
    /// Alpha/luminance extraction policy.
    pub mode: motion_core::MaskMode,
    /// Local mask region, outside which coverage is zero.
    pub rect: Rect,
}

/// A shared coordinate/isolation group, including source provenance.
#[derive(Clone, Debug)]
pub struct VectorGroup {
    /// Optional unique selection ID.
    pub id: Option<Arc<str>>,
    /// Local-to-parent full f64 affine mapping.
    pub transform: Transform,
    /// Whole-group alpha, not per-child alpha.
    pub opacity: f64,
    /// Optional local path coverage, applied once to the completed group.
    pub clip: Option<motion_core::PathClip>,
    /// Content coverage applied once after source filters and before clipping.
    pub mask: Option<VectorMask>,
    /// Whole-group blending against its immediate backdrop.
    pub blend_mode: motion_core::BlendMode,
    /// Isolates descendant blending even with unit opacity and SourceOver.
    pub isolate: bool,
    /// Ordered source-only filters.
    pub filters: Vec<motion_core::Filter>,
    /// Source provenance inherited by descendants without their own interval.
    pub source: Option<Range<usize>>,
    /// Paint order, independent of source reading order.
    pub children: Vec<VectorItem>,
}

impl Default for VectorGroup {
    fn default() -> Self {
        Self {
            id: None,
            transform: Transform::IDENTITY,
            opacity: 1.0,
            clip: None,
            mask: None,
            blend_mode: motion_core::BlendMode::SourceOver,
            isolate: false,
            filters: Vec::new(),
            source: None,
            children: Vec::new(),
        }
    }
}

impl VectorGroup {
    fn node(&self) -> Node {
        self.decorate(self.children.iter().map(item_node))
    }

    pub(crate) fn decorate(&self, children: impl IntoIterator<Item = Node>) -> Node {
        let content = Node::group(children);
        let content = if self.isolate {
            Node::accumulate([content]).expect("one isolated content sample")
        } else {
            content
        };
        let mut node = content.opacity(self.opacity).blend_mode(self.blend_mode);
        for filter in &self.filters {
            node = node.filter(filter.clone());
        }
        if let Some(mask) = &self.mask {
            node = node.mask(
                mask.tree
                    .node()
                    .clip_with(mask.rect, motion_core::ClipMode::Geometric),
                mask.mode,
            );
        }
        if let Some(clip) = &self.clip {
            node = node.clip(clip.clone());
        }
        node = node.transformed(self.transform);
        if let Some(id) = &self.id {
            node = node.named(id);
        }
        node
    }
}

pub(crate) fn item_node(item: &VectorItem) -> Node {
    match item {
        VectorItem::Group(group) => group.node(),
        VectorItem::Path(path) => Node::from(path.clone()),
        VectorItem::Image(image) => Node::from(image.clone()),
    }
}

/// A leaf occurrence with complete world coordinates and inherited metadata.
#[derive(Clone, Debug)]
pub struct VectorLeaf<'a> {
    /// Path/image occurrence, not a copied geometry.
    pub item: &'a VectorItem,
    /// Local-to-tree coordinates.
    pub transform: Transform,
    /// Product of ancestor group opacities.
    pub opacity: f64,
    /// Nearest source interval; None uses paint order as reading order.
    pub source: Option<Range<usize>>,
    /// Child indices starting at the root group.
    pub address: Vec<usize>,
    pub(crate) clips: Vec<motion_core::PathClip>,
}

/// Immutable, cheaply cloned group/path/image tree with unique ID lookup.
/// Construction validates every branch, including invisible ones, under bounded
/// depth (48), items (8192), IDs (1024 bytes each) and aggregate verbs (262144).
#[derive(Clone, Debug)]
pub struct VectorTree {
    data: Arc<TreeData>,
}

#[derive(Clone, Debug)]
struct TreeData {
    view_box: Rect,
    root: VectorGroup,
    ids: BTreeMap<Arc<str>, Vec<usize>>,
    baseline: Option<f64>,
    surface: std::sync::OnceLock<Result<Arc<VectorSurface>, ValidationError>>,
}

impl VectorTree {
    /// Freezes a checked semantic snapshot. No fonts, devices or files are loaded.
    pub fn new(view_box: Rect, root: VectorGroup) -> Result<Self, ValidationError> {
        motion_core::drawing::validate_rect(view_box, "vector_tree", "view_box")?;
        let mut ids = BTreeMap::new();
        let mut count = 0;
        let mut verbs = 0;
        validate_group(&root, &mut Vec::new(), &mut ids, &mut count, &mut verbs)?;
        let tree = Self {
            data: Arc::new(TreeData {
                view_box,
                root,
                ids,
                baseline: None,
                surface: std::sync::OnceLock::new(),
            }),
        };
        tree.node().measure(ValidationLimits::default())?;
        Ok(tree)
    }

    /// Original logical viewport, not a raster or an inferred visible pixel box.
    pub fn view_box(&self) -> Rect {
        self.data.view_box
    }

    /// Read-only paint-order root.
    pub fn root(&self) -> &VectorGroup {
        &self.data.root
    }

    /// Stable ID-to-child-address index; the root group has an empty address.
    pub fn ids(&self) -> &BTreeMap<Arc<str>, Vec<usize>> {
        &self.data.ids
    }

    /// Looks up a non-root occurrence by ID without searching or mutating the tree.
    pub fn get(&self, id: &str) -> Option<&VectorItem> {
        let address = self.data.ids.get(id)?;
        let (last, parents) = address.split_last()?;
        let mut group = &self.data.root;
        for index in parents {
            let VectorItem::Group(child) = &group.children[*index] else {
                return None;
            };
            group = child;
        }
        group.children.get(*last)
    }

    /// Converts the same content to ordinary Node groups and drawables.
    pub fn node(&self) -> Node {
        self.node_mapped(&|_, node| node)
    }

    /// Builds nodes while applying an address-aware immutable modifier.
    /// Addresses follow the original shared tree, not the modified output nodes.
    pub fn node_mapped(&self, map: &impl Fn(&[usize], Node) -> Node) -> Node {
        fn build(
            group: &VectorGroup,
            address: &mut Vec<usize>,
            map: &impl Fn(&[usize], Node) -> Node,
        ) -> Node {
            let children = group
                .children
                .iter()
                .enumerate()
                .map(|(index, item)| {
                    address.push(index);
                    let node = match item {
                        VectorItem::Group(group) => build(group, address, map),
                        _ => map(address, item_node(item)),
                    };
                    address.pop();
                    node
                })
                .collect::<Vec<_>>();
            map(address, group.decorate(children))
        }
        let node = build(&self.data.root, &mut Vec::new(), map);
        if let Some(baseline) = self.data.baseline {
            Node::group([node, Node::from(TextBounds(self.data.view_box))])
                .with_text_metrics(baseline, self.data.view_box)
        } else {
            node
        }
    }

    /// Records a content importer's logical baseline for text-aware placement.
    pub fn with_baseline(mut self, baseline: f64) -> Self {
        Arc::make_mut(&mut self.data).baseline = Some(baseline);
        self
    }

    /// Paint-order occurrences with complete transforms and clip stacks.
    pub fn paint_leaves(&self) -> Vec<VectorLeaf<'_>> {
        let mut leaves = Vec::new();
        gather(
            &self.data.root,
            Transform::IDENTITY,
            1.0,
            None,
            &[],
            &mut Vec::new(),
            &mut leaves,
        );
        leaves
    }

    /// Stable source reading order, independent of visual order (for example bidi).
    /// Leaves without provenance retain paint order after source-bearing leaves.
    pub fn leaves(&self) -> Vec<VectorLeaf<'_>> {
        let mut leaves = self.paint_leaves();
        leaves.sort_by_key(|leaf| {
            leaf.source
                .as_ref()
                .map(|range| range.start)
                .unwrap_or(usize::MAX)
        });
        leaves
    }
}

#[derive(Clone)]
struct TextBounds(Rect);

impl Drawable for TextBounds {
    fn bounds(&self, _: &DrawContext) -> Result<Rect, ValidationError> {
        Ok(self.0)
    }

    fn draw(
        &self,
        _: &DrawContext,
        _: &mut motion_core::Canvas<'_>,
    ) -> Result<(), ValidationError> {
        Ok(())
    }
}

fn metadata(
    id: &Option<Arc<str>>,
    source: &Option<Range<usize>>,
    address: &[usize],
    ids: &mut BTreeMap<Arc<str>, Vec<usize>>,
) -> Result<(), ValidationError> {
    if source.as_ref().is_some_and(|range| range.start > range.end) {
        return Err(invalid("source", "Source ranges must not be inverted"));
    }
    if let Some(id) = id
        && (id.is_empty()
            || id.len() > 1024
            || id.chars().any(char::is_control)
            || ids.insert(id.clone(), address.to_vec()).is_some())
    {
        return Err(invalid(
            "id",
            "IDs must be unique nonempty strings of at most 1024 bytes without controls",
        ));
    }
    Ok(())
}

fn validate_group(
    group: &VectorGroup,
    address: &mut Vec<usize>,
    ids: &mut BTreeMap<Arc<str>, Vec<usize>>,
    count: &mut usize,
    verbs: &mut usize,
) -> Result<(), ValidationError> {
    *count += 1;
    if address.len() > 48 || *count > 8192 {
        return Err(limited("Vector tree depth/item allowance exhausted"));
    }
    metadata(&group.id, &group.source, address, ids)?;
    if group.filters.len() > 32 {
        return Err(limited("Vector group exceeds 32 filters"));
    }
    for filter in &group.filters {
        filter.validate("vector_tree.filters")?;
    }
    if let Some(mask) = &group.mask {
        motion_core::drawing::validate_rect(mask.rect, "vector_tree.mask", "rect")?;
        address.push(usize::MAX);
        validate_group(
            mask.tree.root(),
            address,
            &mut BTreeMap::new(),
            count,
            verbs,
        )?;
        address.pop();
    }
    if !group.transform.is_finite()
        || !group.opacity.is_finite()
        || !(0.0..=1.0).contains(&group.opacity)
    {
        return Err(invalid(
            "group",
            "Require finite matrices and opacity in 0..=1",
        ));
    }
    if let Some(clip) = &group.clip {
        *verbs += clip.path.verbs().len();
    }
    for (index, item) in group.children.iter().enumerate() {
        address.push(index);
        match item {
            VectorItem::Group(child) => validate_group(child, address, ids, count, verbs)?,
            VectorItem::Path(path) => {
                *count += 1;
                *verbs += path.path.verbs().len();
                metadata(&path.id, &path.source, address, ids)?;
                if let Some((paint, _)) = &path.fill {
                    paint.validate()?;
                }
                if let Some((style, paint)) = &path.stroke {
                    style.validate("vector_tree.stroke")?;
                    paint.validate()?;
                }
            }
            VectorItem::Image(image) => {
                *count += 1;
                metadata(&image.id, &image.source, address, ids)?;
                motion_core::drawing::validate_rect(image.rect, "vector_tree.image", "rect")?;
                if !image.pixel_transform.is_finite() {
                    return Err(invalid("transform", "Image pixel mapping must be finite"));
                }
                if image.rect.width() > 0.0 && image.rect.height() > 0.0 {
                    Paint::from(image.pattern()).validate()?;
                }
            }
        }
        address.pop();
        if *count > 8192 || *verbs > 262144 {
            return Err(limited("Vector tree item/verb allowance exhausted"));
        }
    }
    Ok(())
}

fn gather<'a>(
    group: &'a VectorGroup,
    parent: Transform,
    opacity: f64,
    source: Option<Range<usize>>,
    clips: &[motion_core::PathClip],
    address: &mut Vec<usize>,
    leaves: &mut Vec<VectorLeaf<'a>>,
) {
    let transform = group.transform.then(parent);
    let opacity = opacity * group.opacity;
    let source = group.source.clone().or(source);
    let mut clips = clips.to_vec();
    if let Some(mut clip) = group.clip.clone() {
        clip.transform = clip.transform.then(transform);
        clips.push(clip);
    }
    for (index, item) in group.children.iter().enumerate() {
        address.push(index);
        match item {
            VectorItem::Group(child) => gather(
                child,
                transform,
                opacity,
                source.clone(),
                &clips,
                address,
                leaves,
            ),
            _ => {
                let own = match item {
                    VectorItem::Path(path) => path.source.clone(),
                    VectorItem::Image(image) => image.source.clone(),
                    _ => None,
                };
                leaves.push(VectorLeaf {
                    item,
                    transform,
                    opacity,
                    source: own.or(source.clone()),
                    address: address.clone(),
                    clips: clips.clone(),
                });
            }
        }
        address.pop();
    }
}
