//! GPU-specific preparation results, independent of public CPU prepared frames.

use super::device::DeviceGeometry;
use crate::prepared::PreparedScene;
use crate::raster::frame_ir::{FrameIr, GeometrySource, IrPaint, IrPath};
use crate::raster::paint::TableCache;
use crate::raster::prepare::{SceneBackend, prepare_geometry, prepare_scene};
use crate::raster::{PreparationCache, RasterScope};
use motion_core::{RasterLimits, ValidationError, ValidationLimits};
use std::mem::size_of;

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
        cache: Option<&mut PreparationCache>,
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

    pub(crate) fn quality(mut self, quality: motion_core::RenderQuality) -> Self {
        self.scene.quality = quality;
        self
    }

    pub(crate) fn blend_space(mut self, space: motion_core::drawing::BlendSpace) -> Self {
        self.scene.blend_space = space;
        self
    }

    pub(crate) fn has_device_coverage(&self) -> bool {
        !self.geometry.device_fills.is_empty()
    }
}

impl std::ops::Deref for GpuFrame {
    type Target = PreparedScene;

    fn deref(&self) -> &Self::Target {
        &self.scene
    }
}

/// Preparation that keeps resources for upload and emits device coverage when admitted.
struct DeviceBackend<'a> {
    geometry: &'a mut DeviceGeometry,
    cache: Option<&'a mut PreparationCache>,
}

impl SceneBackend for DeviceBackend<'_> {
    fn fixed_bytes(&self) -> u64 {
        size_of::<DeviceGeometry>() as u64
    }

    fn kept_resource_bytes(&self, ir: &FrameIr<'_>) -> u64 {
        ir.resource_bytes()
    }

    fn paint_cache(&mut self) -> Option<&mut TableCache> {
        self.cache.as_deref_mut().map(|cache| &mut cache.paints)
    }

    fn particles(&mut self, analytic_fallback: bool) {
        self.geometry.analytic_cpu_fallbacks += u64::from(analytic_fallback);
    }

    fn fill(
        &mut self,
        frame: &mut PreparedScene,
        geometry: &IrPath<'_>,
        paint: &IrPaint<'_>,
        tolerance: f64,
        scope: &mut RasterScope<'_>,
    ) -> Result<(), ValidationError> {
        // Rectangles and decal patterns keep analytic strips: decals reject samples in f64 against
        // CPU coverage. Every other fill is device coverage or an error.
        let decal = matches!(paint.source, motion_core::Paint::Pattern(pattern) if pattern.decal);
        if decal || !matches!(geometry.source, GeometrySource::Path(_)) {
            let cache = self.cache.as_deref_mut().map(|cache| &mut cache.coverage);
            return prepare_geometry(frame, geometry, tolerance, cache, scope);
        }
        self.geometry.device_fill(frame, geometry, tolerance, scope)
    }
}

/// GPU preparation has its own output and never constructs a CPU prepared frame.
pub(crate) fn prepare_validated_device(
    source: &motion_core::ValidatedDisplayList,
    validation: ValidationLimits,
    limits: RasterLimits,
    tolerance: f64,
    cache: Option<&mut PreparationCache>,
) -> Result<GpuFrame, ValidationError> {
    let ir = FrameIr::build(source, validation, limits, tolerance)?;
    GpuFrame::prepare(&ir, limits, cache)
}
