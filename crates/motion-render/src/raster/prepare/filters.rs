//! Filter guard-band padding, per-row filter support and padded command copies used while
//! preparing filtered display lists.

use motion_core::{DrawCommand, Filter, FilterOperation, Rect, ValidationError};
use std::mem::size_of;

pub(super) fn command_filter_bytes(command: &DrawCommand) -> u64 {
    if let DrawCommand::PushFilterLayer { filters, .. } = command {
        filters
            .iter()
            .map(|filter| size_of::<motion_core::Filter>() as u64 + filter.heap_bytes())
            .sum()
    } else {
        0
    }
}

pub(in crate::raster) fn filter_padding(commands: &[DrawCommand]) -> Result<u32, ValidationError> {
    let mut padding = 0.0_f64;
    let mut chain = 0.0_f64;
    let mut stack = Vec::new();
    for command in commands {
        if let DrawCommand::PushFilterLayer { filters, .. } = command {
            stack.push(chain);
            let mut current = Rect::new(0.0, 0.0, 1.0, 1.0);
            let mut capture = current;
            for filter in filters {
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
        } else {
            match command {
                DrawCommand::PushLayer { .. }
                | DrawCommand::PushRectLayer { .. }
                | DrawCommand::PushShapeLayer { .. }
                | DrawCommand::BeginMask { .. } => stack.push(chain),
                DrawCommand::PopLayer | DrawCommand::EndMask => {
                    chain = stack.pop().unwrap_or(0.0);
                }
                _ => {}
            }
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

pub(super) fn filter_support(filters: &[Filter]) -> Result<u32, ValidationError> {
    let mut rows = 0_u32;
    let blur = |sigma: f64| -> Result<u32, ValidationError> {
        let final_radii = motion_core::filter::box_radii(sigma)?;
        let draft_radii =
            motion_core::filter::quality_box_radii(sigma, motion_core::RenderQuality::Draft)?;
        let sum = |radii: [u32; 3]| {
            radii.into_iter().try_fold(0_u32, |total, radius| {
                total.checked_add(radius).ok_or_else(|| {
                    ValidationError::invalid(
                        "prepared.filters",
                        "support",
                        "Filter support overflow",
                    )
                })
            })
        };
        Ok(sum(final_radii)?.max(sum(draft_radii)?))
    };
    let offset = |value: f64| -> u32 { value.abs().ceil().min(f64::from(u32::MAX - 1)) as u32 + 1 };
    for filter in filters {
        let additional = match filter {
            Filter::Optics(optics) => match optics {
                motion_core::Optics::Bokeh { radius, .. } => offset(*radius),
                motion_core::Optics::Relight { .. } => 1,
                motion_core::Optics::LensVignette { .. } => 0,
                _ => u32::MAX,
            },
            Filter::Bloom { radius, levels, .. } => {
                blur(radius * f64::from(1u32 << (*levels - 1)))?
            }
            Filter::Blur { sigma } | Filter::Glow { sigma, .. } => blur(*sigma)?,
            Filter::DropShadow {
                offset: shift,
                sigma,
                ..
            } => blur(*sigma)?.saturating_add(offset(shift.y)),
            Filter::Custom(operations) => {
                let mut total = 0_u32;
                for operation in operations {
                    let additional = match operation {
                        FilterOperation::Transform(_) => u32::MAX,
                        FilterOperation::BlurXY { sigma_y, .. } => blur(*sigma_y)?,
                        FilterOperation::Blur { sigma } => blur(*sigma)?,
                        FilterOperation::Offset { y, .. } => offset(*y),
                        FilterOperation::ChannelOffset { red, green, blue } => {
                            offset(red.y.abs().max(green.y.abs()).max(blue.y.abs()))
                        }
                        _ => 0,
                    };
                    total = total.saturating_add(additional);
                }
                total
            }
            _ => 0,
        };
        rows = rows.saturating_add(additional);
    }
    Ok(rows)
}

pub(super) fn copy_filters(
    filters: &[motion_core::Filter],
    path: &str,
) -> Result<Vec<motion_core::Filter>, ValidationError> {
    let mut copy = Vec::new();
    copy.try_reserve_exact(filters.len()).map_err(|_| {
        ValidationError::limit(path, "max_bytes", "Could not reserve filter records")
    })?;
    for filter in filters {
        copy.push(if let motion_core::Filter::Custom(operations) = filter {
            let mut retained = Vec::new();
            retained.try_reserve_exact(operations.len()).map_err(|_| {
                ValidationError::limit(path, "max_bytes", "Could not reserve filter primitives")
            })?;
            retained.extend_from_slice(operations);
            motion_core::Filter::Custom(retained)
        } else {
            filter.clone()
        });
    }
    Ok(copy)
}

pub(super) fn shift_command(
    command: &DrawCommand,
    padding: f64,
    path: &str,
) -> Result<DrawCommand, ValidationError> {
    let mut command = if let DrawCommand::PushFilterLayer { bounds, filters } = command {
        DrawCommand::PushFilterLayer {
            bounds: *bounds,
            filters: copy_filters(filters, path)?,
        }
    } else {
        command.clone()
    };
    let shift = motion_core::Transform::translate(padding, padding);
    match &mut command {
        DrawCommand::PushFilterLayer { bounds, .. } => {
            bounds.left += padding;
            bounds.right += padding;
            bounds.top += padding;
            bounds.bottom += padding;
        }
        DrawCommand::PushLayer { bounds, clip, .. }
        | DrawCommand::PushRectLayer { bounds, clip, .. }
        | DrawCommand::PushShapeLayer { bounds, clip, .. } => {
            bounds.left += padding;
            bounds.right += padding;
            bounds.top += padding;
            bounds.bottom += padding;
            if let Some(clip) = clip {
                clip.transform = clip.transform.then(shift);
            }
        }
        DrawCommand::FillRect { transform, .. }
        | DrawCommand::Fill { transform, .. }
        | DrawCommand::ParticleInstances { transform, .. } => *transform = transform.then(shift),
        _ => {}
    }
    if let DrawCommand::PushRectLayer { rect_clip, .. }
    | DrawCommand::PushShapeLayer {
        rect_clip: Some(rect_clip),
        ..
    } = &mut command
    {
        rect_clip.transform = rect_clip.transform.then(shift);
    }
    if let DrawCommand::PushShapeLayer { shape_clip, .. } = &mut command {
        shape_clip.transform = shape_clip.transform.then(shift);
    }
    Ok(command)
}
