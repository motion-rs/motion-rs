//! Shared filter support and fallible copies of admitted filter descriptions.

use motion_core::{Filter, FilterOperation, ValidationError};

pub(in crate::raster) fn filter_support(filters: &[Filter]) -> Result<u32, ValidationError> {
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

pub(in crate::raster) fn copy_filters(
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
