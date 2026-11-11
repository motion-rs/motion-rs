//! Shared drawing inputs and drawable contracts.

use super::*;

/// Explicit preview approximation policy. Final preserves production rendering.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub enum RenderQuality {
    /// Interactive preview; content may use bounded lower-detail approximations.
    Draft,
    /// Full-quality production rendering and export.
    #[default]
    Final,
}

/// Drawing inputs for the frontend.
///
/// Device scale and structural limits are drawing-specific; time, quality,
/// assets, fonts, diagnostics and compute caches live in the shared
/// [`Evaluation`]. Neither implicitly discovers resources.
#[derive(Clone, Debug)]
pub struct DrawContext {
    pub(crate) device_scale: f64,
    limits: crate::ValidationLimits,
    evaluation: std::sync::Arc<Evaluation>,
    font_usage: crate::FontUsage,
    mark_invalid_geometry: bool,
}

impl Default for DrawContext {
    fn default() -> Self {
        Self {
            device_scale: 1.0,
            limits: crate::ValidationLimits::default(),
            evaluation: std::sync::Arc::default(),
            font_usage: crate::FontUsage::default(),
            mark_invalid_geometry: false,
        }
    }
}

impl DrawContext {
    /// Creates drawing inputs with a finite, strictly positive device scale.
    pub fn new(device_scale: f64) -> Result<Self, ValidationError> {
        if !device_scale.is_finite() || device_scale <= 0.0 {
            return Err(ValidationError::invalid(
                "drawing",
                "device_scale",
                "Require a finite, positive device scale",
            ));
        }
        Ok(Self {
            device_scale,
            ..Self::default()
        })
    }

    /// Replaces the shared evaluation environment.
    pub fn with_evaluation(mut self, evaluation: std::sync::Arc<Evaluation>) -> Self {
        self.evaluation = evaluation;
        self
    }

    /// Shared time, quality, resources, diagnostics and caches.
    pub fn evaluation(&self) -> &std::sync::Arc<Evaluation> {
        &self.evaluation
    }

    fn edit(mut self, edit: impl FnOnce(Evaluation) -> Evaluation) -> Self {
        let current = std::sync::Arc::try_unwrap(self.evaluation)
            .unwrap_or_else(|shared| Evaluation::clone(&shared));
        self.evaluation = std::sync::Arc::new(edit(current));
        self
    }

    /// Selects an explicit approximation policy for deferred content recording.
    pub fn with_quality(self, quality: RenderQuality) -> Self {
        self.edit(|evaluation| evaluation.with_quality(quality))
    }

    /// Production quality is the default for every standalone drawing context.
    pub fn render_quality(&self) -> RenderQuality {
        self.evaluation.quality()
    }

    /// Supplies scene-local signed time to deferred content effects.
    pub fn with_time(self, time: crate::Time) -> Self {
        self.edit(|evaluation| evaluation.with_time(time))
    }

    /// Scene-local time; direct static recording defaults to zero.
    pub fn time(&self) -> crate::Time {
        self.evaluation.time()
    }

    /// Device pixels per logical unit; does not implicitly modify local geometry.
    pub fn device_scale(&self) -> f64 {
        self.device_scale
    }

    /// Supplies a shared explicit font snapshot to paragraph bounds and drawing.
    pub fn with_fonts(self, fonts: impl Into<crate::asset::environment::FontEnvironment>) -> Self {
        self.edit(|evaluation| evaluation.with_fonts(fonts))
    }

    /// Font environment for paragraph text, optional for geometry-only drawing.
    pub fn fonts(&self) -> Option<&crate::asset::environment::FontEnvironment> {
        self.evaluation.fonts()
    }

    /// Captures the same lazy/preloaded owner used by the enclosing session.
    pub fn with_assets(self, assets: &crate::Assets) -> Self {
        self.edit(|evaluation| evaluation.with_assets(assets))
    }

    /// Explicit resource owner; an unconfigured context does not resolve relative files.
    pub fn assets(&self) -> &crate::Assets {
        self.evaluation.assets()
    }

    /// Records a drawable's local commands without rendering or color quantization.
    ///
    /// Limits charge the drawable at depth one and every command at depth two.
    /// Callback errors and ignored canvas errors both fail the recording.
    /// This local buffer is not a scene DisplayList; use [`crate::Node::compile`]
    /// for layout, global transforms and isolated subtree opacity.
    ///
    /// ```
    /// use motion_core::prelude::*;
    /// use motion_core::{
    ///     DisplayList, RasterLimits, ResourceStore, ValidatedDisplayList, MAX_PATH_VERBS,
    ///     MAX_STROKE_VERBS, MAX_SVG_PATH_BYTES, sample_offsets,
    /// };
    /// let bounds = Rect::new(0.1, 0.2, 10.3, 20.4);
    /// let color = Color::from_rgb_float(1.25, -0.125, 0.333);
    /// let commands =
    ///     DrawContext::new(2.0)?.record(&bounds.fill(color), ValidationLimits::default())?;
    /// assert_eq!(
    ///     commands,
    ///     vec![DrawCommand::FillRect {
    ///         bounds,
    ///         transform: Transform::IDENTITY,
    ///         paint: Paint::Solid(color)
    ///     }]
    /// );
    /// # Ok::<(), motion_core::ValidationError>(())
    /// ```
    pub fn record(
        &self,
        drawable: &dyn Drawable,
        limits: ValidationLimits,
    ) -> Result<Vec<DrawCommand>, ValidationError> {
        let mut state = ValidationState::new(limits)?;
        state.enter("drawable", 1)?;
        validate_rect(drawable.bounds(self)?, "drawable", "bounds")?;
        Canvas::new(&mut state, "drawable", 2).record(drawable, self)
    }

    /// Supplies the enclosing structural policy to deferred layout callbacks.
    pub fn with_validation_limits(mut self, limits: crate::ValidationLimits) -> Self {
        self.limits = limits;
        self
    }

    /// Structural policy supplied by the enclosing measurement or compilation.
    pub fn validation_limits(&self) -> crate::ValidationLimits {
        self.limits
    }

    /// Supplies partitioned compute caches shared by rendering workers.
    pub fn with_render_cache(self, cache: crate::RenderCache) -> Self {
        self.edit(|evaluation| evaluation.with_cache(cache))
    }

    /// Shared cache partitions, absent for standalone uncached drawing.
    pub fn render_cache(&self) -> Option<&crate::RenderCache> {
        self.evaluation.cache()
    }

    /// Caches immutable layout in the paragraph partition by full typed key and
    /// declared logical payload; without a render cache the value is rebuilt.
    pub fn cache_layout<
        Key: Eq + std::hash::Hash + Send + Sync + 'static,
        Value: Send + Sync + 'static,
    >(
        &self,
        key: Key,
        build: impl FnOnce() -> Value,
        cost: impl FnOnce(&Value) -> usize,
    ) -> std::sync::Arc<Value> {
        match self.render_cache() {
            Some(cache) => cache
                .partition(crate::CachePartition::Paragraph)
                .get_with_cost(key, build, cost),
            None => std::sync::Arc::new(build()),
        }
    }

    /// Carries an export-owned font observation sink into every recorded glyph.
    pub fn with_font_usage(mut self, usage: crate::FontUsage) -> Self {
        self.font_usage = usage;
        self
    }

    /// Font sources actually painted through this context.
    pub fn font_usage(&self) -> &crate::FontUsage {
        &self.font_usage
    }

    /// Draws leaves with invalid geometry as a visible error marker instead of failing
    /// compilation. Intended for preview; export keeps the strict default.
    pub fn with_invalid_geometry_marker(mut self, mark: bool) -> Self {
        self.mark_invalid_geometry = mark;
        self
    }

    /// Whether invalid leaf geometry is drawn as an error marker.
    pub fn marks_invalid_geometry(&self) -> bool {
        self.mark_invalid_geometry
    }

    /// Carries request diagnostics explicitly through deferred content compilation.
    pub fn with_diagnostics(self, diagnostics: crate::RequestDiagnostics) -> Self {
        self.edit(|evaluation| evaluation.with_diagnostics(diagnostics))
    }

    /// Request-owned sink; cloning this drawing context preserves its identity.
    pub fn diagnostics(&self) -> &crate::RequestDiagnostics {
        self.evaluation.diagnostics()
    }
}

/// Content whose bounds and drawing share the same logical coordinates.
///
/// Implementations must be deterministic and time independent for fixed inputs.
/// Bounds are supplied by content. Custom content
/// must enclose its own drawing; the frontend validates bounds and commands but
/// cannot prove arbitrary callbacks return conservative bounds.
///
/// ```
/// use motion_core::prelude::*;
/// use motion_core::{
///     DisplayList, RasterLimits, ResourceStore, ValidatedDisplayList, MAX_PATH_VERBS,
///     MAX_STROKE_VERBS, MAX_SVG_PATH_BYTES, sample_offsets,
/// };
/// let row = Row::new()
///     .gap(4.0)
///     .children([
///         Rect::new(0.0, 0.0, 10.0, 20.0).fill(Color::from_hex(0x24a49b)),
///         Rect::new(0.0, 0.0, 5.0, 10.0).fill(Color::WHITE),
///     ])
///     .at(Point::new(100.0, 80.0));
/// assert_eq!(
///     row.measure(ValidationLimits::default())?,
///     Rect::new(90.5, 70.0, 109.5, 90.0)
/// );
/// let list = row.compile(
///     Size::new(200.0, 160.0),
///     PixelSize::new(200, 160),
///     ValidationLimits::default(),
/// )?;
/// assert_eq!(list.commands.len(), 2);
/// # Ok::<(), motion_core::ValidationError>(())
/// ```
pub trait Drawable: Send + Sync {
    /// Local layout/ink bounds before node placement; invalid content is an error.
    fn bounds(&self, context: &DrawContext) -> Result<Rect, ValidationError>;

    /// Real horizontal text baseline in local coordinates; geometry has none.
    fn baseline(&self, _context: &DrawContext) -> Option<f64> {
        None
    }

    /// Emits local commands; validation errors propagate without partial output.
    fn draw(&self, context: &DrawContext, canvas: &mut Canvas<'_>) -> Result<(), ValidationError>;
}

/// Content that expands into ordinary nodes at the current drawing time.
///
/// Deferred content is not a leaf: it owns no bounds or commands. Measuring and
/// compiling resolve it first, so the outer placement and inherited scene time
/// apply to the resolved subtree.
pub trait DeferredContent: Send + Sync {
    /// Resolves this content to an ordinary node at the drawing time in `context`.
    fn resolve(&self, context: &DrawContext) -> Result<crate::Node, ValidationError>;

    /// Real horizontal text baseline of the resolved node in local coordinates.
    fn baseline(&self, _context: &DrawContext) -> Option<f64> {
        None
    }
}

#[cfg(test)]
mod quality_tests {
    use super::*;

    #[test]
    fn quality_is_explicit_cloned_and_final_by_default() {
        assert_eq!(
            DrawContext::default().render_quality(),
            RenderQuality::Final
        );
        let context = DrawContext::new(0.5)
            .unwrap()
            .with_quality(RenderQuality::Draft);
        assert_eq!(context.clone().render_quality(), RenderQuality::Draft);
        assert_eq!(context.device_scale(), 0.5);
    }
}
