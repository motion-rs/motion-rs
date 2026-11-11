//! CPU preparation APIs and shared consumption of normalized frame semantics.

use super::device::DeviceGeometry;
use super::frame_ir::{COMMAND_PATH_BYTES, FrameIr, GeometrySource, IrClip, IrItem};
#[cfg(feature = "gpu")]
use super::gpu_prepare::GpuFrame;
use super::{CoverageCache, RasterScope, rect_clip};
use crate::prepared::{
    PreparationStats, PreparedFilters, PreparedFrame, PreparedItem, PreparedLayer, PreparedScene,
    RasterLimits,
};
use motion_core::budget::Budget;
use motion_core::{DisplayList, SceneLimits, ValidationError, ValidationLimits};
use std::mem::size_of;

mod coverage;
pub(in crate::raster) mod filters;
mod layers;

pub(crate) use coverage::{paint_record, pixel_bounds};

/// Bounded CPU coverage preparation for editable or validated core commands.
///
/// The complete source preflights before backend-neutral normalization, even for
/// invisible and offscreen content. Returned frames always contain CPU coverage;
/// preparing one never selects a device.
pub trait RenderPrepare {
    /// Prepares CPU coverage using the default 1/256 device-pixel tolerance.
    fn prepare(
        &self,
        validation: ValidationLimits,
        limits: RasterLimits,
    ) -> Result<PreparedFrame, ValidationError>;

    /// Prepares CPU coverage with combined structural and raster allowances.
    fn prepare_within(&self, limits: SceneLimits) -> Result<PreparedFrame, ValidationError> {
        self.prepare(limits.structure, limits.raster)
    }

    /// Prepares CPU coverage using a finite positive device-pixel curve tolerance.
    ///
    /// Curves retain f64 control points until bounded Wang subdivision. Linear
    /// contours close implicitly; four-row active-edge scheduling and analytic
    /// area integration round coverage once to u8. Visible cancelling projections
    /// retain the 1e-7 device-pixel precision guard. No supersampling occurs.
    fn prepare_with_tolerance(
        &self,
        validation: ValidationLimits,
        limits: RasterLimits,
        tolerance: f64,
    ) -> Result<PreparedFrame, ValidationError>;
}

impl RenderPrepare for DisplayList {
    /// Prepares bounded CPU coverage at 1/256 device-pixel tolerance.
    ///
    /// ```
    /// use motion_render::prelude::*;
    /// use motion_core::prelude::*;
    /// use motion_core::{
    ///     DisplayList, RasterLimits, ResourceStore, ValidatedDisplayList, MAX_PATH_VERBS,
    ///     MAX_STROKE_VERBS, MAX_SVG_PATH_BYTES, sample_offsets,
    /// };
    /// use motion_render::{Composition, RenderPrepare, RenderOptions, Renderer, render_still};
    /// let node = Node::group([
    ///     Node::from(Rect::new(0.0, 0.0, 4.0, 4.0).fill(Color::WHITE)),
    ///     Node::from(Rect::new(2.0, 0.0, 6.0, 4.0).fill(Color::WHITE)),
    /// ])
    /// .opacity(0.5);
    /// let list = node.compile(
    ///     Size::new(8.0, 4.0),
    ///     PixelSize::new(8, 4),
    ///     ValidationLimits::default(),
    /// )?;
    /// let prepared = list.prepare(ValidationLimits::default(), RasterLimits::default())?;
    /// assert_eq!(prepared.size(), PixelSize::new(8, 4));
    /// let output = prepared.render_linear(RasterLimits::default())?;
    /// assert_eq!(output.linear_pixel(3, 0), Some([0.5; 4]));
    /// assert_eq!(output.linear_pixel(6, 0), Some([0.0; 4]));
    /// # Ok::<(), motion_core::ValidationError>(())
    /// ```
    fn prepare(
        &self,
        validation: ValidationLimits,
        limits: RasterLimits,
    ) -> Result<PreparedFrame, ValidationError> {
        self.prepare_with_tolerance(validation, limits, 1.0 / 256.0)
    }

    /// Prepares bounded curves with an explicit positive device-pixel tolerance.
    ///
    /// ```
    /// use motion_render::prelude::*;
    /// use motion_core::prelude::*;
    /// use motion_core::{
    ///     DisplayList, RasterLimits, ResourceStore, ValidatedDisplayList, MAX_PATH_VERBS,
    ///     MAX_STROKE_VERBS, MAX_SVG_PATH_BYTES, sample_offsets,
    /// };
    /// use motion_render::{Composition, RenderPrepare, RenderOptions, Renderer, render_still};
    /// let path = Path2D::builder()
    ///     .move_to(Point::ORIGIN)
    ///     .quadratic_to(Point::new(0.5, 2.0), Point::new(1.0, 0.0))
    ///     .build()?;
    /// let list = Node::from(path.fill(Color::WHITE)).compile(
    ///     Size::new(1.0, 1.0),
    ///     PixelSize::new(1, 1),
    ///     ValidationLimits::default(),
    /// )?;
    /// let frame = list.prepare_with_tolerance(
    ///     ValidationLimits::default(), RasterLimits::default(), 1.0 / 1024.0,
    /// )?;
    /// assert_eq!(
    ///     frame.render_linear(RasterLimits::default())?.linear_pixel(0, 0),
    ///     Some([170.0 / 255.0; 4])
    /// );
    /// # Ok::<(), motion_core::ValidationError>(())
    /// ```
    fn prepare_with_tolerance(
        &self,
        validation: ValidationLimits,
        limits: RasterLimits,
        tolerance: f64,
    ) -> Result<PreparedFrame, ValidationError> {
        prepare_with_cache(self, validation, limits, tolerance, None)
    }
}

pub(crate) fn prepare_with_cache(
    source: &DisplayList,
    validation: ValidationLimits,
    limits: RasterLimits,
    tolerance: f64,
    cache: Option<&mut CoverageCache>,
) -> Result<PreparedFrame, ValidationError> {
    let source = source.clone().validate(validation)?;
    prepare_validated_with_cache(&source, validation, limits, tolerance, cache)
}

impl RenderPrepare for motion_core::ValidatedDisplayList {
    fn prepare(
        &self,
        validation: ValidationLimits,
        limits: RasterLimits,
    ) -> Result<PreparedFrame, ValidationError> {
        prepare_validated_with_cache(self, validation, limits, 1.0 / 256.0, None)
    }

    fn prepare_with_tolerance(
        &self,
        validation: ValidationLimits,
        limits: RasterLimits,
        tolerance: f64,
    ) -> Result<PreparedFrame, ValidationError> {
        prepare_validated_with_cache(self, validation, limits, tolerance, None)
    }
}

pub(crate) fn prepare_validated_with_cache(
    source: &motion_core::ValidatedDisplayList,
    validation: ValidationLimits,
    limits: RasterLimits,
    tolerance: f64,
    cache: Option<&mut CoverageCache>,
) -> Result<PreparedFrame, ValidationError> {
    let ir = FrameIr::build(source, validation, limits, tolerance)?;
    prepare_ir(&ir, limits, cache)
}

/// GPU preparation has its own output and never constructs a CPU prepared frame.
#[cfg(feature = "gpu")]
pub(crate) fn prepare_validated_device(
    source: &motion_core::ValidatedDisplayList,
    validation: ValidationLimits,
    limits: RasterLimits,
    tolerance: f64,
    cache: Option<&mut CoverageCache>,
    device: bool,
) -> Result<GpuFrame, ValidationError> {
    let ir = FrameIr::build(source, validation, limits, tolerance)?;
    GpuFrame::prepare(&ir, limits, cache, device)
}

pub(crate) fn prepare_ir(
    ir: &FrameIr<'_>,
    limits: RasterLimits,
    cache: Option<&mut CoverageCache>,
) -> Result<PreparedFrame, ValidationError> {
    Ok(PreparedFrame {
        scene: prepare_scene(ir, limits, cache, None)?,
    })
}

/// A single scope walk shares semantics, not backend preparation results.
pub(crate) fn prepare_scene(
    ir: &FrameIr<'_>,
    limits: RasterLimits,
    mut cache: Option<&mut CoverageCache>,
    mut device: Option<&mut DeviceGeometry>,
) -> Result<PreparedScene, ValidationError> {
    let mut budget = Budget::new(ir.size(), limits)?;
    let ir_stats = ir.stats();
    budget.bytes(ir_stats.retained_bytes, "frame_ir")?;
    budget.work(ir_stats.work, "frame_ir")?;
    budget.merge_peak(ir_stats.peak_bytes);
    budget.bytes(size_of::<PreparedFrame>() as u64, "prepared")?;
    if device.is_some() {
        budget.bytes(size_of::<DeviceGeometry>() as u64, "prepared.gpu")?;
    }
    let mut frame = PreparedScene {
        quality: motion_core::RenderQuality::Final,
        blend_space: motion_core::drawing::BlendSpace::LinearRgb,
        size: ir.size(),
        output_bounds: ir.output_bounds(),
        filter_halo: ir.filter_halo(),
        background: ir.background(),
        items: Vec::new(),
        strips: Vec::new(),
        spans: Vec::new(),
        alphas: Vec::new(),
        paints: Vec::new(),
        paint_sources: Vec::new(),
        clips: Vec::new(),
        stats: PreparationStats::default(),
    };
    let mut rectangles = rect_clip::Preparation::default();
    let mut has_plus_layers = false;
    for (index, item) in ir.items().iter().enumerate() {
        budget.bytes(COMMAND_PATH_BYTES, "prepared.commands")?;
        let path = format!("display_list.commands[{index}]");
        budget.work(1, &path)?;
        let item = match item {
            IrItem::ParticleInstances { batch, transform } => {
                if batch.analytic_fallback().is_some()
                    && let Some(device) = device.as_deref_mut()
                {
                    device.analytic_cpu_fallbacks += 1;
                }
                if transform.inverse().is_none() {
                    budget.release(COMMAND_PATH_BYTES);
                    continue;
                }
                // Resources remain shared; the live IR charge already admits this payload.
                PreparedItem::ParticleInstances {
                    batch: batch.clone(),
                    transform: *transform,
                }
            }
            IrItem::PushLayer {
                layer,
                alpha_clip,
                rectangle_clip,
                shape_clip,
            } => {
                if layer.filters.is_some() {
                    rectangles.begin_mask();
                }
                let mut layer = copy_layer(layer, &mut budget, &path)?;
                if layer.filters.is_none() {
                    layer.alpha_clip = alpha_clip
                        .map(|index| match ir.clip(index) {
                            IrClip::Alpha {
                                source,
                                path: geometry,
                            } => frame.prepare_ir_clip(
                                source,
                                ir.path(*geometry),
                                ir.tolerance(),
                                &mut budget,
                                &path,
                            ),
                            _ => Err(ValidationError::invalid(
                                &path,
                                "clip",
                                "Expected alpha clip",
                            )),
                        })
                        .transpose()?;
                    let rectangle = rectangle_clip
                        .map(|index| match ir.clip(index) {
                            IrClip::Rectangle {
                                source,
                                path: geometry,
                            } => {
                                debug_assert!(ir.path(*geometry).segment_upper_bound > 0);
                                Ok(source)
                            }
                            _ => Err(ValidationError::invalid(
                                &path,
                                "clip",
                                "Expected rectangle clip",
                            )),
                        })
                        .transpose()?;
                    let shape = shape_clip
                        .map(|index| match ir.clip(index) {
                            IrClip::Shape {
                                source,
                                path: geometry,
                            } => {
                                debug_assert!(ir.path(*geometry).segment_upper_bound > 0);
                                Ok(source)
                            }
                            _ => Err(ValidationError::invalid(
                                &path,
                                "clip",
                                "Expected shape clip",
                            )),
                        })
                        .transpose()?;
                    let geometric = rectangles.push(
                        rectangle,
                        shape,
                        &mut frame,
                        ir.tolerance(),
                        &mut budget,
                        &path,
                    )?;
                    layer.geometric_clip =
                        geometric.filter(|_| shape.is_some() || rectangle.is_some());
                }
                has_plus_layers |= layer.filters.is_none()
                    && matches!(
                        layer.blend_mode,
                        motion_core::BlendMode::Plus | motion_core::BlendMode::LinearPlus
                    );
                PreparedItem::PushLayer { layer }
            }
            IrItem::PopLayer => {
                rectangles.pop();
                PreparedItem::PopLayer
            }
            IrItem::BeginMask { mode } => {
                rectangles.begin_mask();
                PreparedItem::BeginMask { mode: *mode }
            }
            IrItem::EndMask => {
                rectangles.pop();
                PreparedItem::EndMask
            }
            IrItem::Fill {
                path: geometry,
                paint,
            } => {
                let geometry = ir.path(*geometry);
                let paint = ir.paint(*paint);
                let strip_start = frame.strips.len();
                let span_start = frame.spans.len();
                let mut scope = RasterScope::new(&mut budget, &path);
                let device_fill = if let Some(device) = device.as_deref_mut() {
                    if !matches!(geometry.source, GeometrySource::Path(_)) {
                        false
                    } else if !device.coverage_enabled {
                        device.fallback_paths.coverage_disabled += 1;
                        false
                    } else if matches!(paint.source, motion_core::Paint::Pattern(pattern) if pattern.decal)
                    {
                        device.fallback_paths.unsupported_paint += 1;
                        false
                    } else {
                        device.device_fill(&mut frame, geometry, ir.tolerance(), &mut scope)?
                    }
                } else {
                    false
                };
                if !device_fill {
                    prepare_geometry(
                        &mut frame,
                        geometry,
                        ir.tolerance(),
                        cache.as_deref_mut(),
                        &mut scope,
                    )?;
                }
                let paint = frame.add_paint(
                    paint.source,
                    paint.bounds,
                    paint.transform,
                    &mut budget,
                    &path,
                    cache.as_deref_mut().map(|cache| &mut cache.paints),
                )?;
                PreparedItem::Fill {
                    strips: strip_start..frame.strips.len(),
                    spans: span_start..frame.spans.len(),
                    paint,
                }
            }
        };
        budget.push(&mut frame.items, item, &path)?;
        budget.release(COMMAND_PATH_BYTES);
    }
    rectangles.release(&mut budget)?;
    if has_plus_layers {
        layers::trim_plus_layers(&mut frame, &mut budget)?;
    }
    // The IR dies after preparation. Only a GPU frame keeps its admitted resource bytes for
    // upload; a CPU frame borrows the immutable resources and owns no copy of them.
    let kept = if device.is_some() {
        ir.resource_bytes()
    } else {
        0
    };
    budget.release(ir_stats.retained_bytes - kept);
    frame.stats = PreparationStats {
        retained_bytes: budget.live_bytes(),
        peak_bytes: budget.peak_bytes(),
        work: budget.work_done(),
    };
    Ok(frame)
}

fn copy_layer(
    layer: &PreparedLayer,
    budget: &mut Budget,
    path: &str,
) -> Result<PreparedLayer, ValidationError> {
    let filters = if let Some(filters) = &layer.filters {
        for filter in &filters.operations {
            budget.bytes(
                size_of::<motion_core::Filter>() as u64 + filter.heap_bytes(),
                path,
            )?;
        }
        Some(PreparedFilters {
            source_bounds: filters.source_bounds,
            operations: filters::copy_filters(&filters.operations, path)?,
        })
    } else {
        None
    };
    Ok(PreparedLayer {
        opacity: layer.opacity,
        bounds: layer.bounds,
        blend_mode: layer.blend_mode,
        alpha_clip: None,
        geometric_clip: None,
        filters,
    })
}

fn prepare_geometry(
    frame: &mut PreparedScene,
    geometry: &super::frame_ir::IrPath<'_>,
    tolerance: f64,
    cache: Option<&mut CoverageCache>,
    scope: &mut RasterScope<'_>,
) -> Result<(), ValidationError> {
    let transform = geometry.transform;
    match geometry.source {
        GeometrySource::Rectangle(bounds) => {
            let [xx, yx, xy, yy, _, _] = transform.coefficients();
            if (xy == 0.0 && yx == 0.0) || (xx == 0.0 && yy == 0.0) {
                frame.rectangle(geometry.bounds, scope.budget, scope.path)
            } else {
                frame.polygon(
                    super::frame_ir::rectangle_verbs(bounds).into_iter(),
                    transform,
                    geometry.rule,
                    tolerance,
                    scope.budget,
                    scope.path,
                )
            }
        }
        GeometrySource::Path(source) => {
            if let Some(cache) = cache {
                cache.polygon(frame, source, transform, geometry.rule, tolerance, scope)
            } else {
                frame.polygon_path(
                    source,
                    transform,
                    geometry.rule,
                    tolerance,
                    scope.budget,
                    scope.path,
                )
            }
        }
    }
}
