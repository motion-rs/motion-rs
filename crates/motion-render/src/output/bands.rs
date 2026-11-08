//! Banded frame conversion that renders prepared frames in bounded horizontal strips.

use super::*;
use crate::{LinearRenderStats, OutputStats, PixelRect, PreparedFrame};
use motion_core::budget::Budget;

#[cfg(test)]
mod tests;

fn smaller_band(height: u32) -> u32 {
    (height / 2 / 4 * 4).max(16)
}

fn band_height(halo: u32, height: u32) -> u32 {
    if halo.saturating_mul(16) >= height {
        height
    } else {
        halo.saturating_mul(8).max(16).min(height)
    }
}

impl FrameBuffer {
    pub(crate) fn from_prepared(
        prepared: &PreparedFrame,
        format: PixelFormat,
        limits: RasterLimits,
        surfaces: &mut crate::cpu::pool::SurfacePool,
        pool: &OutputPool,
    ) -> Result<(Self, LinearRenderStats, OutputStats), ValidationError> {
        Self::from_prepared_banded(
            prepared,
            format,
            limits,
            surfaces,
            pool,
            band_height(prepared.filter_halo, prepared.size().height),
        )
    }

    pub(crate) fn from_prepared_display(
        prepared: &PreparedFrame,
        format: PixelFormat,
        limits: RasterLimits,
        surfaces: &mut crate::cpu::pool::SurfacePool,
        pool: &OutputPool,
        display: crate::display::DisplaySettings<'_>,
    ) -> Result<(Self, LinearRenderStats, OutputStats), ValidationError> {
        if display.display.is_none() {
            return Self::from_prepared(prepared, format, limits, surfaces, pool);
        }
        Self::from_prepared_transformed(
            prepared,
            format,
            limits,
            surfaces,
            pool,
            (
                band_height(prepared.filter_halo, prepared.size().height),
                display,
            ),
        )
    }

    fn from_prepared_banded(
        prepared: &PreparedFrame,
        format: PixelFormat,
        limits: RasterLimits,
        surfaces: &mut crate::cpu::pool::SurfacePool,
        pool: &OutputPool,
        band_height: u32,
    ) -> Result<(Self, LinearRenderStats, OutputStats), ValidationError> {
        Self::from_prepared_transformed(
            prepared,
            format,
            limits,
            surfaces,
            pool,
            (band_height, crate::display::DisplaySettings::default()),
        )
    }

    fn from_prepared_transformed(
        prepared: &PreparedFrame,
        format: PixelFormat,
        limits: RasterLimits,
        surfaces: &mut crate::cpu::pool::SurfacePool,
        pool: &OutputPool,
        settings: (u32, crate::display::DisplaySettings<'_>),
    ) -> Result<(Self, LinearRenderStats, OutputStats), ValidationError> {
        let (mut band_height, display) = settings;
        let size = prepared.size();
        let depth = match format {
            PixelFormat::SrgbRgba8 { .. } => 4,
            PixelFormat::LinearRgba16Float => 8,
            PixelFormat::Yuv420 { bit_depth } => {
                if !size.width.is_multiple_of(2)
                    || !size.height.is_multiple_of(2)
                    || ![8, 10].contains(&bit_depth)
                {
                    return Err(output_invalid(
                        "yuv420",
                        "YUV420 requires even dimensions and 8/10-bit samples",
                    ));
                }
                if bit_depth == 8 { 1 } else { 2 }
            }
        };
        let layouts = if matches!(format, PixelFormat::Yuv420 { .. }) {
            vec![
                (size.width, size.height),
                (size.width / 2, size.height / 2),
                (size.width / 2, size.height / 2),
            ]
        } else {
            vec![(size.width, size.height)]
        };
        let payload = layouts
            .iter()
            .map(|(width, height)| u64::from(*width) * u64::from(*height) * depth)
            .sum::<u64>();
        let retained = payload
            + std::mem::size_of::<Self>() as u64
            + layouts.len() as u64 * std::mem::size_of::<PixelPlane>() as u64;
        let mut budget = Budget::new(prepared.raster_size(), limits)?;
        budget.bytes(prepared.stats().retained_bytes, "linear.prepared")?;
        budget.bytes(retained, "output")?;
        let conversion = Self::display_conversion_work(size, format, display.display);
        budget.work(conversion, "output")?;
        pool.trim(limits.max_bytes / 8);
        let mut planes = Vec::new();
        for (width, height) in layouts {
            let stride = width as usize * depth as usize;
            let mut bytes = storage(stride * height as usize, pool)?;
            bytes.resize(stride * height as usize, 0);
            planes.push(PixelPlane {
                width,
                height,
                stride,
                bytes,
            });
        }
        let mut output = Self {
            size,
            format,
            planes,
            pool: Some(pool.clone()),
        };
        let halo = prepared.filter_halo;
        let origin = prepared.output_bounds();
        let mut peak = budget.peak_bytes();
        let mut work = 0_u64;
        let mut row = 0;
        while row < size.height {
            let height = (size.height - row).min(band_height);
            let band_bounds = PixelRect {
                top: origin.top + row,
                bottom: origin.top + row + height,
                ..origin
            };
            let domain = PixelRect {
                left: 0,
                right: prepared.raster_size().width,
                top: band_bounds.top.saturating_sub(halo),
                bottom: band_bounds
                    .bottom
                    .saturating_add(halo)
                    .min(prepared.raster_size().height),
            };
            let max_work = limits
                .max_work
                .checked_sub(budget.work_done())
                .filter(|work| *work > 0)
                .ok_or_else(|| {
                    ValidationError::limit(
                        "linear",
                        "max_work",
                        "Frame exhausted its shared work budget",
                    )
                })?;
            let max_bytes = limits
                .max_bytes
                .checked_sub(retained)
                .filter(|bytes| *bytes > 0)
                .ok_or_else(|| {
                    ValidationError::limit(
                        "output",
                        "max_bytes",
                        "Output exhausts the byte allowance",
                    )
                })?;
            let mut attempt = LinearRenderStats {
                peak_bytes: 0,
                work: 0,
            };
            let linear = prepared.render_linear_bounds_counted(
                RasterLimits {
                    max_work,
                    max_bytes,
                    ..limits
                },
                surfaces,
                band_bounds,
                domain,
                &mut attempt,
            );
            budget.work(attempt.work, "linear")?;
            work += attempt.work;
            peak = peak.max(attempt.peak_bytes + retained);
            let linear = match linear {
                Err(error)
                    if error.field == "max_bytes"
                        && matches!(
                            error.kind,
                            motion_core::ValidationErrorKind::StructuralLimit
                        )
                        && band_height > 16 =>
                {
                    band_height = smaller_band(band_height);
                    continue;
                }
                result => result?,
            };
            let conversion_bytes = max_bytes
                .checked_sub(prepared.stats().retained_bytes)
                .filter(|bytes| *bytes > 0)
                .ok_or_else(|| {
                    ValidationError::limit(
                        "output",
                        "max_bytes",
                        "Prepared frame exhausts conversion storage",
                    )
                })?;
            let encoded = Self::from_linear_transformed(
                &linear,
                format,
                RasterLimits {
                    max_bytes: conversion_bytes,
                    ..limits
                },
                pool,
                crate::display::DisplaySettings {
                    origin: [origin.left, origin.top + row],
                    ..display
                },
            );
            let source =
                linear.pixels().len() as u64 * 16 + std::mem::size_of::<LinearFrame>() as u64;
            linear.recycle(surfaces);
            let encoded = match encoded {
                Err(error)
                    if error.field == "max_bytes"
                        && matches!(
                            error.kind,
                            motion_core::ValidationErrorKind::StructuralLimit
                        )
                        && band_height > 16 =>
                {
                    band_height = smaller_band(band_height);
                    continue;
                }
                result => result?,
            };
            peak = peak.max(
                retained + prepared.stats().retained_bytes + source + encoded.retained_bytes(),
            );
            for (index, (destination, band)) in
                output.planes.iter_mut().zip(&encoded.planes).enumerate()
            {
                let target_row = if index == 0 { row } else { row / 2 };
                let start = target_row as usize * destination.stride;
                destination.bytes[start..start + band.bytes.len()].copy_from_slice(&band.bytes);
            }
            row += height;
        }
        Ok((
            output,
            LinearRenderStats {
                peak_bytes: peak,
                work,
            },
            OutputStats {
                peak_bytes: peak,
                work: conversion,
            },
        ))
    }
}
