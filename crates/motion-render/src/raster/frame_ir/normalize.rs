//! Ordered scope, clip, paint and resource normalization before backend preparation.

use super::{COMMAND_PATH_BYTES, FrameIr, GeometrySource, IrClip, IrItem, IrPaint};
use crate::prepared::PreparedLayer;
use crate::raster::prepare::filters::{copy_filters, filter_support};
use motion_core::budget::Budget;
use motion_core::{
    DrawCommand, FillRule, Paint, Rect, Transform, ValidationError, drawing::ClipGeometry,
};
use std::mem::size_of;

/// Four half-float channels per retained immutable pattern texel.
const PATTERN_TEXEL_BYTES: u64 = 8;

impl<'a> FrameIr<'a> {
    pub(super) fn normalize(
        &mut self,
        commands: &'a [DrawCommand],
        padding: u32,
        budget: &mut Budget,
    ) -> Result<(), ValidationError> {
        let shift = Transform::translate(f64::from(padding), f64::from(padding));
        let transform = |value: Transform| {
            if padding == 0 {
                value
            } else {
                value.then(shift)
            }
        };
        let bounds = |value: Rect| {
            Rect::new(
                value.left + f64::from(padding),
                value.top + f64::from(padding),
                value.right + f64::from(padding),
                value.bottom + f64::from(padding),
            )
        };
        let mut stack = Vec::new();
        let mut halo = 0_u32;
        for (index, command) in commands.iter().enumerate() {
            budget.work(1, "frame_ir.commands")?;
            budget.bytes(COMMAND_PATH_BYTES, "frame_ir.commands")?;
            let path = format!("display_list.commands[{index}]");
            let item = match command {
                DrawCommand::ParticleInstances {
                    batch,
                    transform: matrix,
                } => {
                    self.resources(batch.bytes(), budget, &path)?;
                    IrItem::ParticleInstances {
                        batch: batch.clone(),
                        transform: transform(*matrix),
                    }
                }
                DrawCommand::PushFilterLayer {
                    bounds: source,
                    filters,
                } => {
                    let source = if padding == 0 {
                        *source
                    } else {
                        bounds(*source)
                    };
                    let mut capture = source;
                    let mut current = source;
                    for filter in filters {
                        budget.work(filter.structural_cost() as u64, &path)?;
                        budget.bytes(
                            size_of::<motion_core::Filter>() as u64 + filter.heap_bytes(),
                            &path,
                        )?;
                        let (output, intermediate) = filter.extent(current, &path)?;
                        capture = motion_core::filter::union(capture, intermediate);
                        current = output;
                    }
                    budget.push(&mut stack, halo, &path)?;
                    halo = halo.saturating_add(filter_support(filters)?);
                    self.filter_halo = self.filter_halo.max(halo);
                    IrItem::PushLayer {
                        layer: PreparedLayer {
                            opacity: 1.0,
                            bounds: crate::raster::prepare::pixel_bounds(capture, self.size),
                            blend_mode: motion_core::BlendMode::SourceOver,
                            alpha_clip: None,
                            geometric_clip: None,
                            filters: Some(crate::prepared::PreparedFilters {
                                source_bounds: source,
                                operations: copy_filters(filters, &path)?,
                            }),
                        },
                        alpha_clip: None,
                        rectangle_clip: None,
                        shape_clip: None,
                    }
                }
                DrawCommand::PushLayer {
                    opacity,
                    bounds: source,
                    blend_mode,
                    clip,
                }
                | DrawCommand::PushRectLayer {
                    opacity,
                    bounds: source,
                    blend_mode,
                    clip,
                    ..
                }
                | DrawCommand::PushShapeLayer {
                    opacity,
                    bounds: source,
                    blend_mode,
                    clip,
                    ..
                } => {
                    let alpha_clip = if let Some(clip) = clip {
                        let clip_path = format!("{path}.clip");
                        let matrix = transform(clip.transform);
                        let geometry = self.add_path(
                            GeometrySource::Path(&clip.path),
                            matrix,
                            clip.rule,
                            budget,
                            &clip_path,
                        )?;
                        let mut source = clip.clone();
                        source.transform = matrix;
                        Some(self.add_clip(
                            IrClip::Alpha {
                                source,
                                path: geometry,
                            },
                            budget,
                            &clip_path,
                        )?)
                    } else {
                        None
                    };
                    let rectangle = match command {
                        DrawCommand::PushRectLayer { rect_clip, .. } => Some(rect_clip),
                        DrawCommand::PushShapeLayer { rect_clip, .. } => rect_clip.as_ref(),
                        _ => None,
                    };
                    let rectangle_clip = if let Some(clip) = rectangle {
                        let matrix = transform(clip.transform);
                        let geometry = self.add_path(
                            GeometrySource::Rectangle(clip.bounds),
                            matrix,
                            FillRule::NonZero,
                            budget,
                            &path,
                        )?;
                        let mut source = *clip;
                        source.transform = matrix;
                        Some(self.add_clip(
                            IrClip::Rectangle {
                                source,
                                path: geometry,
                            },
                            budget,
                            &path,
                        )?)
                    } else {
                        None
                    };
                    let shape_clip = if let DrawCommand::PushShapeLayer { shape_clip, .. } = command
                    {
                        let matrix = transform(shape_clip.transform);
                        let (geometry, rule) = match &shape_clip.geometry {
                            ClipGeometry::Rectangle(rect) => {
                                (GeometrySource::Rectangle(*rect), FillRule::NonZero)
                            }
                            ClipGeometry::Path(path) => {
                                (GeometrySource::Path(path), shape_clip.rule)
                            }
                        };
                        let geometry = self.add_path(geometry, matrix, rule, budget, &path)?;
                        let mut source = shape_clip.clone();
                        source.transform = matrix;
                        Some(self.add_clip(
                            IrClip::Shape {
                                source,
                                path: geometry,
                            },
                            budget,
                            &path,
                        )?)
                    } else {
                        None
                    };
                    budget.push(&mut stack, halo, &path)?;
                    IrItem::PushLayer {
                        layer: PreparedLayer {
                            opacity: *opacity,
                            bounds: crate::raster::prepare::pixel_bounds(
                                if padding == 0 {
                                    *source
                                } else {
                                    bounds(*source)
                                },
                                self.size,
                            ),
                            blend_mode: *blend_mode,
                            alpha_clip: None,
                            geometric_clip: None,
                            filters: None,
                        },
                        alpha_clip,
                        rectangle_clip,
                        shape_clip,
                    }
                }
                DrawCommand::PopLayer | DrawCommand::EndMask => {
                    halo = stack.pop().ok_or_else(|| {
                        ValidationError::invalid(&path, "scope", "Unbalanced frame scope")
                    })?;
                    budget.release(size_of::<u32>() as u64);
                    if matches!(command, DrawCommand::PopLayer) {
                        IrItem::PopLayer
                    } else {
                        IrItem::EndMask
                    }
                }
                DrawCommand::BeginMask { mode } => {
                    budget.push(&mut stack, halo, &path)?;
                    IrItem::BeginMask { mode: *mode }
                }
                DrawCommand::FillRect {
                    bounds,
                    transform: matrix,
                    paint,
                } => {
                    let matrix = transform(*matrix);
                    let geometry = self.add_path(
                        GeometrySource::Rectangle(*bounds),
                        matrix,
                        FillRule::NonZero,
                        budget,
                        &path,
                    )?;
                    let paint = self.add_paint(paint, *bounds, matrix, budget, &path)?;
                    IrItem::Fill {
                        path: geometry,
                        paint,
                    }
                }
                DrawCommand::Fill {
                    path: geometry,
                    transform: matrix,
                    paint,
                    rule,
                } => {
                    let matrix = transform(*matrix);
                    let path_index = self.add_path(
                        GeometrySource::Path(geometry),
                        matrix,
                        *rule,
                        budget,
                        &path,
                    )?;
                    let paint =
                        self.add_paint(paint, geometry.bounding_box(), matrix, budget, &path)?;
                    IrItem::Fill {
                        path: path_index,
                        paint,
                    }
                }
                _ => {
                    return Err(ValidationError::unsupported(
                        &path,
                        "command",
                        "Unsupported drawing command",
                    ));
                }
            };
            budget.push(&mut self.items, item, &path)?;
            budget.release(COMMAND_PATH_BYTES);
        }
        Ok(())
    }

    fn add_clip(
        &mut self,
        clip: IrClip,
        budget: &mut Budget,
        path: &str,
    ) -> Result<usize, ValidationError> {
        let index = self.clips.len();
        budget.push(&mut self.clips, clip, path)?;
        Ok(index)
    }

    fn add_paint(
        &mut self,
        source: &'a Paint,
        bounds: Rect,
        transform: Transform,
        budget: &mut Budget,
        path: &str,
    ) -> Result<usize, ValidationError> {
        if let Some(gradient) = source.gradient() {
            self.resources(
                std::mem::size_of_val(gradient.stops.as_ref()) as u64,
                budget,
                path,
            )?;
        }
        if let Paint::Solid(color) = source {
            crate::raster::prepare::paint_record(*color, path)?;
        }
        if let Paint::Pattern(pattern) = source
            && self.seen_images.insert(pattern.image.identity())
        {
            budget.work(pattern.image.levels() as u64, path)?;
            for level in 0..pattern.image.levels() {
                let (width, height) = pattern.image.size(level);
                let bytes = u64::from(width)
                    .checked_mul(u64::from(height))
                    .and_then(|pixels| pixels.checked_mul(PATTERN_TEXEL_BYTES))
                    .ok_or_else(|| {
                        ValidationError::limit(path, "max_bytes", "Pattern resource bytes overflow")
                    })?;
                self.resources(bytes, budget, path)?;
            }
        }
        let index = self.paints.len();
        budget.push(
            &mut self.paints,
            IrPaint {
                source,
                bounds,
                transform,
            },
            path,
        )?;
        Ok(index)
    }

    fn resources(
        &mut self,
        bytes: u64,
        budget: &mut Budget,
        path: &str,
    ) -> Result<(), ValidationError> {
        budget.bytes(bytes, path)?;
        self.resource_bytes = self.resource_bytes.checked_add(bytes).ok_or_else(|| {
            ValidationError::limit(path, "max_bytes", "Frame resource bytes overflow")
        })?;
        Ok(())
    }
}

pub(super) fn padding(
    commands: &[DrawCommand],
    budget: &mut Budget,
) -> Result<u32, ValidationError> {
    let mut padding = 0.0_f64;
    let mut chain = 0.0_f64;
    let mut stack = Vec::new();
    budget.work(commands.len() as u64, "prepared.filters")?;
    for command in commands {
        match command {
            DrawCommand::PushFilterLayer { filters, .. } => {
                budget.push(&mut stack, chain, "prepared.filters")?;
                let mut current = Rect::new(0.0, 0.0, 1.0, 1.0);
                let mut capture = current;
                for filter in filters {
                    budget.work(filter.structural_cost() as u64, "prepared.filters")?;
                    let (output, intermediate) = filter.extent(current, "prepared.filters")?;
                    current = output;
                    capture = motion_core::filter::union(capture, intermediate);
                }
                chain += (-capture.left)
                    .max(-capture.top)
                    .max(capture.right - 1.0)
                    .max(capture.bottom - 1.0)
                    .max(0.0)
                    .ceil();
                padding = padding.max(chain);
            }
            DrawCommand::PushLayer { .. }
            | DrawCommand::PushRectLayer { .. }
            | DrawCommand::PushShapeLayer { .. }
            | DrawCommand::BeginMask { .. } => {
                budget.push(&mut stack, chain, "prepared.filters")?;
            }
            DrawCommand::PopLayer | DrawCommand::EndMask => {
                chain = stack.pop().ok_or_else(|| {
                    ValidationError::invalid("prepared.filters", "scope", "Unbalanced filter scope")
                })?;
                budget.release(size_of::<f64>() as u64);
            }
            _ => {}
        }
    }
    if !padding.is_finite() || padding > f64::from(u32::MAX) {
        return Err(ValidationError::limit(
            "prepared.filters",
            "max_pixels",
            "Filter guard band exceeds addressable pixels",
        ));
    }
    Ok(padding as u32)
}
