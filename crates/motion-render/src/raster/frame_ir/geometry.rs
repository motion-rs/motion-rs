//! Device curve transformation and conservative Wang admission without flattening.

use super::{FrameIr, GeometrySource, IrPath};
use motion_core::budget::Budget;
use motion_core::{FillRule, PathVerb, Point, Rect, Transform, ValidationError};

/// Quadratic-to-cubic degree elevation and Wang's bound use the reference arithmetic.
const WANG_FACTOR: f64 = 0.75;

impl<'a> FrameIr<'a> {
    pub(super) fn add_path(
        &mut self,
        source: GeometrySource<'a>,
        transform: Transform,
        rule: FillRule,
        budget: &mut Budget,
        path: &str,
    ) -> Result<usize, ValidationError> {
        let local_bounds = match source {
            GeometrySource::Rectangle(bounds) => bounds,
            GeometrySource::Path(geometry) => geometry.bounding_box(),
        };
        let bounds = motion_core::display_list::mapped_bounds(local_bounds, transform, path)?;
        let mut device = Vec::new();
        let rectangle = rectangle_verbs(local_bounds);
        let verbs = match source {
            GeometrySource::Rectangle(_) => rectangle.as_slice(),
            GeometrySource::Path(geometry) => geometry.verbs(),
        };
        budget.work(verbs.len() as u64, path)?;
        budget.bytes(std::mem::size_of_val(verbs) as u64, path)?;
        device.try_reserve_exact(verbs.len()).map_err(|_| {
            ValidationError::limit(path, "max_bytes", "Could not reserve device curves")
        })?;
        let mut current = Point::ORIGIN;
        let mut start = Point::ORIGIN;
        let mut segments = 0_u64;
        for verb in verbs {
            let mapped = match *verb {
                PathVerb::MoveTo(point) => PathVerb::MoveTo(transform.apply(point)),
                PathVerb::LineTo(point) => PathVerb::LineTo(transform.apply(point)),
                PathVerb::Close => PathVerb::Close,
                PathVerb::QuadraticTo(control, end) => {
                    PathVerb::QuadraticTo(transform.apply(control), transform.apply(end))
                }
                PathVerb::CubicTo(first, second, end) => PathVerb::CubicTo(
                    transform.apply(first),
                    transform.apply(second),
                    transform.apply(end),
                ),
            };
            let additional = match mapped {
                PathVerb::MoveTo(point) => {
                    current = point;
                    start = point;
                    1 // Implicit close of the preceding contour.
                }
                PathVerb::LineTo(point) => {
                    current = point;
                    1
                }
                PathVerb::Close => {
                    current = start;
                    1
                }
                PathVerb::QuadraticTo(control, end) => {
                    let curve = kurbo::QuadBez::new(
                        (current.x, current.y),
                        (control.x, control.y),
                        (end.x, end.y),
                    )
                    .raise();
                    current = end;
                    wang(
                        [curve.p0, curve.p1, curve.p2, curve.p3],
                        self.tolerance,
                        path,
                    )?
                }
                PathVerb::CubicTo(first, second, end) => {
                    let points = [current, first, second, end]
                        .map(|point| kurbo::Point::new(point.x, point.y));
                    current = end;
                    wang(points, self.tolerance, path)?
                }
            };
            segments = segments.checked_add(additional).ok_or_else(|| {
                ValidationError::limit(path, "max_work", "Path segment bound overflow")
            })?;
            device.push(mapped);
        }
        segments = segments.checked_add(1).ok_or_else(|| {
            ValidationError::limit(path, "max_work", "Path segment bound overflow")
        })?;
        self.segments = self.segments.checked_add(segments).ok_or_else(|| {
            ValidationError::limit(path, "max_work", "Frame segment bound overflow")
        })?;
        let index = self.paths.len();
        budget.push(
            &mut self.paths,
            IrPath {
                source,
                transform,
                rule,
                bounds,
                device_verbs: device,
                segment_upper_bound: segments,
            },
            path,
        )?;
        Ok(index)
    }
}

pub(in crate::raster) fn rectangle_verbs(bounds: Rect) -> [PathVerb; 5] {
    [
        PathVerb::MoveTo(Point::new(bounds.left, bounds.top)),
        PathVerb::LineTo(Point::new(bounds.right, bounds.top)),
        PathVerb::LineTo(Point::new(bounds.right, bounds.bottom)),
        PathVerb::LineTo(Point::new(bounds.left, bounds.bottom)),
        PathVerb::Close,
    ]
}

pub(in crate::raster) fn wang(
    points: [kurbo::Point; 4],
    tolerance: f64,
    path: &str,
) -> Result<u64, ValidationError> {
    let [first, second, third, fourth] = points;
    let left = (third - second) - (second - first);
    let right = (fourth - third) - (third - second);
    let curvature = libm::hypot(left.x, left.y).max(libm::hypot(right.x, right.y));
    let count = if curvature == 0.0 {
        1.0
    } else {
        (libm::ceil(libm::sqrt(WANG_FACTOR * curvature / tolerance) * 0.5) * 2.0).max(2.0)
    };
    if !count.is_finite() || count >= u64::MAX as f64 {
        return Err(ValidationError::limit(
            path,
            "max_work",
            "Wang subdivision count overflow",
        ));
    }
    Ok(count as u64)
}
