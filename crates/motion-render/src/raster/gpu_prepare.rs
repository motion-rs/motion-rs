//! GPU-specific preparation results, independent of public CPU prepared frames.

use super::CoverageCache;
use super::device::DeviceGeometry;
use super::frame_ir::FrameIr;
use super::prepare::{DeviceBackend, prepare_scene};
#[cfg(test)]
use crate::prepared::PreparedFrame;
use crate::prepared::PreparedScene;
use motion_core::{RasterLimits, ValidationError};

/// GPU scene records and independently generated device geometry.
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct GpuFrame {
    pub(crate) scene: PreparedScene,
    pub(crate) geometry: DeviceGeometry,
}

impl GpuFrame {
    pub(crate) fn prepare(
        ir: &FrameIr<'_>,
        limits: RasterLimits,
        cache: Option<&mut CoverageCache>,
    ) -> Result<Self, ValidationError> {
        let mut geometry = DeviceGeometry::default();
        let scene = prepare_scene(
            ir,
            limits,
            &mut DeviceBackend {
                geometry: &mut geometry,
                cache,
            },
        )?;
        Ok(Self { scene, geometry })
    }

    #[cfg(feature = "gpu")]
    pub(crate) fn quality(mut self, quality: motion_core::RenderQuality) -> Self {
        self.scene.quality = quality;
        self
    }

    #[cfg(feature = "gpu")]
    pub(crate) fn blend_space(mut self, space: motion_core::drawing::BlendSpace) -> Self {
        self.scene.blend_space = space;
        self
    }

    pub(crate) fn has_device_coverage(&self) -> bool {
        !self.geometry.device_fills.is_empty()
    }

    /// Explicit compatibility conversion, never needed by GPU submission.
    #[cfg(test)]
    pub(crate) fn into_cpu(
        mut self,
        limits: RasterLimits,
    ) -> Result<PreparedFrame, ValidationError> {
        self.geometry
            .materialize_device_coverage(&mut self.scene, limits)?;
        Ok(PreparedFrame { scene: self.scene })
    }

    #[cfg(test)]
    pub(crate) fn materialize_device_coverage(
        &mut self,
        limits: RasterLimits,
    ) -> Result<(), ValidationError> {
        self.geometry
            .materialize_device_coverage(&mut self.scene, limits)
    }

    #[cfg(test)]
    pub(crate) fn compose_linear(
        &self,
        limits: RasterLimits,
    ) -> Result<crate::cpu::LinearFrame, ValidationError> {
        self.clone().into_cpu(limits)?.scene.compose_linear(limits)
    }
}

impl std::ops::Deref for GpuFrame {
    type Target = PreparedScene;

    fn deref(&self) -> &Self::Target {
        &self.scene
    }
}
