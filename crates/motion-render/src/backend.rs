//! Observable native backend selection and startup parity contract.

/// A session's fixed rendering backend. Device loss after this decision is a
/// failure, not a mid-export switch to a different pixel implementation.
#[derive(Clone, Debug)]
pub struct BackendDecision {
    /// Actual renderer used for this session.
    pub renderer: crate::Renderer,
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
///
/// The default is the GPU when feature `gpu` is enabled and the CPU reference otherwise.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Renderer {
    /// Bounded CPU reference rasterizer; does not require a window or GPU adapter.
    Cpu,
    /// Native wgpu composition using an available Metal, Vulkan or DX12 adapter.
    /// Requires feature `gpu`; no fallback to the CPU renderer is performed.
    Gpu,
}

impl Default for Renderer {
    fn default() -> Self {
        BackendPreference::default().into_renderer()
    }
}

/// Requested native backend; the GPU never silently falls back to the CPU.
///
/// This is the only way to choose a backend. [`Renderer`] is the outcome
/// reported by [`BackendDecision`]. The default is the GPU on hardware adapters
/// when feature `gpu` is enabled and the CPU reference otherwise.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum BackendPreference {
    /// Bounded CPU reference, selected explicitly.
    Cpu,
    /// Native GPU; setup, self-check and unsupported content are errors.
    Gpu {
        /// Admits software adapters such as llvmpipe or WARP.
        allow_software: bool,
    },
}

impl Default for BackendPreference {
    fn default() -> Self {
        if cfg!(feature = "gpu") {
            Self::gpu()
        } else {
            Self::Cpu
        }
    }
}

impl BackendPreference {
    /// Explicit GPU on hardware adapters only.
    pub const fn gpu() -> Self {
        Self::Gpu {
            allow_software: false,
        }
    }

    const fn into_renderer(self) -> Renderer {
        match self {
            Self::Cpu => Renderer::Cpu,
            Self::Gpu { .. } => Renderer::Gpu,
        }
    }

    /// Whether software GPU adapters are admitted; always false for CPU.
    pub const fn allow_software(self) -> bool {
        match self {
            Self::Cpu => false,
            Self::Gpu { allow_software } => allow_software,
        }
    }

    /// Sets the software adapter policy; CPU has none and is returned unchanged.
    pub const fn with_software(self, allow_software: bool) -> Self {
        match self {
            Self::Cpu => Self::Cpu,
            Self::Gpu { .. } => Self::Gpu { allow_software },
        }
    }

    /// Whether the GPU is requested.
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
    fn gpu_precision_rejection_is_an_error_not_a_cpu_retry() -> motion_core::Result<()> {
        use motion_core::NodeModifiers;
        use motion_core::Shape;
        let mut session =
            crate::Composition::new(4, 4, crate::FrameClock::new(motion_core::fps!(30), 150))
                .with_options(
                    crate::RenderOptions::default().backend(crate::BackendPreference::gpu()),
                )?
                .frame_renderer(motion_core::Size::new(4.0, 4.0))?;
        let error = session
            .render_frame(0, |_| {
                motion_core::Node::from(
                    motion_core::Rect::new(0.0, 0.0, 4.0, 4.0).fill(motion_core::Color::WHITE),
                )
                .opacity(1e-50)
            })
            .unwrap_err();
        assert!(error.to_string().contains("precision"), "{error}");
        assert!(
            session
                .backend_decision()
                .is_none_or(|decision| decision.renderer == crate::Renderer::Gpu)
        );
        Ok(())
    }

    #[test]
    fn default_backend_is_gpu_only_when_the_feature_is_enabled() {
        assert_eq!(
            crate::BackendPreference::default().wants_gpu(),
            cfg!(feature = "gpu")
        );
        assert_eq!(
            crate::Renderer::default() == crate::Renderer::Gpu,
            cfg!(feature = "gpu")
        );
    }
}
