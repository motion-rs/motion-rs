//! Validated device-space drawing commands, with explicit transforms and resources.

use crate::drawing::validate_rect;
use crate::validation::ValidationState;
use crate::{
    Color, DrawCommand, Point, Rect, SceneStats, Size, Transform, ValidationError, ValidationLimits,
};

/// Integer output dimensions; construction does not allocate pixel storage.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct PixelSize {
    /// Output width, required to be positive during compilation/preflight.
    pub width: u32,
    /// Output height, required to be positive during compilation/preflight.
    pub height: u32,
}

impl PixelSize {
    /// Stores output dimensions; invalid zero dimensions fail preflight.
    pub const fn new(width: u32, height: u32) -> Self {
        Self { width, height }
    }

    fn validate(self) -> Result<(), ValidationError> {
        if self.width == 0 || self.height == 0 {
            return Err(ValidationError::invalid(
                "display_list",
                "size",
                "Output dimensions must be positive",
            ));
        }
        Ok(())
    }

    /// Returns centered contain mapping and its uniform logical-to-pixel scale.
    /// Rejects zero output, nonpositive design sizes and unrepresentable mapped extents.
    pub fn mapping(self, design: Size) -> Result<(Transform, f64), ValidationError> {
        self.validate()?;
        if !design.width.is_finite()
            || !design.height.is_finite()
            || design.width <= 0.0
            || design.height <= 0.0
        {
            return Err(ValidationError::invalid(
                "display_list",
                "design_size",
                "Design dimensions must be finite and positive",
            ));
        }
        let width = f64::from(self.width);
        let height = f64::from(self.height);
        let scale = (width / design.width).min(height / design.height);
        let mapped_width = design.width * scale;
        let mapped_height = design.height * scale;
        if !scale.is_finite()
            || scale <= 0.0
            || !mapped_width.is_finite()
            || !mapped_height.is_finite()
            || mapped_width == 0.0
            || mapped_height == 0.0
        {
            return Err(ValidationError::invalid(
                "display_list",
                "transform",
                "Design-to-device mapping must remain finite and nonzero",
            ));
        }
        let transform = Transform::scale_non_uniform(scale, scale).then(Transform::translate(
            (width - mapped_width) * 0.5,
            (height - mapped_height) * 0.5,
        ));
        Ok((transform, scale))
    }
}

/// Flat drawing commands with composed local-to-device transforms.
///
/// Fields remain inspectable/editable; consumers must preflight edited lists.
/// This is not a PreparedFrame or a renderer and allocates no output pixels.
#[derive(Clone, Debug, PartialEq)]
pub struct DisplayList {
    /// Integer output size, independent of logical design dimensions.
    pub output_size: PixelSize,
    /// Optional floating background color; None leaves the output transparent.
    pub background: Option<Color>,
    /// Ordered fills and balanced isolated-layer boundaries.
    pub commands: Vec<DrawCommand>,
}

/// Immutable command snapshot whose structure and properties have been checked.
/// Clones share commands without exposing mutation. Validation covers structure,
/// geometry and paint properties, not raster admission or device availability.
/// Tighter consumers must check their limits.
#[derive(Clone, Debug, PartialEq)]
pub struct ValidatedDisplayList {
    list: std::sync::Arc<DisplayList>,
    stats: SceneStats,
}

impl ValidatedDisplayList {
    /// Returns an explicitly editable copy, whose consumers must validate again.
    pub fn into_editable(self) -> DisplayList {
        std::sync::Arc::try_unwrap(self.list).unwrap_or_else(|list| list.as_ref().clone())
    }

    /// Sets only the output background without invalidating command requirements.
    pub fn with_background(mut self, background: Option<Color>) -> Self {
        std::sync::Arc::make_mut(&mut self.list).background = background;
        self
    }

    /// Checks positive structural limits against retained validation requirements.
    /// Rewalks commands only when their measured count/depth exceeds the new allowance,
    /// preserving the same detailed diagnostic as editable-list preflight.
    pub fn preflight(&self, limits: ValidationLimits) -> Result<SceneStats, ValidationError> {
        ValidationState::new(limits)?;
        if self.stats.nodes > limits.max_nodes || self.stats.max_depth > limits.max_depth {
            self.list.preflight(limits)
        } else {
            Ok(self.stats)
        }
    }

    /// Read-only checked commands and output description.
    pub fn display_list(&self) -> &DisplayList {
        &self.list
    }

    /// Requirements measured by the original full validation.
    pub fn stats(&self) -> SceneStats {
        self.stats
    }
}

impl std::ops::Deref for ValidatedDisplayList {
    type Target = DisplayList;

    fn deref(&self) -> &Self::Target {
        self.display_list()
    }
}

impl DisplayList {
    /// Consumes an editable list into a shared immutable validated snapshot.
    pub fn validate(
        self,
        limits: ValidationLimits,
    ) -> Result<ValidatedDisplayList, ValidationError> {
        let stats = self.preflight(limits)?;
        Ok(ValidatedDisplayList {
            list: std::sync::Arc::new(self),
            stats,
        })
    }

    /// Validates all commands, including transparent/offscreen geometry.
    ///
    /// Limits count commands and every path verb, and layer nesting determines
    /// output depth. This is not a pixel/allocation budget or backend probe.
    pub fn preflight(&self, limits: ValidationLimits) -> Result<SceneStats, ValidationError> {
        self.output_size.validate()?;
        let mut state = ValidationState::new(limits)?;
        let mut layers = ScopeStack::default();
        for (index, command) in self.commands.iter().enumerate() {
            validate_command(
                command,
                &mut state,
                &mut layers,
                &format!("display_list.commands[{index}]"),
            )?;
        }
        require_balanced(&layers)?;
        Ok(state.stats)
    }
}

/// Incrementally validates commands and layer grammar before producing a display list.
/// Discard the builder after an error; prior structural charges are not rolled back.
pub struct Builder {
    list: DisplayList,
    state: ValidationState,
    layers: ScopeStack,
}

impl Builder {
    /// Creates an empty list with checked output dimensions and structural limits.
    pub fn new(size: PixelSize, limits: ValidationLimits) -> Result<Self, ValidationError> {
        size.validate()?;
        Ok(Self {
            list: DisplayList {
                output_size: size,
                background: None,
                commands: Vec::new(),
            },
            state: ValidationState::new(limits)?,
            layers: ScopeStack::default(),
        })
    }

    /// Validates and charges a command at the supplied diagnostic path before storing it.
    pub fn push(&mut self, command: DrawCommand, path: &str) -> Result<(), ValidationError> {
        validate_command(&command, &mut self.state, &mut self.layers, path)?;
        self.list.commands.push(command);
        Ok(())
    }

    /// Returns the list only when every layer and mask scope has been closed.
    pub fn finish(self) -> Result<DisplayList, ValidationError> {
        require_balanced(&self.layers)?;
        Ok(self.list)
    }

    /// Freezes already checked commands without repeating structural traversal.
    pub fn finish_validated(self) -> Result<ValidatedDisplayList, ValidationError> {
        require_balanced(&self.layers)?;
        Ok(ValidatedDisplayList {
            list: std::sync::Arc::new(self.list),
            stats: self.state.stats,
        })
    }
}

#[derive(Clone, Copy)]
enum Scope {
    Layer { masked: bool },
    Mask,
}

struct ScopeStack {
    entries: [Scope; 128],
    len: usize,
}

impl Default for ScopeStack {
    fn default() -> Self {
        Self {
            entries: [Scope::Mask; 128],
            len: 0,
        }
    }
}

impl ScopeStack {
    fn last(&self) -> Option<Scope> {
        self.len.checked_sub(1).map(|index| self.entries[index])
    }
}

fn validate_command(
    command: &DrawCommand,
    state: &mut ValidationState,
    layers: &mut ScopeStack,
    path: &str,
) -> Result<(), ValidationError> {
    if let DrawCommand::FillRect { paint, .. } | DrawCommand::Fill { paint, .. } = command {
        paint.validate().map_err(|mut error| {
            error.path = path.to_owned();
            error
        })?;
    }

    if matches!(layers.last(), Some(Scope::Layer { masked: true }))
        && !matches!(command, DrawCommand::PopLayer)
    {
        return Err(ValidationError::invalid(
            path,
            "masks",
            "Only PopLayer may follow completed mask artwork",
        ));
    }

    let depth = match command {
        DrawCommand::PopLayer if !matches!(layers.last(), Some(Scope::Layer { .. })) => {
            return Err(ValidationError::invalid(
                path,
                "layers",
                "PopLayer requires an open layer",
            ));
        }
        DrawCommand::EndMask if !matches!(layers.last(), Some(Scope::Mask)) => {
            return Err(ValidationError::invalid(
                path,
                "masks",
                "EndMask requires an open mask phase",
            ));
        }
        DrawCommand::BeginMask { .. }
            if !matches!(layers.last(), Some(Scope::Layer { masked: false })) =>
        {
            return Err(ValidationError::invalid(
                path,
                "masks",
                "BeginMask requires an unmasked layer",
            ));
        }
        DrawCommand::PopLayer | DrawCommand::EndMask => layers.len,
        _ => layers.len + 1,
    };
    let verbs = match command {
        DrawCommand::ParticleInstances { batch, transform } => {
            batch.validate()?;
            mapped_bounds(batch.bounds(), *transform, path)?;
            batch.len()
        }
        DrawCommand::PushFilterLayer { bounds, filters } => {
            validate_rect(*bounds, path, "bounds")?;
            let mut current = *bounds;
            for (index, filter) in filters.iter().enumerate() {
                let path = format!("{path}.filters[{index}]");
                filter.validate(&path)?;
                for operation in 0..filter.structural_cost() {
                    state.enter_with(|| format!("{path}[{operation}]"), depth)?;
                }
                current = filter.extent(current, &path)?.0;
            }
            0
        }
        DrawCommand::PushLayer {
            opacity,
            bounds,
            clip,
            ..
        }
        | DrawCommand::PushRectLayer {
            opacity,
            bounds,
            clip,
            ..
        }
        | DrawCommand::PushShapeLayer {
            opacity,
            bounds,
            clip,
            ..
        } => {
            validate_rect(*bounds, path, "bounds")?;
            if let DrawCommand::PushRectLayer { rect_clip, .. }
            | DrawCommand::PushShapeLayer {
                rect_clip: Some(rect_clip),
                ..
            } = command
            {
                let clip_path = format!("{path}.rect_clip");
                mapped_bounds(rect_clip.bounds, rect_clip.transform, &clip_path)?;
                state.enter(&clip_path, depth)?;
            }
            if let DrawCommand::PushShapeLayer { shape_clip, .. } = command {
                let clip_path = format!("{path}.shape_clip");
                let verbs = shape_clip.validate(&clip_path)?;
                state.enter(&clip_path, depth)?;
                for index in 0..verbs {
                    state.enter_with(|| format!("{clip_path}.verbs[{index}]"), depth)?;
                }
            }
            if !opacity.is_finite() || !(0.0..=1.0).contains(opacity) {
                return Err(ValidationError::invalid(
                    path,
                    "opacity",
                    "Layer opacity must be finite in 0..=1",
                ));
            }
            if let Some(clip) = clip {
                validate_path(&clip.path, clip.transform, &format!("{path}.clip"))?;
                clip.path.verbs().len()
            } else {
                0
            }
        }
        DrawCommand::PopLayer | DrawCommand::BeginMask { .. } | DrawCommand::EndMask => 0,
        DrawCommand::FillRect {
            bounds, transform, ..
        } => {
            mapped_bounds(*bounds, *transform, path)?;
            0
        }
        DrawCommand::Fill {
            path: geometry,
            transform,
            ..
        } => {
            validate_path(geometry, *transform, path)?;
            geometry.verbs().len()
        }
    };
    state.enter(path, depth)?;
    for index in 0..verbs {
        let prefix = if matches!(
            command,
            DrawCommand::PushLayer { .. }
                | DrawCommand::PushRectLayer { .. }
                | DrawCommand::PushShapeLayer { .. }
        ) {
            format!("{path}.clip")
        } else {
            path.to_owned()
        };
        state.enter_with(|| format!("{prefix}.verbs[{index}]"), depth)?;
    }
    match command {
        DrawCommand::PushFilterLayer { .. }
        | DrawCommand::PushLayer { .. }
        | DrawCommand::PushRectLayer { .. }
        | DrawCommand::PushShapeLayer { .. } => {
            layers.entries[layers.len] = Scope::Layer { masked: false };
            layers.len += 1;
        }
        DrawCommand::BeginMask { .. } => {
            layers.entries[layers.len - 1] = Scope::Layer { masked: true };
            layers.entries[layers.len] = Scope::Mask;
            layers.len += 1;
        }
        DrawCommand::PopLayer | DrawCommand::EndMask => layers.len -= 1,
        _ => {}
    }
    Ok(())
}

/// Checks local and transformed path controls for finite, representable geometry.
/// This does not flatten curves, charge structural units or determine pixel coverage.
pub fn validate_path(
    geometry: &crate::Path2D,
    transform: Transform,
    path: &str,
) -> Result<(), ValidationError> {
    geometry.validate()?;
    if !transform.is_finite() {
        return Err(ValidationError::invalid(
            path,
            "transform",
            "Command transform must be finite",
        ));
    }
    let mut hull: Option<Rect> = None;
    for (index, verb) in geometry.verbs().iter().enumerate() {
        for point in verb.points().into_iter().flatten() {
            let point = transform.apply(point);
            if !point.x.is_finite() || !point.y.is_finite() {
                return Err(ValidationError::invalid(
                    format!("{path}.verbs[{index}]"),
                    "coordinates",
                    "Transformed path coordinates must remain finite",
                ));
            }
            hull = Some(
                hull.map_or(Rect::new(point.x, point.y, point.x, point.y), |rect| {
                    Rect::new(
                        rect.left.min(point.x),
                        rect.top.min(point.y),
                        rect.right.max(point.x),
                        rect.bottom.max(point.y),
                    )
                }),
            );
        }
    }
    if let Some(hull) = hull {
        validate_rect(hull, path, "coordinates")?;
    }
    Ok(())
}

/// Maps all rectangle corners and checks finite coordinates and enclosing spans.
/// Returns a conservative axis-aligned bound, not a geometric clipping region.
pub fn mapped_bounds(
    bounds: Rect,
    transform: Transform,
    path: &str,
) -> Result<Rect, ValidationError> {
    validate_rect(bounds, path, "bounds")?;
    if !transform.is_finite() {
        return Err(ValidationError::invalid(
            path,
            "transform",
            "Command transform must be finite",
        ));
    }
    let corners = [
        Point::new(bounds.left, bounds.top),
        Point::new(bounds.right, bounds.top),
        Point::new(bounds.right, bounds.bottom),
        Point::new(bounds.left, bounds.bottom),
    ]
    .map(|point| transform.apply(point));
    if corners
        .iter()
        .any(|point| !point.x.is_finite() || !point.y.is_finite())
    {
        return Err(ValidationError::invalid(
            path,
            "transform",
            "Mapped geometry must remain finite",
        ));
    }
    let rect = Rect::new(
        corners
            .iter()
            .map(|point| point.x)
            .fold(f64::INFINITY, f64::min),
        corners
            .iter()
            .map(|point| point.y)
            .fold(f64::INFINITY, f64::min),
        corners
            .iter()
            .map(|point| point.x)
            .fold(f64::NEG_INFINITY, f64::max),
        corners
            .iter()
            .map(|point| point.y)
            .fold(f64::NEG_INFINITY, f64::max),
    );
    validate_rect(rect, path, "bounds")?;
    Ok(rect)
}

fn require_balanced(layers: &ScopeStack) -> Result<(), ValidationError> {
    if layers.len != 0 {
        return Err(ValidationError::invalid(
            "display_list",
            "layers",
            "Every layer and mask phase must close in nesting order",
        ));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{FillRule, Paint, Path2D};
    use std::sync::Arc;

    fn rect() -> DrawCommand {
        DrawCommand::FillRect {
            bounds: Rect::new(0.0, 0.0, 10.0, 20.0),
            transform: Transform::translate(0.125, 0.25),
            paint: Paint::Solid(Color::WHITE),
        }
    }

    fn list(commands: Vec<DrawCommand>) -> DisplayList {
        DisplayList {
            output_size: PixelSize::new(200, 200),
            background: None,
            commands,
        }
    }

    #[test]
    fn contain_mapping_centers_design_space_and_rejects_invalid_dimensions() {
        let (transform, scale) = PixelSize::new(200, 200)
            .mapping(Size::new(100.0, 50.0))
            .unwrap();
        assert_eq!(scale, 2.0);
        assert_eq!(transform.apply(Point::ORIGIN), Point::new(0.0, 50.0));
        assert_eq!(
            transform.apply(Point::new(100.0, 50.0)),
            Point::new(200.0, 150.0)
        );
        for size in [
            Size::new(0.0, 1.0),
            Size::new(f64::NAN, 1.0),
            Size::new(f64::from_bits(1), f64::MAX),
        ] {
            assert!(PixelSize::new(200, 200).mapping(size).is_err());
        }
        assert!(PixelSize::new(0, 100).mapping(Size::new(1.0, 1.0)).is_err());
    }

    #[test]
    fn balanced_layers_have_exact_output_work_and_depth_limits() {
        let commands = vec![
            DrawCommand::PushLayer {
                opacity: 0.5,
                bounds: Rect::new(0.0, 0.0, 10.0, 20.0),
                blend_mode: crate::drawing::BlendMode::SourceOver,
                clip: None,
            },
            rect(),
            DrawCommand::PopLayer,
        ];
        let list = list(commands);
        let exact = ValidationLimits {
            max_nodes: 3,
            max_depth: 2,
        };
        assert_eq!(list.preflight(exact).unwrap().nodes, 3);
        assert_eq!(
            list.preflight(ValidationLimits {
                max_nodes: 2,
                ..exact
            })
            .unwrap_err()
            .field,
            "max_nodes"
        );
        assert_eq!(
            list.preflight(ValidationLimits {
                max_depth: 1,
                ..exact
            })
            .unwrap_err()
            .field,
            "max_depth"
        );
    }

    #[test]
    fn edited_lists_reject_unbalanced_or_invalid_layers() {
        assert_eq!(
            list(vec![DrawCommand::PopLayer])
                .preflight(ValidationLimits::default())
                .unwrap_err()
                .field,
            "layers"
        );
        for opacity in [0.5, f64::NAN] {
            let error = list(vec![DrawCommand::PushLayer {
                opacity,
                bounds: Rect::default(),
                blend_mode: crate::drawing::BlendMode::SourceOver,
                clip: None,
            }])
            .preflight(ValidationLimits::default())
            .unwrap_err();
            assert_eq!(
                error.field,
                if opacity.is_nan() {
                    "opacity"
                } else {
                    "layers"
                }
            );
        }
    }

    #[test]
    fn transformed_controls_validate_even_when_transformed_curve_bounds_fit() {
        let mut geometry = Path2D::new();
        geometry.try_move_to(Point::ORIGIN).unwrap();
        geometry
            .try_quadratic_to(Point::new(1.0e308, 1.0), Point::new(0.0, 2.0))
            .unwrap();
        let command = DrawCommand::Fill {
            path: Arc::new(geometry),
            transform: Transform::scale_non_uniform(2.0, 1.0),
            paint: Paint::Solid(Color::WHITE),
            rule: FillRule::NonZero,
        };
        let error = list(vec![command])
            .preflight(ValidationLimits::default())
            .unwrap_err();
        assert_eq!(error.field, "coordinates");
        assert!(error.path.contains(".verbs[1]"));
    }

    #[test]
    fn builder_charges_each_path_verb_before_retaining_output() {
        let geometry = Path2D::from_rect(Rect::new(0.0, 0.0, 10.0, 20.0)).unwrap();
        let command = DrawCommand::Fill {
            path: Arc::new(geometry),
            transform: Transform::IDENTITY,
            paint: Paint::Solid(Color::WHITE),
            rule: FillRule::NonZero,
        };
        let mut exact = Builder::new(
            PixelSize::new(100, 100),
            ValidationLimits {
                max_nodes: 6,
                max_depth: 1,
            },
        )
        .unwrap();
        exact.push(command.clone(), "native").unwrap();
        assert_eq!(exact.finish().unwrap().commands.len(), 1);
        let mut short = Builder::new(
            PixelSize::new(100, 100),
            ValidationLimits {
                max_nodes: 5,
                max_depth: 1,
            },
        )
        .unwrap();
        assert_eq!(
            short.push(command, "native").unwrap_err().field,
            "max_nodes"
        );
        assert!(short.list.commands.is_empty());
    }
}

#[cfg(test)]
mod validated_tests {
    use super::*;

    #[test]
    fn immutable_snapshots_recheck_strict_budgets_and_preserve_diagnostics() {
        let command = DrawCommand::FillRect {
            bounds: Rect::new(0.0, 0.0, 2.0, 2.0),
            transform: Transform::IDENTITY,
            paint: Color::WHITE.into(),
        };
        let mut editable = DisplayList {
            output_size: PixelSize::new(4, 4),
            background: None,
            commands: vec![command.clone(), command],
        };
        let validated = editable
            .clone()
            .validate(ValidationLimits::default())
            .unwrap();
        editable.commands.clear();
        assert_eq!(validated.display_list().commands.len(), 2);
        let cloned = validated.clone();
        assert!(std::ptr::eq(
            cloned.display_list(),
            validated.display_list()
        ));
        let limits = ValidationLimits {
            max_nodes: 1,
            ..ValidationLimits::default()
        };
        assert_eq!(
            validated.preflight(limits).unwrap_err(),
            validated.display_list().preflight(limits).unwrap_err()
        );
        assert_eq!(
            validated.preflight(ValidationLimits::default()).unwrap(),
            validated.stats()
        );
    }
}
