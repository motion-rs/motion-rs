//! Conservative curve descriptors and Wang-sized GPU edge storage without host flattening.

use super::{
    DEVICE_COORDINATE_LIMIT, DEVICE_MAX_FILL_VISITS, DEVICE_MAX_FILLS, DEVICE_MAX_FRAME_VISITS,
    DEVICE_MAX_PIXELS, DeviceGeometry, DeviceTotals, TileLayout,
};
use crate::prepared::{DeviceFill, PreparedScene, Strip};
use crate::raster::{RasterScope, frame_ir, pixel_bounds};
use motion_core::{PathVerb, Point, ValidationError};

#[derive(Clone, Debug, PartialEq)]
pub(crate) struct DeviceCurve {
    pub(crate) points: [Point; 4],
    pub(crate) segments: u32,
    #[cfg(test)]
    pub(super) tolerance: f64,
}

impl DeviceGeometry {
    pub(super) fn curve_fill(
        &mut self,
        scene: &mut PreparedScene,
        geometry: &frame_ir::IrPath<'_>,
        tolerance: f64,
        scope: &mut RasterScope<'_>,
    ) -> Result<bool, ValidationError> {
        let (budget, path) = (&mut *scope.budget, scope.path);
        let pixels = pixel_bounds(geometry.bounds, scene.size);
        if pixels.is_empty() {
            return Ok(true);
        }
        let used = self.device_totals;
        if self.device_fills.len() >= DEVICE_MAX_FILLS
            || used.pixels + pixels.pixels() > DEVICE_MAX_PIXELS
        {
            self.fallback_paths.frame_limit += 1;
            return Ok(false);
        }
        // Control hulls bound every generated point. Reject before allocating descriptors.
        let within = |point: Point| {
            point.x.is_finite()
                && point.y.is_finite()
                && (point.x - f64::from(pixels.left)).abs() <= DEVICE_COORDINATE_LIMIT
                && (point.y - f64::from(pixels.top)).abs() <= DEVICE_COORDINATE_LIMIT
        };
        if !geometry.device_verbs.iter().all(|verb| match *verb {
            PathVerb::MoveTo(a) | PathVerb::LineTo(a) => within(a),
            PathVerb::QuadraticTo(a, b) => within(a) && within(b),
            PathVerb::CubicTo(a, b, c) => within(a) && within(b) && within(c),
            PathVerb::Close => true,
        }) {
            self.fallback_paths.coordinate_range += 1;
            return Ok(false);
        }
        let bytes =
            (geometry.device_verbs.len() as u64 + 1) * std::mem::size_of::<DeviceCurve>() as u64;
        budget.bytes(bytes, path)?;
        budget.work(
            geometry.device_verbs.len() as u64 + geometry.segment_upper_bound,
            path,
        )?;
        let mut curves = Vec::new();
        curves
            .try_reserve_exact(geometry.device_verbs.len() + 1)
            .map_err(|_| {
                ValidationError::limit(path, "max_bytes", "Could not reserve device curves")
            })?;
        let mut current = Point::ORIGIN;
        let mut start = Point::ORIGIN;
        let mut open = false;
        let mut append = |points: [Point; 4], line: bool| -> Result<(), ValidationError> {
            let count = if line {
                1
            } else {
                frame_ir::wang(points.map(|p| kurbo::Point::new(p.x, p.y)), tolerance, path)?
            };
            let segments = u32::try_from(count).map_err(|_| {
                ValidationError::limit(path, "max_work", "Device curve segment count overflow")
            })?;
            curves.push(DeviceCurve {
                points,
                segments,
                #[cfg(test)]
                tolerance,
            });
            Ok(())
        };
        for verb in &geometry.device_verbs {
            match *verb {
                PathVerb::MoveTo(point) => {
                    if open && current != start {
                        append([current, current, start, start], true)?;
                    }
                    current = point;
                    start = point;
                    open = true;
                }
                PathVerb::LineTo(point) => {
                    append([current, current, point, point], true)?;
                    current = point;
                }
                PathVerb::QuadraticTo(control, point) => {
                    let curve = kurbo::QuadBez::new(
                        (current.x, current.y),
                        (control.x, control.y),
                        (point.x, point.y),
                    )
                    .raise();
                    append(
                        [curve.p0, curve.p1, curve.p2, curve.p3].map(|p| Point::new(p.x, p.y)),
                        false,
                    )?;
                    current = point;
                }
                PathVerb::CubicTo(a, b, point) => {
                    append([current, a, b, point], false)?;
                    current = point;
                }
                PathVerb::Close => {
                    if current != start {
                        append([current, current, start, start], true)?;
                    }
                    current = start;
                    open = false;
                }
            }
        }
        if open && current != start {
            append([current, current, start, start], true)?;
        }
        let segments = curves
            .iter()
            .map(|curve| u64::from(curve.segments))
            .sum::<u64>();
        debug_assert!(segments <= geometry.segment_upper_bound);
        let Some(tiles) = TileLayout::admit_curves(&curves, pixels, budget, path)? else {
            self.fallback_paths.tile_capacity += 1;
            budget.release(bytes);
            return Ok(false);
        };
        if tiles.work > DEVICE_MAX_FILL_VISITS || used.visits + tiles.work > DEVICE_MAX_FRAME_VISITS
        {
            self.fallback_paths.work_limit += 1;
            budget.release(bytes);
            return Ok(false);
        }
        let retained = curves.len() as u64 * std::mem::size_of::<DeviceCurve>() as u64;
        budget.bytes(retained, path)?;
        self.device_curves.try_reserve(curves.len()).map_err(|_| {
            ValidationError::limit(path, "max_bytes", "Could not retain device curves")
        })?;
        let first = self.device_curves.len();
        self.device_curves.extend(curves);
        budget.release(bytes);
        let alpha_offset = self
            .device_fills
            .last()
            .map_or(0, |fill| Self::device_sample_end(&scene.strips[fill.strip]));
        budget.push(
            &mut self.device_fills,
            DeviceFill {
                strip: scene.strips.len(),
                edges: 0..0,
                curves: first..self.device_curves.len(),
                segments: segments as u32,
                rule: geometry.rule,
                tiles,
            },
            path,
        )?;
        budget.push(
            &mut scene.strips,
            Strip {
                x: pixels.left,
                y: pixels.top,
                width: pixels.width(),
                height: pixels.height(),
                alpha_offset,
            },
            path,
        )?;
        self.device_totals = DeviceTotals {
            pixels: used.pixels + pixels.pixels(),
            visits: used.visits + tiles.work,
        };
        Ok(true)
    }
}
