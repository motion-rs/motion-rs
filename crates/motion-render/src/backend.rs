//! Observable native backend selection and startup parity contract.

use motion_core::Result;

/// A session's fixed rendering backend. Device loss after this decision is a
/// failure, not a mid-export switch to a different pixel implementation.
#[derive(Clone, Debug)]
pub struct BackendDecision {
    /// Actual renderer used for this session.
    pub renderer: crate::Renderer,
    /// GPU setup/self-check explanation when Automatic selected CPU instead.
    pub reason: Option<String>,
}

#[cfg(feature = "gpu")]
pub(crate) fn software_adapter(name: &str, device_type: &str) -> bool {
    let name = name.to_ascii_lowercase();
    device_type.eq_ignore_ascii_case("cpu")
        || [
            "llvmpipe",
            "lavapipe",
            "swiftshader",
            "warp",
            "software",
            "basic render",
        ]
        .iter()
        .any(|label| name.contains(label))
}

/// Offscreen composition backend, independent of the Studio window renderer.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Renderer {
    /// Default CPU rasterizer; does not require a window or GPU adapter.
    #[default]
    Cpu,
    /// Native wgpu composition using an available Metal, Vulkan or DX12 adapter.
    /// Requires feature `gpu`; no fallback to the CPU renderer is performed.
    Gpu,
}

/// Native backend selection; explicit GPU never silently falls back.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum BackendPreference {
    /// Bounded CPU reference.
    #[default]
    Cpu,
    /// Explicit native GPU, without fallback.
    Gpu,
    /// Validates a native GPU once, falling back only before the session starts.
    Automatic,
}

impl BackendPreference {
    /// Returns the initial session renderer, without probing device availability.
    /// Automatic starts with CPU; session policy decides whether a GPU is admissible.
    pub fn renderer(self) -> Result<crate::Renderer> {
        match self {
            Self::Cpu => Ok(crate::Renderer::Cpu),
            Self::Gpu => Ok(crate::Renderer::Gpu),
            Self::Automatic => Ok(crate::Renderer::Cpu),
        }
    }
}

#[cfg(test)]
mod tests {
    #[test]
    #[cfg(feature = "gpu")]
    fn software_adapter_policy_has_no_device_side_effects() {
        for name in [
            "llvmpipe (LLVM)",
            "Microsoft WARP",
            "SwiftShader Device",
            "Microsoft Basic Render Driver",
            "lavapipe",
        ] {
            assert!(super::software_adapter(name, "Other"));
        }
        assert!(super::software_adapter("Unknown", "Cpu"));
        for name in ["Apple M3", "NVIDIA RTX", "AMD Radeon", "Intel Iris Xe"] {
            assert!(!super::software_adapter(name, "IntegratedGpu"));
        }
    }

    #[test]
    #[cfg(feature = "gpu")]
    fn automatic_precision_fallback_precedes_device_initialization() -> motion_core::Result<()> {
        use motion_core::NodeModifiers;
        let mut session = crate::Composition::new(4, 4)
            .backend_preference(crate::BackendPreference::Automatic, false)
            .frame_renderer(motion_core::Size::new(4.0, 4.0))?;
        session.render_frame(0, |_| {
            motion_core::Node::from(
                motion_core::Rect::new(0.0, 0.0, 4.0, 4.0).fill(motion_core::Color::WHITE),
            )
            .opacity(1e-50)
        })?;
        assert!(session.gpu_info().is_none());
        let decision = session.backend_decision().unwrap();
        assert_eq!(decision.renderer, crate::Renderer::Cpu);
        assert!(
            decision
                .reason
                .as_deref()
                .is_some_and(|reason| reason.contains("precision"))
        );
        Ok(())
    }

    #[test]
    #[cfg(not(feature = "gpu"))]
    fn automatic_without_feature_falls_back_before_rendering() -> motion_core::Result<()> {
        let mut session = crate::Composition::new(4, 4)
            .backend_preference(crate::BackendPreference::Automatic, false)
            .frame_renderer(motion_core::Size::new(4.0, 4.0))?;
        session.render_frame(0, |_| motion_core::Node::empty())?;
        let decision = session.backend_decision().unwrap();
        assert_eq!(decision.renderer, crate::Renderer::Cpu);
        assert!(
            decision
                .reason
                .as_deref()
                .is_some_and(|reason| reason.contains("feature gpu"))
        );
        Ok(())
    }
}
