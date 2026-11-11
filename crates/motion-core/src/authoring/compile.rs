//! Scene compilation: bounded layout/recording followed by device emission.

use std::fmt::Write;

use super::spatial::Bounds;
use super::{Node, NodeKind};
use crate::display_list::{Builder, mapped_bounds};
use crate::drawing::BlendMode;
use crate::validation::ValidationState;
use crate::{
    Canvas, DrawCommand, DrawContext, PixelSize, Size, Transform, ValidatedDisplayList,
    ValidationError, ValidationLimits,
};

mod inspection;
#[cfg(test)]
mod tests;

pub use inspection::NodeInspection;

/// Spare bytes reserved after a parent path for one `.children[n]` or `.commands[n]` suffix.
const CHILD_PATH_SLACK: usize = 24;

impl Node {
    /// Compiles content to flat device-space commands without rendering.
    ///
    /// Logical design space is contained and centered in the output dimensions.
    /// bounds/draw callbacks receive the resulting device scale once per
    /// occurrence. Layout and stable sibling z order precede matrix composition.
    /// Nonunit opacity isolates its whole subtree without f32 narrowing.
    ///
    /// All input branches and emitted work validate, including empty, transparent
    /// and offscreen content. There is no culling yet. Source and output have independent
    /// instances of the supplied limits. This does not prepare coverage/pixels.
    ///
    /// ```
    /// use motion_core::prelude::*;
    /// use motion_core::{
    ///     DisplayList, RasterLimits, ResourceStore, ValidatedDisplayList, MAX_PATH_VERBS,
    ///     MAX_STROKE_VERBS, MAX_SVG_PATH_BYTES, sample_offsets,
    /// };
    /// use motion_core::authoring::Scene as _;
    /// let cache = SceneCache::new(0);
    /// let context = SceneContext::new(
    ///     Time::ZERO,
    ///     fps!(30),
    ///     Size::new(100.0, 50.0),
    ///     &cache,
    /// );
    /// let scene = |context: &SceneContext<'_>| {
    ///     let path = Path2D::from_rect(Rect::new(0.0, 0.0, 10.0, 10.0)).unwrap();
    ///     Row::new()
    ///         .gap(4.0)
    ///         .children([
    ///             Node::from(Rect::new(0.0, 0.0, 10.0, 20.0).fill(Color::WHITE)),
    ///             Node::from(path.fill(Color::from_hex(0x24a49b))),
    ///         ])
    ///         .at(context.center())
    ///         .opacity(0.5)
    /// };
    /// let list = scene.render(&context).compile(
    ///     context.size(),
    ///     PixelSize::new(200, 200),
    ///     ValidationLimits::default(),
    /// )?;
    /// assert_eq!(list.commands.len(), 4);
    /// assert!(matches!(
    ///     list.commands[0],
    ///     DrawCommand::PushLayer { opacity: 0.5, .. }
    /// ));
    /// assert!(matches!(
    ///     list.commands[2],
    ///     DrawCommand::Fill {
    ///         rule: FillRule::NonZero,
    ///         ..
    ///     }
    /// ));
    /// list.preflight(ValidationLimits::default())?;
    /// # Ok::<(), motion_core::ValidationError>(())
    /// ```
    pub fn compile(
        &self,
        design_size: Size,
        output_size: PixelSize,
        limits: ValidationLimits,
    ) -> Result<ValidatedDisplayList, ValidationError> {
        self.compile_in_fonts(design_size, output_size, limits, None)
    }

    /// Compiles with one immutable font environment shared by paragraph callbacks.
    pub fn compile_with_fonts(
        &self,
        design_size: Size,
        output_size: PixelSize,
        limits: ValidationLimits,
        fonts: impl Into<crate::asset::environment::FontEnvironment>,
    ) -> Result<ValidatedDisplayList, ValidationError> {
        let fonts = fonts.into();
        self.compile_in_fonts(design_size, output_size, limits, Some(&fonts))
    }

    fn compile_in_fonts(
        &self,
        design_size: Size,
        output_size: PixelSize,
        limits: ValidationLimits,
        fonts: Option<&crate::asset::environment::FontEnvironment>,
    ) -> Result<ValidatedDisplayList, ValidationError> {
        let mut context = DrawContext::default();
        if let Some(fonts) = fonts {
            context = context.with_fonts(fonts.clone());
        }
        self.compile_with_context(design_size, output_size, limits, &context)
    }

    /// Compiles with explicit resources and a reusable session layout cache.
    pub fn compile_with_context(
        &self,
        design_size: Size,
        output_size: PixelSize,
        limits: ValidationLimits,
        drawing: &DrawContext,
    ) -> Result<ValidatedDisplayList, ValidationError> {
        let (root, scale) = output_size.mapping(design_size)?;
        let mut context = drawing.clone().with_validation_limits(limits);
        context.device_scale = scale;
        let mut source = ValidationState::new(limits)?;
        let plan = plan(self, "node", 1, &context, &mut source)?;
        let mut output = Builder::new(output_size, limits)?;
        plan.emit(root, &mut output)?;
        output.finish_validated()
    }
}

struct Plan {
    transform: Transform,
    bounds: Bounds,
    opacity: f64,
    blend_mode: BlendMode,
    filters: Vec<crate::Filter>,
    clip: Option<crate::PathClip>,
    rect_clip: Option<crate::RectClip>,
    shape_clip: Option<crate::ShapeClip>,
    mask: Option<(Box<Plan>, crate::drawing::MaskMode)>,
    path: String,
    content: Content,
    has_commands: bool,
    accumulate: bool,
}

enum Content {
    Leaf(Vec<DrawCommand>),
    Children(Vec<Plan>),
}

fn plan(
    node: &Node,
    path: &str,
    depth: usize,
    context: &DrawContext,
    state: &mut ValidationState,
) -> Result<Plan, ValidationError> {
    let context = &context
        .clone()
        .with_time(node.data.time.unwrap_or(context.time()));
    let path = node
        .data
        .name
        .as_ref()
        .map_or_else(|| path.to_owned(), |name| format!("{path}(name={name})"));
    state.enter(&path, depth)?;
    let opacity = node.data.opacity;
    if !opacity.is_finite() || !(0.0..=1.0).contains(&opacity) {
        return Err(ValidationError::invalid(
            &path,
            "opacity",
            "Require finite opacity in 0..=1",
        ));
    }
    node.validate_clip(&path, depth, state)?;
    let mask = node
        .data
        .mask
        .as_ref()
        .map(|mask| {
            plan(
                &mask.node,
                &format!("{path}.mask"),
                depth + 1,
                context,
                state,
            )
            .map(|planned| (Box::new(planned), mask.mode))
        })
        .transpose()?;
    let (content, bounds, has_commands) = match &node.data.kind {
        NodeKind::Empty => (Content::Children(Vec::new()), Bounds::Empty, false),
        NodeKind::Relative(relative) => {
            let mut subject = plan(
                &relative.node,
                &format!("{path}.subject"),
                depth + 1,
                context,
                state,
            )?;
            let reference = relative.reference.measure_bounds(
                &format!("{path}.reference"),
                depth + 1,
                state,
                context,
            )?;
            let translation =
                relative.translation(subject.bounds.rect(&path)?, reference.rect(&path)?)?;
            subject.transform = subject.transform.then(translation);
            subject.bounds = subject.bounds.transformed(translation, &path)?;
            let bounds = subject.bounds;
            let has_commands = subject.has_commands;
            (Content::Children(vec![subject]), bounds, has_commands)
        }
        NodeKind::Leaf(drawable) => {
            let leaf = record_leaf(drawable.as_ref(), &path, depth, context, state);
            let (bounds, commands) = match leaf {
                Err(error)
                    if context.marks_invalid_geometry()
                        && error.kind == crate::ValidationErrorKind::InvalidValue =>
                {
                    invalid_geometry_marker(&path, depth, state)?
                }
                leaf => leaf?,
            };
            let has_commands = !commands.is_empty();
            (Content::Leaf(commands), Bounds::Known(bounds), has_commands)
        }
        NodeKind::Deferred(content) => {
            let resolved = content.resolve(context).map_err(|mut error| {
                error.path = format!("{path}.{}", error.path);
                error
            })?;
            let expanded = plan(
                &resolved,
                &format!("{path}.resolved"),
                depth + 1,
                context,
                state,
            )?;
            let bounds = expanded.bounds;
            let has_commands = expanded.has_commands;
            (Content::Children(vec![expanded]), bounds, has_commands)
        }
        NodeKind::Group(children)
        | NodeKind::Layout(_, children)
        | NodeKind::Accumulate(children) => {
            let mut planned = Vec::with_capacity(children.len());
            let mut child_path = String::with_capacity(path.len() + CHILD_PATH_SLACK);
            for (index, child) in children.iter().enumerate() {
                child_path.clear();
                let _ = write!(child_path, "{path}.children[{index}]");
                planned.push((index, plan(child, &child_path, depth + 1, context, state)?));
            }
            if let NodeKind::Layout(layout, _) = node.data.kind {
                let geometry: Vec<_> = planned
                    .iter()
                    .map(|(index, child)| (*index, child.bounds))
                    .collect();
                let baselines = layout.baselines(children, context, &path)?;
                for ((_, child), placement) in planned
                    .iter_mut()
                    .zip(layout.placements(&geometry, &baselines, &path)?)
                {
                    if let Some((placement, bounds)) = placement {
                        child.transform = child.transform.then(placement.transform);
                        child.bounds = bounds;
                    }
                }
            }
            planned.sort_by_key(|(index, _)| children[*index].data.z_index);
            let mut bounds = Bounds::Empty;
            let mut has_commands = false;
            let children = planned
                .into_iter()
                .map(|(_, child)| {
                    bounds = bounds.union(child.bounds);
                    has_commands |= child.has_commands;
                    child
                })
                .collect();
            (Content::Children(children), bounds, has_commands)
        }
    };
    let placement = node.data.placement.resolve(bounds, &path)?;
    let bounds = bounds.transformed(placement.transform, &path)?;
    Ok(Plan {
        transform: placement.transform,
        bounds,
        opacity,
        blend_mode: node.data.blend_mode,
        filters: node.data.filters.clone(),
        clip: node.data.clip.clone(),
        rect_clip: node.data.rect_clip,
        shape_clip: node.data.shape_clip.clone(),
        mask,
        path,
        content,
        has_commands,
        accumulate: matches!(node.data.kind, NodeKind::Accumulate(_)),
    })
}

fn record_leaf(
    drawable: &dyn crate::Drawable,
    path: &str,
    depth: usize,
    context: &DrawContext,
    state: &mut ValidationState,
) -> Result<(crate::Rect, Vec<DrawCommand>), ValidationError> {
    let bounds = drawable.bounds(context)?;
    crate::drawing::validate_rect(bounds, path, "bounds")?;
    let commands = Canvas::new(state, path, depth + 1).record(drawable, context)?;
    Ok((bounds, commands))
}

/// Magenta square at the leaf's local origin standing in for geometry that failed validation.
fn invalid_geometry_marker(
    path: &str,
    depth: usize,
    state: &mut ValidationState,
) -> Result<(crate::Rect, Vec<DrawCommand>), ValidationError> {
    let bounds = crate::Rect::new(0.0, 0.0, 24.0, 24.0);
    let mut canvas = Canvas::new(state, path, depth + 1);
    canvas.fill_rect(bounds, crate::Color::from_rgb_float(1.0, 0.0, 1.0))?;
    Ok((bounds, canvas.finish()?))
}

impl Plan {
    fn input_bounds(&self, parent: Transform) -> Result<Bounds, ValidationError> {
        let transform = self.transform.then(parent);
        match &self.content {
            Content::Leaf(_) => self.bounds.transformed(parent, &self.path),
            Content::Children(children) => {
                let mut bounds = Bounds::Empty;
                for child in children {
                    bounds = bounds.union(child.visual_bounds(transform)?);
                }
                Ok(bounds)
            }
        }
    }

    fn visual_bounds(&self, parent: Transform) -> Result<Bounds, ValidationError> {
        let bounds = self.input_bounds(parent)?;
        if let Bounds::Known(mut bounds) = bounds {
            for filter in &self.filters {
                bounds = filter
                    .device(self.transform.then(parent), &self.path)?
                    .extent(bounds, &self.path)?
                    .0;
            }
            Ok(Bounds::Known(bounds))
        } else {
            Ok(bounds)
        }
    }

    fn emit(self, parent: Transform, output: &mut Builder) -> Result<(), ValidationError> {
        let transform = self.transform.then(parent);
        if !transform.is_finite() {
            return Err(ValidationError::invalid(
                &self.path,
                "transform",
                "Composed device transform must remain finite",
            ));
        }
        let input_bounds = self.input_bounds(parent)?;
        let visual_bounds = self.visual_bounds(parent)?;
        let filters = self
            .filters
            .iter()
            .map(|filter| filter.device(transform, &self.path))
            .collect::<Result<Vec<_>, _>>()?;
        let rect_clip = self
            .rect_clip
            .map(|mut clip| {
                clip.transform = clip.transform.then(transform);
                mapped_bounds(
                    clip.bounds,
                    clip.transform,
                    &format!("{}.rect_clip", self.path),
                )?;
                Ok::<_, ValidationError>(clip)
            })
            .transpose()?;
        let shape_clip = self
            .shape_clip
            .map(|mut clip| {
                clip.transform = clip.transform.then(transform);
                clip.validate(&format!("{}.shape_clip", self.path))?;
                Ok::<_, ValidationError>(clip)
            })
            .transpose()?;
        let isolate = (self.accumulate
            || self.opacity != 1.0
            || self.blend_mode != BlendMode::SourceOver
            || self.clip.is_some()
            || self.rect_clip.is_some()
            || shape_clip.is_some()
            || self.mask.is_some()
            || !filters.is_empty())
            && self.has_commands;
        if isolate {
            let bounds = visual_bounds.rect(&self.path)?;
            let clip = self.clip.map(|mut clip| {
                clip.transform = clip.transform.then(transform);
                clip
            });
            let command = if let Some(shape_clip) = shape_clip {
                DrawCommand::PushShapeLayer {
                    opacity: self.opacity,
                    bounds,
                    blend_mode: self.blend_mode,
                    clip,
                    rect_clip,
                    shape_clip,
                }
            } else if let Some(rect_clip) = rect_clip {
                DrawCommand::PushRectLayer {
                    opacity: self.opacity,
                    bounds,
                    blend_mode: self.blend_mode,
                    clip,
                    rect_clip,
                }
            } else {
                DrawCommand::PushLayer {
                    opacity: self.opacity,
                    bounds,
                    blend_mode: self.blend_mode,
                    clip,
                }
            };
            output.push(command, &format!("{}.push_layer", self.path))?;
        }
        if self.has_commands && !filters.is_empty() {
            output.push(
                DrawCommand::PushFilterLayer {
                    bounds: input_bounds.rect(&self.path)?,
                    filters,
                },
                &format!("{}.filters", self.path),
            )?;
        }
        match self.content {
            Content::Leaf(commands) => {
                let mut command_path = String::with_capacity(self.path.len() + CHILD_PATH_SLACK);
                for (index, mut command) in commands.into_iter().enumerate() {
                    match &mut command {
                        DrawCommand::FillRect {
                            transform: local, ..
                        }
                        | DrawCommand::Fill {
                            transform: local, ..
                        }
                        | DrawCommand::ParticleInstances {
                            transform: local, ..
                        } => *local = local.then(transform),
                        DrawCommand::PushFilterLayer { .. }
                        | DrawCommand::PushLayer { .. }
                        | DrawCommand::PushRectLayer { .. }
                        | DrawCommand::PushShapeLayer { .. }
                        | DrawCommand::PopLayer
                        | DrawCommand::BeginMask { .. }
                        | DrawCommand::EndMask => {
                            return Err(ValidationError::invalid(
                                &self.path,
                                "commands",
                                "Local drawables cannot supply compiled layers",
                            ));
                        }
                    }
                    command_path.clear();
                    let _ = write!(command_path, "{}.commands[{index}]", self.path);
                    output.push(command, &command_path)?;
                }
            }
            Content::Children(children) => {
                for child in children {
                    child.emit(transform, output)?;
                }
            }
        }
        if self.has_commands && !self.filters.is_empty() {
            output.push(DrawCommand::PopLayer, &format!("{}.filters.pop", self.path))?;
        }
        if isolate {
            if let Some((mask, mode)) = self.mask {
                output.push(
                    DrawCommand::BeginMask { mode },
                    &format!("{}.begin_mask", self.path),
                )?;
                mask.emit(transform, output)?;
                output.push(DrawCommand::EndMask, &format!("{}.end_mask", self.path))?;
            }
            output.push(DrawCommand::PopLayer, &format!("{}.pop_layer", self.path))?;
        }
        Ok(())
    }
}

#[cfg(test)]
mod deferred_content_tests {
    use super::*;
    use crate::Shape;
    use crate::{Color, DeferredContent, NodeModifiers, Rect, TimeLiteral};

    struct Deferred;

    impl DeferredContent for Deferred {
        fn resolve(&self, context: &DrawContext) -> Result<Node, ValidationError> {
            Ok(Rect::new(0.0, 0.0, 2.0 + context.time().as_seconds(), 1.0)
                .fill(Color::WHITE)
                .into())
        }
    }

    #[test]
    fn deferred_content_preserves_outer_placement_and_inherited_local_time() {
        let node = Node::deferred(Deferred)
            .translate(10.0, 0.0)
            .with_scene_time(1.seconds().after_origin());
        assert_eq!(
            node.measure(ValidationLimits::default()).unwrap(),
            Rect::new(10.0, 0.0, 13.0, 1.0)
        );
        let list = node
            .compile(
                Size::new(20.0, 4.0),
                PixelSize::new(20, 4),
                ValidationLimits::default(),
            )
            .unwrap();
        assert_eq!(list.commands.len(), 1);
    }
}
