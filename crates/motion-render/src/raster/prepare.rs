//! Device-space coverage preparation and filter bounds.
//!
//! This module turns display lists into prepared frames; filter padding lives in `filters`
//! and rectangle, strip and clip coverage emission in `coverage`.

use motion_core::{
    DisplayList, DrawCommand, PixelSize, SceneLimits, ValidationError, ValidationLimits,
};

use super::{CoverageCache, RasterScope, rect_clip};
use crate::prepared::*;
use filters::{command_filter_bytes, copy_filters, filter_padding, filter_support, shift_command};
use motion_core::budget::Budget;
use std::mem::size_of;

mod coverage;
mod filters;

pub(crate) use coverage::{paint_record, pixel_bounds};

/// Bounded raster preparation for editable or validated core commands.
/// Editable lists are fully validated first; validated snapshots reuse measured
/// requirements when the requested structural limits allow them. Prepared paints
/// reference immutable pixels and gradient tables shared by both renderers.
pub trait RenderPrepare {
    /// Prepares coverage using the default device-pixel tolerance.
    fn prepare(
        &self,
        validation: ValidationLimits,
        limits: RasterLimits,
    ) -> Result<PreparedFrame, ValidationError>;

    /// Prepares coverage with combined structural and raster allowances.
    fn prepare_within(&self, limits: SceneLimits) -> Result<PreparedFrame, ValidationError> {
        self.prepare(limits.structure, limits.raster)
    }

    /// Prepares coverage using an explicit positive device-pixel tolerance.
    fn prepare_with_tolerance(
        &self,
        validation: ValidationLimits,
        limits: RasterLimits,
        tolerance: f64,
    ) -> Result<PreparedFrame, ValidationError>;
}

impl RenderPrepare for DisplayList {
    /// Prepares bounded coverage with 1/256 device-pixel curve tolerance.
    ///
    /// The full source list preflights first, even when invisible/offscreen.
    /// Independent raster limits admit output dimensions, live logical bytes
    /// and work. Linear contours are implicitly closed and split at crossings
    /// before integrating filled trapezoids; coverage is rounded once to u8.
    /// Four-row active-edge scheduling excludes unrelated edges and skips empty
    /// bands while preserving original edge order within the shared integrator.
    /// Filled intervals are integrated analytically using shared scalar geometry;
    /// this is not multisample rasterization or a separate backend tessellator.
    /// Severely cancelling visible edge projections fail a conservative 1e-7
    /// device-pixel precision guard rather than silently losing subpixel shape.
    /// No supersampling or f32 geometry conversion occurs.
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

    /// Prepares coverage with a finite positive device-pixel Bezier tolerance.
    ///
    /// Control points transform before conservative Wang-based uniform subdivision.
    /// The tolerance bounds chord geometry, not pixel-area error or topology below
    /// that scale. Both fill rules use the shared linear edges. Generated work and
    /// live storage are admitted against raster limits; a conservative arithmetic
    /// guard rejects coordinates that cannot resolve the requested tolerance.
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
    ///     ValidationLimits::default(),
    ///     RasterLimits::default(),
    ///     1.0 / 1024.0,
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
    mut cache: Option<&mut CoverageCache>,
) -> Result<PreparedFrame, ValidationError> {
    source.preflight(validation)?;
    let source = source.display_list();
    let (mut frame, mut budget, padding) = initialize_frame(source, limits, tolerance)?;
    let mut filter_chain = 0_u32;
    let mut filter_stack = Vec::new();
    let mut has_plus_layers = false;
    let mut rectangles = rect_clip::Preparation::default();
    for (index, command) in source.commands.iter().enumerate() {
        let path = format!("display_list.commands[{index}]");
        budget.work(1, &path)?;
        let shifted;
        let temporary = if padding == 0 {
            0
        } else {
            command_filter_bytes(command)
        };
        budget.bytes(temporary, &path)?;
        let command = if padding == 0 {
            command
        } else {
            shifted = shift_command(command, f64::from(padding), &path)?;
            &shifted
        };
        let Some(item) = prepare_command(
            &mut frame,
            command,
            tolerance,
            cache.as_deref_mut(),
            &mut rectangles,
            &mut RasterScope::new(&mut budget, &path),
        )?
        else {
            continue;
        };
        match &item {
            PreparedItem::PushLayer { layer } => {
                has_plus_layers |= layer.filters.is_none()
                    && matches!(
                        layer.blend_mode,
                        motion_core::BlendMode::Plus | motion_core::BlendMode::LinearPlus
                    );
                budget.push(&mut filter_stack, filter_chain, &path)?;
                if let Some(filters) = &layer.filters {
                    filter_chain =
                        filter_chain.saturating_add(filter_support(&filters.operations)?);
                    frame.filter_halo = frame.filter_halo.max(filter_chain);
                }
            }
            PreparedItem::BeginMask { .. } => {
                budget.push(&mut filter_stack, filter_chain, &path)?;
            }
            PreparedItem::PopLayer | PreparedItem::EndMask => {
                filter_chain = filter_stack.pop().expect("validated filter support stack");
                budget.release(size_of::<u32>() as u64);
            }
            _ => {}
        }
        budget.push(&mut frame.items, item, &path)?;
        budget.release(temporary);
    }
    rectangles.release(&mut budget)?;
    if has_plus_layers {
        trim_plus_layers(&mut frame, &mut budget)?;
    }
    frame.stats = PreparationStats {
        retained_bytes: budget.live_bytes(),
        peak_bytes: budget.peak_bytes(),
        work: budget.work_done(),
    };
    Ok(frame)
}

fn initialize_frame(
    source: &DisplayList,
    limits: RasterLimits,
    tolerance: f64,
) -> Result<(PreparedFrame, Budget, u32), ValidationError> {
    if !tolerance.is_finite() || tolerance <= 0.0 {
        return Err(ValidationError::invalid(
            "prepared",
            "tolerance",
            "Curve tolerance must be finite and positive",
        ));
    }
    let padding = filter_padding(&source.commands)?;
    let dimension = |value: u32| value.checked_add(padding.checked_mul(2)?);
    let size = PixelSize::new(
        dimension(source.output_size.width).ok_or_else(|| {
            ValidationError::limit(
                "prepared.filters",
                "max_pixels",
                "Filter raster width overflow",
            )
        })?,
        dimension(source.output_size.height).ok_or_else(|| {
            ValidationError::limit(
                "prepared.filters",
                "max_pixels",
                "Filter raster height overflow",
            )
        })?,
    );
    let mut budget = Budget::new(size, limits)?;
    budget.bytes(size_of::<PreparedFrame>() as u64, "prepared")?;
    let frame = PreparedFrame {
        quality: motion_core::RenderQuality::Final,
        blend_space: motion_core::drawing::BlendSpace::LinearRgb,
        size,
        output_bounds: PixelRect {
            left: padding,
            top: padding,
            right: padding + source.output_size.width,
            bottom: padding + source.output_size.height,
        },
        filter_halo: 0,
        background: source
            .background
            .map(|color| paint_record(color, "prepared.background"))
            .transpose()?,
        items: Vec::new(),
        strips: Vec::new(),
        spans: Vec::new(),
        alphas: Vec::new(),
        paints: Vec::new(),
        paint_sources: Vec::new(),
        clips: Vec::new(),
        stats: PreparationStats::default(),
    };

    Ok((frame, budget, padding))
}

fn prepare_command(
    frame: &mut PreparedFrame,
    command: &DrawCommand,
    tolerance: f64,
    mut cache: Option<&mut CoverageCache>,
    rectangles: &mut rect_clip::Preparation,
    scope: &mut RasterScope<'_>,
) -> Result<Option<PreparedItem>, ValidationError> {
    let size = frame.size;
    let budget = &mut *scope.budget;
    let path = scope.path;
    let item = match command {
        DrawCommand::ParticleInstances { batch, transform } => {
            if transform.inverse().is_none() {
                return Ok(None);
            }
            budget.bytes(batch.bytes(), path)?;
            PreparedItem::ParticleInstances {
                batch: batch.clone(),
                transform: *transform,
            }
        }
        DrawCommand::PushFilterLayer { bounds, filters } => {
            rectangles.begin_mask();
            let mut capture = *bounds;
            let mut current = *bounds;
            for filter in filters {
                budget.work(filter.structural_cost() as u64, path)?;
                let (output, intermediate) = filter.extent(current, path)?;
                capture = motion_core::filter::union(capture, intermediate);
                current = output;
                budget.bytes(
                    size_of::<motion_core::Filter>() as u64 + filter.heap_bytes(),
                    path,
                )?;
            }
            PreparedItem::PushLayer {
                layer: PreparedLayer {
                    bounds: pixel_bounds(capture, size),
                    opacity: 1.0,
                    blend_mode: motion_core::BlendMode::SourceOver,
                    alpha_clip: None,
                    geometric_clip: None,
                    filters: Some(PreparedFilters {
                        source_bounds: *bounds,
                        operations: copy_filters(filters, path)?,
                    }),
                },
            }
        }
        DrawCommand::PushLayer { .. }
        | DrawCommand::PushRectLayer { .. }
        | DrawCommand::PushShapeLayer { .. } => {
            prepare_layer(frame, command, tolerance, rectangles, scope)?
        }
        DrawCommand::PopLayer => {
            rectangles.pop();
            PreparedItem::PopLayer
        }
        DrawCommand::BeginMask { mode } => {
            rectangles.begin_mask();
            PreparedItem::BeginMask { mode: *mode }
        }
        DrawCommand::EndMask => {
            rectangles.pop();
            PreparedItem::EndMask
        }
        DrawCommand::FillRect { .. } => {
            prepare_fill_rect(frame, command, tolerance, cache.as_deref_mut(), scope)?
        }
        DrawCommand::Fill { .. } => prepare_fill_path(frame, command, tolerance, cache, scope)?,
        _ => {
            return Err(ValidationError::unsupported(
                path,
                "command",
                "Unsupported drawing command",
            ));
        }
    };
    Ok(Some(item))
}

fn prepare_layer(
    frame: &mut PreparedFrame,
    command: &DrawCommand,
    tolerance: f64,
    rectangles: &mut rect_clip::Preparation,
    scope: &mut RasterScope<'_>,
) -> Result<PreparedItem, ValidationError> {
    let size = frame.size;
    let budget = &mut *scope.budget;
    let path = scope.path;
    match command {
        DrawCommand::PushLayer {
            opacity,
            bounds,
            blend_mode,
            clip,
        }
        | DrawCommand::PushRectLayer {
            opacity,
            bounds,
            blend_mode,
            clip,
            ..
        }
        | DrawCommand::PushShapeLayer {
            opacity,
            bounds,
            blend_mode,
            clip,
            ..
        } => Ok({
            let clip = clip
                .as_ref()
                .map(|clip| frame.prepare_clip(clip, tolerance, budget, &format!("{path}.clip")))
                .transpose()?;
            let rectangle = match command {
                DrawCommand::PushRectLayer { rect_clip, .. } => Some(rect_clip),
                DrawCommand::PushShapeLayer { rect_clip, .. } => rect_clip.as_ref(),
                _ => None,
            };
            let shape = if let DrawCommand::PushShapeLayer { shape_clip, .. } = command {
                Some(shape_clip)
            } else {
                None
            };
            let geometric = rectangles.push(rectangle, shape, frame, tolerance, budget, path)?;
            PreparedItem::PushLayer {
                layer: PreparedLayer {
                    opacity: *opacity,
                    bounds: pixel_bounds(*bounds, size),
                    blend_mode: *blend_mode,
                    alpha_clip: clip,
                    geometric_clip: geometric.filter(|_| shape.is_some() || rectangle.is_some()),
                    filters: None,
                },
            }
        }),
        _ => Err(ValidationError::unsupported(
            path,
            "command",
            "Unsupported drawing command",
        )),
    }
}

fn prepare_fill_rect(
    frame: &mut PreparedFrame,
    command: &DrawCommand,
    tolerance: f64,
    cache: Option<&mut CoverageCache>,
    scope: &mut RasterScope<'_>,
) -> Result<PreparedItem, ValidationError> {
    let budget = &mut *scope.budget;
    let path = scope.path;
    match command {
        DrawCommand::FillRect {
            bounds,
            transform,
            paint: source_paint,
        } => Ok({
            let [xx, yx, xy, yy, _, _] = transform.coefficients();
            let strip_start = frame.strips.len();
            let span_start = frame.spans.len();
            if (xy == 0.0 && yx == 0.0) || (xx == 0.0 && yy == 0.0) {
                let bounds = motion_core::display_list::mapped_bounds(*bounds, *transform, path)?;
                frame.rectangle(bounds, budget, path)?;
            } else {
                use motion_core::{PathVerb, Point};
                let verbs = [
                    PathVerb::MoveTo(Point::new(bounds.left, bounds.top)),
                    PathVerb::LineTo(Point::new(bounds.right, bounds.top)),
                    PathVerb::LineTo(Point::new(bounds.right, bounds.bottom)),
                    PathVerb::LineTo(Point::new(bounds.left, bounds.bottom)),
                    PathVerb::Close,
                ];
                frame.polygon(
                    verbs.into_iter(),
                    *transform,
                    motion_core::FillRule::NonZero,
                    tolerance,
                    budget,
                    path,
                )?;
            }
            let paint = frame.add_paint(
                source_paint,
                *bounds,
                *transform,
                budget,
                path,
                cache.map(|cache| &mut cache.paints),
            )?;
            PreparedItem::Fill {
                strips: strip_start..frame.strips.len(),
                spans: span_start..frame.spans.len(),
                paint,
            }
        }),
        _ => Err(ValidationError::unsupported(
            path,
            "command",
            "Unsupported drawing command",
        )),
    }
}

fn prepare_fill_path(
    frame: &mut PreparedFrame,
    command: &DrawCommand,
    tolerance: f64,
    mut cache: Option<&mut CoverageCache>,
    scope: &mut RasterScope<'_>,
) -> Result<PreparedItem, ValidationError> {
    let budget = &mut *scope.budget;
    let path = scope.path;
    match command {
        DrawCommand::Fill {
            path: geometry,
            transform,
            paint: source_paint,
            rule,
        } => Ok({
            let strip_start = frame.strips.len();
            let span_start = frame.spans.len();
            if let Some(cache) = cache.as_deref_mut() {
                cache.polygon(
                    frame,
                    geometry,
                    *transform,
                    *rule,
                    tolerance,
                    &mut RasterScope::new(budget, path),
                )?;
            } else {
                frame.polygon_path(geometry, *transform, *rule, tolerance, budget, path)?;
            }
            let paint = frame.add_paint(
                source_paint,
                geometry.bounding_box(),
                *transform,
                budget,
                path,
                cache.map(|cache| &mut cache.paints),
            )?;
            PreparedItem::Fill {
                strips: strip_start..frame.strips.len(),
                spans: span_start..frame.spans.len(),
                paint,
            }
        }),
        _ => Err(ValidationError::unsupported(
            path,
            "command",
            "Unsupported drawing command",
        )),
    }
}

fn trim_plus_layers(frame: &mut PreparedFrame, budget: &mut Budget) -> Result<(), ValidationError> {
    struct Capture {
        index: Option<usize>,
        content: Option<PixelRect>,
    }
    let union = |content: &mut Option<PixelRect>, bounds: PixelRect| {
        if bounds.pixels() > 0 {
            *content = Some(content.map_or(bounds, |old| PixelRect {
                left: old.left.min(bounds.left),
                top: old.top.min(bounds.top),
                right: old.right.max(bounds.right),
                bottom: old.bottom.max(bounds.bottom),
            }));
        }
    };
    let mut captures: Vec<Capture> = Vec::new();
    budget.work(frame.items.len() as u64, "prepared.layers")?;
    for index in 0..frame.items.len() {
        match &frame.items[index] {
            PreparedItem::PushLayer { .. } => budget.push(
                &mut captures,
                Capture {
                    index: Some(index),
                    content: None,
                },
                "prepared.layers",
            )?,
            PreparedItem::BeginMask { .. } => {
                if let Some(capture) = captures.last_mut()
                    && let Some(owner) = capture.index
                    && let PreparedItem::PushLayer { layer } = &frame.items[owner]
                {
                    capture.content = Some(layer.bounds);
                }
                budget.push(
                    &mut captures,
                    Capture {
                        index: None,
                        content: None,
                    },
                    "prepared.layers",
                )?;
            }
            PreparedItem::EndMask | PreparedItem::PopLayer => {
                let capture = captures.pop().expect("validated layer bounds stack");
                budget.release(size_of::<Capture>() as u64);
                if let Some(owner) = capture.index {
                    let PreparedItem::PushLayer { layer } = &mut frame.items[owner] else {
                        unreachable!("validated layer capture")
                    };
                    if layer.filters.is_none()
                        && layer.alpha_clip.is_none()
                        && layer.geometric_clip.is_none()
                        && matches!(
                            layer.blend_mode,
                            motion_core::BlendMode::Plus | motion_core::BlendMode::LinearPlus
                        )
                    {
                        layer.bounds =
                            layer.bounds.intersect(capture.content.unwrap_or(PixelRect {
                                left: 0,
                                top: 0,
                                right: 0,
                                bottom: 0,
                            }));
                    }
                    if let Some(parent) = captures.last_mut() {
                        union(&mut parent.content, layer.bounds);
                    }
                }
            }
            PreparedItem::Fill { strips, spans, .. } => {
                budget.work((strips.len() + spans.len()) as u64, "prepared.layers")?;
                if let Some(capture) = captures.last_mut() {
                    for strip in &frame.strips[strips.clone()] {
                        union(
                            &mut capture.content,
                            PixelRect {
                                left: strip.x,
                                top: strip.y,
                                right: strip.x + strip.width,
                                bottom: strip.y + strip.height,
                            },
                        );
                    }
                    for span in &frame.spans[spans.clone()] {
                        union(
                            &mut capture.content,
                            PixelRect {
                                left: span.start_x,
                                top: span.y,
                                right: span.end_x,
                                bottom: span.y + span.height,
                            },
                        );
                    }
                }
            }
            PreparedItem::ParticleInstances { .. } => {
                if let Some(capture) = captures.last_mut()
                    && let Some(owner) = capture.index
                    && let PreparedItem::PushLayer { layer } = &frame.items[owner]
                {
                    capture.content = Some(layer.bounds);
                }
            }
        }
    }
    Ok(())
}
