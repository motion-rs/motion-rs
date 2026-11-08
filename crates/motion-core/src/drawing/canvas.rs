//! Bounded command recording for drawable content.

use super::*;

/// A bounded command recorder passed to content.
///
/// The first failure poisons the recording, even when a callback ignores the
/// returned error. Geometry is checked and budget charged before storage grows.
pub struct Canvas<'a> {
    commands: Vec<DrawCommand>,
    state: &'a mut ValidationState,
    path: &'a str,
    depth: usize,
    error: Option<ValidationError>,
}

impl<'a> Canvas<'a> {
    /// Records evaluated instances without expanding them into path geometry.
    pub fn instances(
        &mut self,
        batch: Arc<dyn crate::instances::InstanceBatch>,
        transform: Transform,
    ) -> Result<(), ValidationError> {
        let result = (|| {
            batch.validate()?;
            crate::display_list::mapped_bounds(batch.bounds(), transform, self.path)?;
            self.push(DrawCommand::ParticleInstances { batch, transform })
        })();
        if let Err(error) = &result {
            self.error = Some(error.clone());
        }
        result
    }

    /// Records a shared font-unit/path with its local transform.
    pub fn fill_path(
        &mut self,
        geometry: Arc<Path2D>,
        transform: Transform,
        paint: impl Into<Paint>,
        rule: FillRule,
    ) -> Result<(), ValidationError> {
        let result = geometry
            .validate()
            .and_then(|_| {
                crate::display_list::mapped_bounds(geometry.bounding_box(), transform, self.path)
            })
            .and_then(|_| {
                self.push(DrawCommand::Fill {
                    path: geometry,
                    transform,
                    paint: paint.into(),
                    rule,
                })
            });
        if let Err(error) = &result {
            self.error = Some(error.clone());
        }
        result
    }

    /// Creates a local recorder borrowing the caller's structural budget and diagnostic path.
    /// Commands and path verbs are charged at `depth` before they enter the result.
    pub fn new(state: &'a mut ValidationState, path: &'a str, depth: usize) -> Self {
        Self {
            commands: Vec::new(),
            state,
            path,
            depth,
            error: None,
        }
    }

    /// Records a filled rectangle without narrowing geometry or color.
    pub fn fill_rect(
        &mut self,
        bounds: Rect,
        paint: impl Into<Paint>,
    ) -> Result<(), ValidationError> {
        self.push(DrawCommand::FillRect {
            bounds,
            transform: Transform::IDENTITY,
            paint: paint.into(),
        })
    }

    /// Fills geometry, retaining its explicit contour rule.
    ///
    /// Each path verb also consumes one shared work-budget unit at command
    /// depth. Rectangles retain the existing constant-cost fill fast path.
    pub fn fill(
        &mut self,
        shape: &impl Shape,
        paint: impl Into<Paint>,
        rule: FillRule,
    ) -> Result<(), ValidationError> {
        if let Some(error) = &self.error {
            return Err(error.clone());
        }
        if let Err(error) = shape.validate() {
            self.error = Some(error.clone());
            return Err(error);
        }
        if let Err(mut error) = validate_rect(shape.bounding_box(), "", "bounds") {
            error.path = format!("{}.commands[{}]", self.path, self.commands.len());
            self.error = Some(error.clone());
            return Err(error);
        }
        let paint = paint.into();
        if let Some(bounds) = shape.as_rect() {
            return self.fill_rect(bounds, paint);
        }
        self.push(DrawCommand::Fill {
            path: Arc::new(shape.path()),
            transform: Transform::IDENTITY,
            paint,
            rule,
        })
    }

    /// Expands a bounded line/curve stroke into a NonZero fill.
    ///
    /// Source verbs and generated outline verbs charge the shared structural
    /// budget independently. Invalid parameters are checked even at zero width.
    /// Errors poison the recording even if the caller ignores the Result.
    pub fn stroke(
        &mut self,
        shape: &impl Shape,
        style: &StrokeStyle,
        paint: impl Into<Paint>,
    ) -> Result<(), ValidationError> {
        if let Some(error) = &self.error {
            return Err(error.clone());
        }
        let path = format!("{}.commands[{}].stroke", self.path, self.commands.len());
        let result = (|| {
            style.validate(&path)?;
            shape.validate()?;
            validate_rect(shape.bounding_box(), &path, "bounds")?;
            let geometry = shape.path();
            for index in 0..geometry.verbs().len() {
                self.state
                    .enter_with(|| format!("{path}.source.verbs[{index}]"), self.depth)?;
            }
            let outline = style.outline(&geometry, &path)?;
            if outline.verbs().is_empty() {
                return Ok(());
            }
            self.push(DrawCommand::Fill {
                path: Arc::new(outline),
                transform: Transform::IDENTITY,
                paint: paint.into(),
                rule: FillRule::NonZero,
            })
        })();
        if let Err(error) = &result {
            self.error = Some(error.clone());
        }
        result
    }

    fn push(&mut self, command: DrawCommand) -> Result<(), ValidationError> {
        if let Some(error) = &self.error {
            return Err(error.clone());
        }
        let index = self.commands.len();
        let path = || format!("{}.commands[{index}]", self.path);
        let (bounds, verbs) = match &command {
            DrawCommand::ParticleInstances { batch, .. } => (batch.bounds(), batch.len()),
            DrawCommand::FillRect { bounds, .. } => (*bounds, 0),
            DrawCommand::Fill { path, .. } => (path.bounding_box(), path.verbs().len()),
            DrawCommand::PushFilterLayer { .. }
            | DrawCommand::PushLayer { .. }
            | DrawCommand::PushRectLayer { .. }
            | DrawCommand::PushShapeLayer { .. }
            | DrawCommand::PopLayer
            | DrawCommand::BeginMask { .. }
            | DrawCommand::EndMask => {
                let error = ValidationError::invalid(
                    path(),
                    "commands",
                    "Canvas records local fills, not compiled layer boundaries",
                );
                self.error = Some(error.clone());
                return Err(error);
            }
        };
        if let DrawCommand::FillRect { paint, .. } | DrawCommand::Fill { paint, .. } = &command
            && let Err(mut error) = paint.validate()
        {
            error.path = path();
            self.error = Some(error.clone());
            return Err(error);
        }
        let result = validate_rect(bounds, "", "bounds")
            .map_err(|mut error| {
                error.path = path();
                error
            })
            .and_then(|()| self.state.enter_with(path, self.depth))
            .and_then(|()| {
                for verb in 0..verbs {
                    self.state
                        .enter_with(|| format!("{}.verbs[{verb}]", path()), self.depth)?;
                }
                Ok(())
            });
        if let Err(error) = result {
            self.error = Some(error.clone());
            return Err(error);
        }
        self.commands.push(command);
        Ok(())
    }

    /// Invokes one drawable and returns its complete validated command sequence.
    /// The first recorder failure wins even when ignored by the callback; no partial
    /// commands escape on error. Callback panics propagate to the caller.
    pub fn record(
        mut self,
        drawable: &dyn Drawable,
        context: &DrawContext,
    ) -> Result<Vec<DrawCommand>, ValidationError> {
        if let Some(resolved) = drawable.resolve_node(context)? {
            for command in resolved.record_local(context, crate::ValidationLimits::default())? {
                self.push(command)?;
            }
            return self.finish();
        }
        let path = self.path;
        let result = drawable.draw(context, &mut self);
        let commands = self.finish()?;
        result.map_err(|mut error| {
            error.path = if error.path.is_empty() {
                format!("{path}.draw")
            } else {
                format!("{path}.draw.{}", error.path)
            };
            error
        })?;
        Ok(commands)
    }

    fn finish(self) -> Result<Vec<DrawCommand>, ValidationError> {
        self.error.map_or(Ok(self.commands), Err)
    }
}
