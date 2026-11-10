//! Observable native backend selection and startup parity contract.

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

/// Requested native backend; explicit GPU never silently falls back.
///
/// This is the only way to choose a backend. [`Renderer`] is the outcome
/// reported by [`BackendDecision`].
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum BackendPreference {
    /// Bounded CPU reference.
    #[default]
    Cpu,
    /// Explicit native GPU, without fallback.
    Gpu {
        /// Admits software adapters such as llvmpipe or WARP.
        allow_software: bool,
    },
    /// Validates a native GPU once, falling back only before the session starts.
    Automatic {
        /// Admits software adapters such as llvmpipe or WARP.
        allow_software: bool,
    },
}

impl BackendPreference {
    /// Explicit GPU on hardware adapters only.
    pub const fn gpu() -> Self {
        Self::Gpu {
            allow_software: false,
        }
    }

    /// GPU on hardware adapters with a startup fallback to CPU.
    pub const fn automatic() -> Self {
        Self::Automatic {
            allow_software: false,
        }
    }

    /// Whether software GPU adapters are admitted; always false for CPU.
    pub const fn allow_software(self) -> bool {
        match self {
            Self::Cpu => false,
            Self::Gpu { allow_software } | Self::Automatic { allow_software } => allow_software,
        }
    }

    /// Sets the software adapter policy; CPU has none and is returned unchanged.
    pub const fn with_software(self, allow_software: bool) -> Self {
        match self {
            Self::Cpu => Self::Cpu,
            Self::Gpu { .. } => Self::Gpu { allow_software },
            Self::Automatic { .. } => Self::Automatic { allow_software },
        }
    }

    /// Whether a GPU may be used, strictly or after a startup check.
    pub const fn wants_gpu(self) -> bool {
        !matches!(self, Self::Cpu)
    }
}

impl From<Renderer> for BackendPreference {
    /// Requests exactly `renderer`; GPU is strict and hardware-only.
    fn from(renderer: Renderer) -> Self {
        match renderer {
            Renderer::Cpu => Self::Cpu,
            Renderer::Gpu => Self::gpu(),
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
        let mut session = crate::Composition::new(4, 4, crate::FrameClock::new(30, 150))
            .with_options(
                crate::RenderOptions::default().backend(crate::BackendPreference::automatic()),
            )?
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
        let mut session = crate::Composition::new(4, 4, crate::FrameClock::new(30, 150))
            .with_options(
                crate::RenderOptions::default().backend(crate::BackendPreference::automatic()),
            )?
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
