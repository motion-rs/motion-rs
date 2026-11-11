//! The `Svg` node builder with lazy loading, viewport fitting, and failure placeholders.

use super::*;
use motion_core::{
    Canvas, DeferredContent, DrawContext, Drawable, Node, NodeModifiers, ValidationError,
};

type Selection = Arc<dyn Fn(Node) -> Node + Send + Sync>;

#[derive(Clone)]
enum Input {
    Source(motion_core::Source),
    Lazy(motion_core::Handle<SvgAsset>),
    Loaded(Arc<SvgAsset>),
}

/// resource-backed SVG operations; build/compile uses the same Node path
/// for preview, still and export. Callbacks run synchronously on the CPU.
#[derive(Clone)]
pub struct Svg {
    input: Input,
    size: Option<Size>,
    progress: Option<f64>,
    selections: Vec<(Arc<str>, Selection)>,
}

impl Svg {
    /// Captures a source resolved using session resources and fonts during compilation.
    pub fn open(source: impl Into<motion_core::Source>) -> Self {
        Self {
            input: Input::Source(source.into()),
            size: None,
            progress: None,
            selections: Vec::new(),
        }
    }

    /// Captures a lazy resource without reading or parsing it.
    pub fn new(handle: motion_core::Handle<SvgAsset>) -> Self {
        Self {
            input: Input::Lazy(handle),
            size: None,
            progress: None,
            selections: Vec::new(),
        }
    }

    /// Captures an already validated immutable vector asset.
    pub fn from_asset(asset: Arc<SvgAsset>) -> Self {
        Self {
            input: Input::Loaded(asset),
            size: None,
            progress: None,
            selections: Vec::new(),
        }
    }

    /// Centers a contain mapping in this finite positive viewport.
    pub fn fit(mut self, size: Size) -> Self {
        self.size = Some(size);
        self
    }

    /// Uses the common cumulative vector handwriting effect.
    pub fn write(mut self, progress: f64) -> Self {
        self.progress = Some(progress);
        self
    }

    /// Applies a normal modifier callback to an authored ID (# is optional).
    /// Unknown IDs fail rather than silently selecting nothing. Callbacks follow
    /// declaration order; resource IDs in masks are scoped to their resource.
    pub fn select(
        mut self,
        id: impl Into<Arc<str>>,
        callback: impl Fn(Node) -> Node + Send + Sync + 'static,
    ) -> Self {
        let id = id.into();
        self.selections.push((
            Arc::from(id.strip_prefix('#').unwrap_or(&id)),
            Arc::new(callback),
        ));
        self
    }

    fn asset(&self, context: &DrawContext) -> motion_core::Result<Arc<SvgAsset>> {
        match &self.input {
            Input::Source(source) => context
                .assets()
                .load::<SvgAsset>(source.clone())
                .get_with(context.diagnostics()),
            Input::Lazy(handle) => handle.get_with(context.diagnostics()),
            Input::Loaded(asset) => Ok(asset.clone()),
        }
    }

    /// Returns the canonical shared vector model for inspection/effects.
    pub fn vector_tree(&self) -> motion_core::Result<Arc<VectorTree>> {
        Ok(self.asset(&DrawContext::default())?.tree.clone())
    }

    /// Builds checked content and reports invalid sizing/IDs/progress.
    pub fn try_node(&self) -> motion_core::Result<Node> {
        self.node_with(&DrawContext::default())
    }

    fn node_with(&self, context: &DrawContext) -> motion_core::Result<Node> {
        let asset = self.asset(context)?;
        let size = self.size.unwrap_or(asset.size());
        if !size.width.is_finite()
            || !size.height.is_finite()
            || size.width <= 0.0
            || size.height <= 0.0
            || self.selections.len() > 256
        {
            return Err(unsupported(
                "SVG fit requires positive finite size; at most 256 selections",
            ));
        }
        for (id, _) in &self.selections {
            if !asset.tree.ids().contains_key(id.as_ref()) {
                return Err(unsupported(format!("SVG selection ID not found: {id}")));
            }
        }
        let selections = self
            .selections
            .iter()
            .map(|(id, modify)| {
                VectorSelection::shared([asset.tree.ids()[id.as_ref()].clone()], modify)
            })
            .collect::<Vec<_>>();
        let mut node = asset.tree.reveal(&selections, self.progress)?;
        let scale = (size.width / asset.size().width).min(size.height / asset.size().height);
        let transform = Transform::scale(scale).then(Transform::translate(
            (size.width - asset.size().width * scale) * 0.5,
            (size.height - asset.size().height * scale) * 0.5,
        ));
        node = node
            .clip_with(asset.size().to_rect(), motion_core::ClipMode::Geometric)
            .transformed(transform)
            .clip_with(size.to_rect(), motion_core::ClipMode::Geometric);
        Ok(Node::group([node, Node::from(Viewport(size.to_rect()))])
            .pivot(motion_core::Pivot::Point(size.to_rect().center())))
    }
}

#[derive(Clone)]
struct Viewport(Rect);

impl Drawable for Viewport {
    fn bounds(&self, _: &DrawContext) -> Result<Rect, ValidationError> {
        Ok(self.0)
    }

    fn draw(&self, _: &DrawContext, _: &mut Canvas<'_>) -> Result<(), ValidationError> {
        Ok(())
    }
}

#[derive(Clone)]
struct FailedResource(Svg);

impl Drawable for FailedResource {
    fn bounds(&self, context: &DrawContext) -> Result<Rect, ValidationError> {
        let _ = self.0.asset(context);
        Ok(Rect::new(
            0.0,
            0.0,
            self.0.size.map_or(64.0, |size| size.width),
            self.0.size.map_or(64.0, |size| size.height),
        ))
    }

    fn draw(&self, context: &DrawContext, canvas: &mut Canvas<'_>) -> Result<(), ValidationError> {
        let _ = self.0.asset(context);
        canvas.fill(
            &self.bounds(context)?,
            Color::rgb(255, 0, 255),
            motion_core::FillRule::NonZero,
        )
    }
}

#[derive(Clone)]
struct DeferredSvg(Svg);

impl DeferredContent for DeferredSvg {
    fn resolve(&self, context: &DrawContext) -> Result<Node, ValidationError> {
        match self.0.node_with(context) {
            Ok(node) => Ok(node),
            Err(error) => {
                if error.kind() == motion_core::ErrorKind::Asset {
                    Ok(Node::from(FailedResource(self.0.clone())))
                } else {
                    Err(ValidationError::invalid(
                        "svg",
                        "content",
                        error.to_string(),
                    ))
                }
            }
        }
    }
}

impl From<Svg> for Node {
    fn from(svg: Svg) -> Node {
        Node::deferred(DeferredSvg(svg))
    }
}
