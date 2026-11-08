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

/// Time-independent drawing inputs for the frontend.
///
/// Device scale and an optional immutable font snapshot are shared by bounds
/// measurement and recording; neither implicitly discovers resources.
#[derive(Clone, Debug)]
pub struct DrawContext {
    quality: RenderQuality,
    time: crate::Time,
    pub(crate) device_scale: f64,

    fonts: Option<crate::asset::environment::FontEnvironment>,
    assets: crate::Assets,

    limits: crate::ValidationLimits,
    layout_cache: Option<std::sync::Arc<crate::SceneCache>>,
    render_cache: Option<crate::RenderCache>,

    diagnostics: crate::RequestDiagnostics,
    font_usage: crate::FontUsage,
}

impl Default for DrawContext {
    fn default() -> Self {
        Self {
            quality: RenderQuality::Final,
            time: crate::Time::ZERO,
            device_scale: 1.0,
            fonts: None,
            assets: crate::Assets::default(),
            limits: crate::ValidationLimits::default(),
            layout_cache: None,
            render_cache: None,
            diagnostics: crate::RequestDiagnostics::default(),
            font_usage: crate::FontUsage::default(),
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
            quality: RenderQuality::Final,
            time: crate::Time::ZERO,
            device_scale,
            fonts: None,
            assets: crate::Assets::default(),
            limits: crate::ValidationLimits::default(),
            layout_cache: None,
            render_cache: None,
            diagnostics: crate::RequestDiagnostics::default(),
            font_usage: crate::FontUsage::default(),
        })
    }

    /// Selects an explicit approximation policy for deferred content recording.
    pub fn quality(mut self, quality: RenderQuality) -> Self {
        self.quality = quality;
        self
    }

    /// Production quality is the default for every standalone drawing context.
    pub fn render_quality(&self) -> RenderQuality {
        self.quality
    }

    /// Supplies scene-local signed time to deferred content effects.
    pub fn with_time(mut self, time: crate::Time) -> Self {
        self.time = time;
        self
    }

    /// Scene-local time; direct static recording defaults to zero.
    pub fn time(&self) -> crate::Time {
        self.time
    }

    /// Device pixels per logical unit; does not implicitly modify local geometry.
    pub fn device_scale(&self) -> f64 {
        self.device_scale
    }

    /// Supplies a shared explicit font snapshot to paragraph bounds and drawing.
    pub fn with_fonts(
        mut self,
        fonts: impl Into<crate::asset::environment::FontEnvironment>,
    ) -> Self {
        self.fonts = Some(fonts.into());
        self
    }

    /// Font environment for paragraph text, optional for geometry-only drawing.
    pub fn fonts(&self) -> Option<&crate::asset::environment::FontEnvironment> {
        self.fonts.as_ref()
    }

    /// Captures the same lazy/preloaded owner used by the enclosing session.
    pub fn with_assets(mut self, assets: &crate::Assets) -> Self {
        self.assets = assets.clone().with_diagnostics(self.diagnostics.clone());
        if let Some(fonts) = assets.fonts() {
            self.fonts = Some(fonts.clone());
        }
        self
    }

    /// Explicit resource owner; an unconfigured context does not resolve relative files.
    pub fn assets(&self) -> &crate::Assets {
        &self.assets
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
    ///     DisplayList, RasterLimits, ResourcePool, ValidatedDisplayList, MAX_PATH_VERBS,
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
        validate_rect(drawable.try_bounds(self)?, "drawable", "bounds")?;
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

    /// Supplies content-layout LRU storage shared by one render session.
    pub fn with_layout_cache(mut self, cache: std::sync::Arc<crate::SceneCache>) -> Self {
        self.layout_cache = Some(cache);
        self
    }

    /// Optional session layout cache, separate from active-frame budgets.
    pub fn layout_cache(&self) -> Option<&std::sync::Arc<crate::SceneCache>> {
        self.layout_cache.as_ref()
    }

    /// Supplies partitioned pure caches shared by rendering workers.
    pub fn render_cache(mut self, cache: crate::RenderCache) -> Self {
        self.layout_cache = Some(cache.partition(crate::CachePartition::Paragraph).clone());
        self.render_cache = Some(cache);
        self
    }

    /// Shared cache partitions, absent for standalone uncached drawing.
    pub fn configured_render_cache(&self) -> Option<&crate::RenderCache> {
        self.render_cache.as_ref()
    }

    /// Caches immutable layout by full typed key and declared logical payload.
    pub fn cache_layout<
        Key: Eq + std::hash::Hash + Send + Sync + 'static,
        Value: Send + Sync + 'static,
    >(
        &self,
        key: Key,
        build: impl FnOnce() -> Value,
        cost: impl FnOnce(&Value) -> usize,
    ) -> std::sync::Arc<Value> {
        match &self.layout_cache {
            Some(cache) => cache.get_with_cost(key, build, cost),
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

    /// Carries request diagnostics explicitly through deferred content compilation.
    pub fn with_diagnostics(mut self, diagnostics: crate::RequestDiagnostics) -> Self {
        self.diagnostics = diagnostics;
        self.assets = self.assets.with_diagnostics(self.diagnostics.clone());
        self
    }

    /// Request-owned sink; cloning this drawing context preserves its identity.
    pub fn diagnostics(&self) -> &crate::RequestDiagnostics {
        &self.diagnostics
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
///     DisplayList, RasterLimits, ResourcePool, ValidatedDisplayList, MAX_PATH_VERBS,
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
    /// Resolves deferred content to ordinary nodes at the current drawing time.
    fn resolve_node(&self, _context: &DrawContext) -> Result<Option<crate::Node>, ValidationError> {
        Ok(None)
    }

    /// Local layout/ink bounds before node placement.
    fn bounds(&self, context: &DrawContext) -> Rect;

    /// Checked bounds for fallible content such as explicit-font paragraphs.
    /// Existing geometry implementations may keep the infallible bounds method.
    fn try_bounds(&self, context: &DrawContext) -> Result<Rect, ValidationError> {
        Ok(self.bounds(context))
    }

    /// Real horizontal text baseline in local coordinates; geometry has none.
    fn baseline(&self, _context: &DrawContext) -> Option<f64> {
        None
    }

    /// Emits local commands; validation errors propagate without partial output.
    fn draw(&self, context: &DrawContext, canvas: &mut Canvas<'_>) -> Result<(), ValidationError>;
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
        let context = DrawContext::new(0.5).unwrap().quality(RenderQuality::Draft);
        assert_eq!(context.clone().render_quality(), RenderQuality::Draft);
        assert_eq!(context.device_scale(), 0.5);
    }
}
