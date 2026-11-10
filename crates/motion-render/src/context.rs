//! Explicit ownership of assets and structural policy.
#[cfg(test)]
use motion_content::AssetsExt;

use std::sync::Arc;

use crate::Renderer;

use motion_core::{Assets, Node, SceneStats, ValidationError, ValidationLimits};

#[cfg(test)]
use motion_core::ValidationErrorKind;

pub use motion_core::FontIdentity;

/// graphics API actually selected by wgpu, not a compiled capability.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum GpuBackend {
    /// Apple's Metal API.
    Metal,
    /// Vulkan, normally used on Linux and optionally Windows.
    Vulkan,
    /// Direct3D 12, available on Windows.
    Dx12,
}

/// Adapter metadata captured after successful GPU initialization.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct GpuInfo {
    /// Actual graphics API.
    pub backend: GpuBackend,
    /// Adapter's reported name.
    pub name: String,
    /// Vendor identifier reported by the adapter.
    pub vendor: u32,
    /// Device identifier reported by the adapter.
    pub device: u32,
    /// Adapter classification, including `Cpu` for a software graphics driver.
    pub device_type: String,
    /// Driver name, which may be empty on some platforms.
    pub driver: String,
    /// Additional driver information, which may be empty.
    pub driver_info: String,
}

/// Compiled platform capabilities, not a probe of a live adapter or device.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct RenderCapabilities {
    /// Headless CPU composition is compiled in.
    pub software: bool,
    /// Native wgpu composition is compiled in; adapter initialization can still fail.
    pub gpu: bool,
    /// macOS Metal composition is compiled in; initialization can still fail.
    pub metal: bool,
}

impl RenderCapabilities {
    /// Queries feature/platform support without loading fonts or opening a device.
    pub fn compiled() -> Self {
        Self {
            software: true,
            gpu: cfg!(all(feature = "gpu", not(target_arch = "wasm32"))),
            metal: cfg!(all(feature = "gpu", target_os = "macos")),
        }
    }

    /// Whether the selected composition backend is compiled for this platform.
    pub fn supports(self, renderer: Renderer) -> bool {
        match renderer {
            Renderer::Cpu => self.software,
            Renderer::Gpu => self.gpu,
        }
    }
}

/// Reusable resource ownership, separate from a composition's dimensions and time.
///
/// Construction never initializes a GPU. Clones share immutable asset snapshots;
/// modifying the asset registry is copy-on-write. Session devices, targets/caches
/// and explicitly supplied font environments have separate ownership.
#[derive(Clone)]
pub struct RenderContext {
    assets: Arc<Assets>,
    limits: ValidationLimits,
}

impl Default for RenderContext {
    fn default() -> Self {
        Self {
            assets: Arc::new(Assets::default()),
            limits: ValidationLimits::default(),
        }
    }
}

impl RenderContext {
    /// Creates an empty, lazy context using default scene and asset budgets.
    pub fn new() -> Self {
        Self::default()
    }

    /// Attaches unified lazy and preloaded resources without decoding or upload.
    pub fn with_assets(mut self, assets: Assets) -> Self {
        self.assets = Arc::new(assets);
        self
    }

    /// Unified resource owner attached to this context.
    pub fn assets(&self) -> &Assets {
        &self.assets
    }

    /// Isolates future registrations from existing context snapshots.
    pub fn assets_mut(&mut self) -> &mut Assets {
        self.assets = Arc::new(self.assets.snapshot());
        Arc::get_mut(&mut self.assets).expect("new resource snapshot")
    }

    /// Replaces structural allowances after validating their bounded ranges.
    pub fn with_validation_limits(
        mut self,
        limits: ValidationLimits,
    ) -> Result<Self, ValidationError> {
        motion_core::validation::ValidationState::new(limits)?;
        self.limits = limits;
        Ok(self)
    }

    /// Structural allowances used by rendering and context preflight.
    pub fn validation_limits(&self) -> ValidationLimits {
        self.limits
    }

    /// Feature/platform support; live device creation is deferred until rendering.
    pub fn capabilities(&self) -> RenderCapabilities {
        RenderCapabilities::compiled()
    }

    /// Checks structure and bounds with a default, font-free DrawContext.
    /// Does not record drawing, probe live devices or admit raster budgets.
    /// Use font-aware Node preflight when text needs an explicit environment.
    pub fn preflight(&self, scene: &Node) -> Result<SceneStats, ValidationError> {
        scene.preflight_with(&motion_core::DrawContext::default(), self.limits)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use motion_core::Shape;

    #[test]
    fn compiled_gpu_policy_is_distinct_from_live_adapter_metadata() {
        let capabilities = RenderCapabilities::compiled();
        assert!(capabilities.supports(Renderer::Cpu));
        assert_eq!(
            capabilities.supports(Renderer::Gpu),
            cfg!(all(feature = "gpu", not(target_arch = "wasm32")))
        );
        assert_eq!(
            capabilities.metal,
            cfg!(all(feature = "gpu", target_os = "macos"))
        );
    }

    #[cfg(not(feature = "gpu"))]
    #[test]
    fn disabled_gpu_fails_before_loading_invalid_font_data() {
        let error = crate::Composition::new(32, 32, crate::FrameClock::new(30, 150))
            .with_options(crate::RenderOptions::default().backend(Renderer::Gpu.into()))?
            .frame_renderer(motion_core::Size::new(32.0, 32.0))
            .err()
            .expect("GPU is not compiled");
        assert_eq!(
            error.downcast_ref::<ValidationError>().unwrap().kind,
            ValidationErrorKind::Unsupported
        );
    }

    #[test]
    fn composition_preflight_preserves_context_policy_without_loading_fonts() -> motion_core::Result
    {
        let context = RenderContext::new().with_validation_limits(ValidationLimits {
            max_nodes: 1,
            max_depth: 1,
        })?;
        let composition = crate::Composition::new(24, 24, crate::FrameClock::new(30, 150))
            .with_options(crate::RenderOptions::default().context(context))?;
        let cloned = composition.clone();
        assert_eq!(cloned.context().validation_limits().max_nodes, 1);
        let size = motion_core::Size::new(24.0, 24.0);
        assert_eq!(cloned.preflight_frame(2, size, |_| Node::empty())?.nodes, 1);
        let error = cloned
            .preflight_frame(2, size, |_| {
                Node::group([Node::group([Node::from(
                    motion_core::Rect::new(0.0, 0.0, 1.0, 1.0).fill(motion_core::Color::WHITE),
                )])])
            })
            .unwrap_err();
        let error = error.downcast_ref::<ValidationError>().unwrap();
        assert_eq!(error.kind, ValidationErrorKind::StructuralLimit);
        assert_eq!(error.frame, Some(2));
        Ok(())
    }

    #[test]
    fn contexts_share_assets_until_mutated_and_preflight_without_a_device() -> motion_core::Result {
        let mut context = RenderContext::new().with_assets(Assets::new(".")?);
        context.assets_mut().preload(
            "pixel",
            Arc::new(motion_content::ImageAsset::from_rgba8(1, 1, vec![255; 4])?),
        )?;
        let cloned = context.clone();
        context.assets_mut().preload(
            "new",
            Arc::new(motion_content::ImageAsset::from_rgba8(1, 1, vec![0; 4])?),
        )?;
        assert!(cloned.assets().image("pixel").get().is_ok());
        assert_eq!(
            cloned.assets().image("new").status(),
            motion_core::AssetStatus::Unloaded
        );
        assert_eq!(
            context.assets().image("new").status(),
            motion_core::AssetStatus::Ready
        );
        assert_eq!(context.preflight(&Node::empty())?.nodes, 1);
        assert!(context.capabilities().supports(Renderer::Cpu));
        assert!(
            context
                .with_validation_limits(ValidationLimits {
                    max_depth: 129,
                    ..ValidationLimits::default()
                })
                .is_err()
        );
        Ok(())
    }

    #[test]
    fn frame_diagnostics_and_context_policy_survive_cache_clear() -> motion_core::Result {
        use motion_core::NodeModifiers;
        let context = RenderContext::new().with_validation_limits(ValidationLimits {
            max_nodes: 1,
            max_depth: 1,
        })?;
        let composition = crate::Composition::new(24, 24, crate::FrameClock::new(30, 150))
            .with_options(crate::RenderOptions::default().context(context.clone()))?;
        let mut renderer = composition.frame_renderer(motion_core::Size::new(24.0, 24.0))?;
        let error = renderer
            .render_frame(7, |_| {
                Node::from(
                    motion_core::Rect::new(0.0, 0.0, 8.0, 8.0).fill(motion_core::Color::WHITE),
                )
                .filter(motion_core::Filter::Blur { sigma: f64::NAN })
            })
            .err()
            .unwrap();
        let diagnostic = error
            .downcast_ref::<ValidationError>()
            .expect("typed scene error");
        assert_eq!(diagnostic.frame, Some(7));
        assert!(diagnostic.path.contains("node.filters[0]"));
        assert_eq!(diagnostic.field, "filters");
        renderer.cache().clear();
        assert_eq!(composition.context().validation_limits().max_nodes, 1);
        let error = renderer
            .render_frame(0, |_| {
                Node::group([Node::group([Node::from(
                    motion_core::Rect::new(0.0, 0.0, 1.0, 1.0).fill(motion_core::Color::WHITE),
                )])])
            })
            .err()
            .unwrap();
        assert_eq!(
            error.downcast_ref::<ValidationError>().unwrap().kind,
            ValidationErrorKind::StructuralLimit
        );
        Ok(())
    }
}
