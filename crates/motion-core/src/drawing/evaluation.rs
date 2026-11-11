//! Immutable evaluation environment shared by scene and drawing contexts.

use std::sync::Arc;

use super::RenderQuality;

/// Time, quality and explicit resources for one evaluation.
///
/// `SceneContext` and `DrawContext` build on the same environment so deferred
/// content sees identical time, quality, assets, fonts, diagnostics and compute
/// caches whichever one measures or records it. The environment is immutable
/// once shared; builders take `self` and drawing contexts hold it by [`Arc`].
#[derive(Clone, Debug)]
pub struct Evaluation {
    time: crate::Time,
    quality: RenderQuality,
    assets: crate::Assets,
    fonts: Option<crate::asset::environment::FontEnvironment>,
    diagnostics: crate::RequestDiagnostics,
    cache: Option<crate::RenderCache>,
}

impl Default for Evaluation {
    fn default() -> Self {
        Self {
            time: crate::Time::ZERO,
            quality: RenderQuality::Final,
            assets: crate::Assets::default(),
            fonts: None,
            diagnostics: crate::RequestDiagnostics::default(),
            cache: None,
        }
    }
}

impl Evaluation {
    /// Creates a Final-quality environment at time zero without assets or caches.
    pub fn new() -> Self {
        Self::default()
    }

    /// Selects an explicit approximation policy for deferred content.
    pub fn with_quality(mut self, quality: RenderQuality) -> Self {
        self.quality = quality;
        self
    }

    /// Production quality is the default.
    pub fn quality(&self) -> RenderQuality {
        self.quality
    }

    /// Supplies scene-local signed time to deferred content effects.
    pub fn with_time(mut self, time: crate::Time) -> Self {
        self.time = time;
        self
    }

    /// Scene-local time; defaults to zero.
    pub fn time(&self) -> crate::Time {
        self.time
    }

    /// Captures the same lazy/preloaded owner used by the enclosing session.
    ///
    /// The assets inherit this environment's diagnostics, and their font
    /// environment replaces any previously supplied one.
    pub fn with_assets(mut self, assets: &crate::Assets) -> Self {
        self.assets = assets.clone().with_diagnostics(self.diagnostics.clone());
        if let Some(fonts) = assets.fonts() {
            self.fonts = Some(fonts.clone());
        }
        self
    }

    /// Explicit resource owner; an unconfigured environment resolves no relative files.
    pub fn assets(&self) -> &crate::Assets {
        &self.assets
    }

    /// Supplies a shared explicit font snapshot.
    pub fn with_fonts(
        mut self,
        fonts: impl Into<crate::asset::environment::FontEnvironment>,
    ) -> Self {
        self.fonts = Some(fonts.into());
        self
    }

    /// Font environment for paragraph text, optional for geometry-only content.
    pub fn fonts(&self) -> Option<&crate::asset::environment::FontEnvironment> {
        self.fonts.as_ref()
    }

    /// Carries request diagnostics explicitly through deferred content compilation.
    pub fn with_diagnostics(mut self, diagnostics: crate::RequestDiagnostics) -> Self {
        self.diagnostics = diagnostics;
        self.assets = self.assets.with_diagnostics(self.diagnostics.clone());
        self
    }

    /// Request-owned sink; cloning the environment preserves its identity.
    pub fn diagnostics(&self) -> &crate::RequestDiagnostics {
        &self.diagnostics
    }

    /// Supplies partitioned compute caches shared by rendering workers.
    pub fn with_cache(mut self, cache: crate::RenderCache) -> Self {
        self.cache = Some(cache);
        self
    }

    /// Shared compute-cache partitions, absent for standalone uncached evaluation.
    pub fn cache(&self) -> Option<&crate::RenderCache> {
        self.cache.as_ref()
    }

    /// Shares this environment between contexts without copying it.
    pub fn shared(self) -> Arc<Self> {
        Arc::new(self)
    }
}
