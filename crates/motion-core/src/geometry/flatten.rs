//! Shared bounded Bezier-to-edge geometry, independent of pixel composition.
use crate::budget::{Budget, RasterLimits};
use crate::{PathVerb, PixelSize, Point, Rect, Transform, ValidationError};
use kurbo::{CubicBez, ParamCurve, QuadBez};
use std::mem::size_of;

#[derive(Clone, Copy)]
/// Nonhorizontal directed edge stored from smaller to larger vertical coordinate.
pub struct Edge {
    /// Endpoint with the smaller vertical coordinate.
    pub top: Point,
    /// Endpoint with the larger vertical coordinate.
    pub bottom: Point,
    /// Original direction: positive for increasing vertical coordinates, negative otherwise.
    pub winding: i32,
}

impl Edge {
    /// Interpolates the horizontal coordinate, clamping queries to this edge's endpoints.
    pub fn at(self, vertical: f64) -> f64 {
        if self.top.x == self.bottom.x {
            return self.top.x;
        }
        let progress = ((vertical - self.top.y) / (self.bottom.y - self.top.y)).clamp(0.0, 1.0);
        self.top.x * (1.0 - progress) + self.bottom.x * progress
    }

    /// Rejects visible opposite-sign projection cancellation above the precision guard.
    /// `start` and `end` describe the vertical band; `width` is its raster viewport.
    pub fn check_projection(
        self,
        start: f64,
        end: f64,
        width: u32,
        path: &str,
    ) -> Result<(), ValidationError> {
        let first = self.at(start);
        let last = self.at(end);
        let cancellation =
            (self.top.x < 0.0 && self.bottom.x > 0.0) || (self.top.x > 0.0 && self.bottom.x < 0.0);
        let error = self.top.x.abs().max(self.bottom.x.abs()) * (16.0 * f64::EPSILON);
        if cancellation
            && first.min(last) <= f64::from(width)
            && first.max(last) >= 0.0
            && error > 1.0e-7
        {
            return Err(ValidationError::invalid(
                path,
                "geometry",
                "Visible edge cancellation exceeds the 1e-7 device-pixel precision guard",
            ));
        }
        Ok(())
    }
}

/// Flattens a path for geometric surface sampling and returns edges plus charged work.
/// Uses the edge storage allowance and at most four million work units; allocates no pixels.
pub fn surface_edges(
    source: &crate::Path2D,
    transform: Transform,
    tolerance: f64,
    max_edges: usize,
    max_work: u64,
) -> Result<(Vec<Edge>, u64), ValidationError> {
    if max_work == 0 {
        return Err(ValidationError::limit(
            "vector_tree.surface",
            "max_work",
            "Surface preparation work exhausted",
        ));
    }
    let mut budget = Budget::new(
        PixelSize::new(1, 1),
        RasterLimits {
            max_pixels: 1,
            max_bytes: (max_edges as u64 + 1) * size_of::<Edge>() as u64,
            max_work: max_work.min(4_000_000),
        },
    )?;
    let edges = path_edges(
        source.verbs().iter().copied(),
        transform,
        tolerance,
        &mut budget,
        "vector_tree.surface",
    )?;
    Ok((edges, budget.work_done()))
}

/// Device-space bounds of flattened edges; infinite and inverted when there are none.
pub fn edge_bounds(edges: &[Edge]) -> Rect {
    let mut bounds = Rect::new(
        f64::INFINITY,
        f64::INFINITY,
        f64::NEG_INFINITY,
        f64::NEG_INFINITY,
    );
    for edge in edges {
        bounds.left = bounds.left.min(edge.top.x).min(edge.bottom.x);
        bounds.right = bounds.right.max(edge.top.x).max(edge.bottom.x);
        bounds.top = bounds.top.min(edge.top.y);
        bounds.bottom = bounds.bottom.max(edge.bottom.y);
    }
    bounds
}

/// Transforms verbs into directed edges, implicitly closing each contour.
/// Curve precision, generated storage and flattening work share the caller's budget.
pub fn path_edges(
    verbs: impl Iterator<Item = PathVerb>,
    transform: Transform,
    tolerance: f64,
    budget: &mut Budget,
    path: &str,
) -> Result<Vec<Edge>, ValidationError> {
    let mut edges = Vec::new();
    let mut current = Point::ORIGIN;
    let mut start = Point::ORIGIN;
    for verb in verbs {
        budget.work(1, path)?;
        match verb {
            PathVerb::MoveTo(point) => {
                add_edge(&mut edges, current, start, budget, path)?;
                start = transform.apply(point);
                current = start;
            }
            PathVerb::LineTo(point) => {
                let end = transform.apply(point);
                add_edge(&mut edges, current, end, budget, path)?;
                current = end;
            }
            PathVerb::Close => {
                add_edge(&mut edges, current, start, budget, path)?;
                current = start;
            }
            PathVerb::QuadraticTo(control, end) => {
                let control = transform.apply(control);
                let end = transform.apply(end);
                let curve = QuadBez::new(
                    (current.x, current.y),
                    (control.x, control.y),
                    (end.x, end.y),
                )
                .raise();
                flatten_curve(&mut edges, curve, tolerance, budget, path)?;
                current = end;
            }
            PathVerb::CubicTo(first, second, end) => {
                let first = transform.apply(first);
                let second = transform.apply(second);
                let end = transform.apply(end);
                let curve = CubicBez::new(
                    (current.x, current.y),
                    (first.x, first.y),
                    (second.x, second.y),
                    (end.x, end.y),
                );
                flatten_curve(&mut edges, curve, tolerance, budget, path)?;
                current = end;
            }
        }
    }
    add_edge(&mut edges, current, start, budget, path)?;
    Ok(edges)
}

/// Appends bounded curve edges, charging generated storage and flattening work.
/// Rejects coordinates that cannot resolve the requested device-space tolerance.
/// Wang's uniform subdivision bounds chord error by `3 * curvature / (4 * count^2)`.
/// Curved counts remain even so symmetric reversals do not collapse into omitted horizontal edges.
pub fn flatten_curve(
    edges: &mut Vec<Edge>,
    curve: CubicBez,
    tolerance: f64,
    budget: &mut Budget,
    path: &str,
) -> Result<(), ValidationError> {
    budget.work(1, path)?;
    if curve.p0 == curve.p1 && curve.p0 == curve.p2 && curve.p0 == curve.p3 {
        return Ok(());
    }
    let magnitude = [curve.p0, curve.p1, curve.p2, curve.p3]
        .iter()
        .fold(0.0f64, |scale, point| {
            scale.max(point.x.abs()).max(point.y.abs())
        });
    if magnitude * (128.0 * f64::EPSILON) > tolerance * 0.25 {
        return Err(ValidationError::invalid(
            path,
            "geometry",
            "Device-space curve arithmetic cannot resolve the requested tolerance",
        ));
    }
    let first = (curve.p2 - curve.p1) - (curve.p1 - curve.p0);
    let second = (curve.p3 - curve.p2) - (curve.p2 - curve.p1);
    let curvature = libm::hypot(first.x, first.y).max(libm::hypot(second.x, second.y));
    let count = if curvature == 0.0 {
        1.0
    } else {
        (libm::ceil(libm::sqrt(0.75 * curvature / tolerance) * 0.5) * 2.0).max(2.0)
    };
    if !count.is_finite() || count > u64::MAX as f64 {
        return Err(ValidationError::limit(
            path,
            "max_work",
            "Wang subdivision count overflow",
        ));
    }
    budget.work(count as u64, path)?;
    let mut previous = Point::new(curve.p0.x, curve.p0.y);
    for index in 1..=count as u64 {
        let point = if index == count as u64 {
            curve.p3
        } else {
            curve.eval(index as f64 / count)
        };
        let current = Point::new(point.x, point.y);
        add_edge(edges, previous, current, budget, path)?;
        previous = current;
    }
    Ok(())
}

/// Appends a directed, nonhorizontal edge and charges its retained storage.
pub fn add_edge(
    edges: &mut Vec<Edge>,
    start: Point,
    end: Point,
    budget: &mut Budget,
    path: &str,
) -> Result<(), ValidationError> {
    if start.y != end.y {
        let edge = if start.y < end.y {
            Edge {
                top: start,
                bottom: end,
                winding: 1,
            }
        } else {
            Edge {
                top: end,
                bottom: start,
                winding: -1,
            }
        };
        budget.push(edges, edge, path)?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use kurbo::ParamCurveDeriv;

    #[test]
    fn wang_segments_follow_derivative_bound_and_device_tolerance() {
        let curves = [
            CubicBez::new((0.0, 0.0), (0.0, 8.0), (8.0, -8.0), (8.0, 0.0)),
            CubicBez::new((1.0, 2.0), (1.0, 2.0), (1.0, 2.0), (8.0, 9.0)),
            CubicBez::new((0.0, 0.0), (3.0, 1.0), (6.0, 2.0), (9.0, 3.0)),
            QuadBez::new((0.0, 0.0), (4.0, 8.0), (8.0, 1.0)).raise(),
        ];
        for curve in curves {
            for scale in [0.01, 1.0, 32.0] {
                let curve = CubicBez::new(
                    (curve.p0.x * scale, curve.p0.y * scale),
                    (curve.p1.x * scale, curve.p1.y * scale),
                    (curve.p2.x * scale, curve.p2.y * scale),
                    (curve.p3.x * scale, curve.p3.y * scale),
                );
                let derivative = curve.deriv().deriv();
                let bound = derivative
                    .eval(0.0)
                    .to_vec2()
                    .hypot()
                    .max(derivative.eval(1.0).to_vec2().hypot());
                for tolerance in [0.001, 0.125, 0.5, 1.0] {
                    let count = if bound == 0.0 {
                        1
                    } else {
                        let required =
                            libm::ceil(libm::sqrt(bound / (8.0 * tolerance))).max(1.0) as u64;
                        required + required % 2
                    };
                    let mut budget =
                        Budget::new(PixelSize::new(1, 1), RasterLimits::default()).unwrap();
                    let mut actual = Vec::new();
                    flatten_curve(&mut actual, curve, tolerance, &mut budget, "test.wang").unwrap();
                    let mut expected = Vec::new();
                    for segment in 0..count {
                        let start = curve.eval(segment as f64 / count as f64);
                        let end = curve.eval((segment + 1) as f64 / count as f64);
                        if start.y < end.y {
                            expected.push((
                                Point::new(start.x, start.y),
                                Point::new(end.x, end.y),
                                1,
                            ));
                        } else if start.y > end.y {
                            expected.push((
                                Point::new(end.x, end.y),
                                Point::new(start.x, start.y),
                                -1,
                            ));
                        }
                        for sample in 0..=64 {
                            let progress = f64::from(sample) / 64.0;
                            let parameter = (segment as f64 + progress) / count as f64;
                            let chord = start.lerp(end, progress);
                            let error = (curve.eval(parameter) - chord).hypot();
                            assert!(
                                error <= tolerance * (1.0 + 1e-9),
                                "error {error}, tolerance {tolerance}, count {count}"
                            );
                        }
                    }
                    assert_eq!(
                        actual
                            .iter()
                            .map(|edge| (edge.top, edge.bottom, edge.winding))
                            .collect::<Vec<_>>(),
                        expected
                    );
                }
            }
        }
    }

    #[test]
    fn wang_reduction_preserves_precision_and_work_failures() {
        let curve = CubicBez::new((0.0, 0.0), (0.0, 8.0), (8.0, -8.0), (8.0, 0.0));
        let mut budget = Budget::new(
            PixelSize::new(1, 1),
            RasterLimits {
                max_work: 1,
                ..RasterLimits::default()
            },
        )
        .unwrap();
        let error =
            flatten_curve(&mut Vec::new(), curve, 0.125, &mut budget, "test.wang").unwrap_err();
        assert_eq!(error.field, "max_work");
        let curve = CubicBez::new(
            (1e13, 0.0),
            (1e13, 8.0),
            (1e13 + 8.0, -8.0),
            (1e13 + 8.0, 0.0),
        );
        let mut budget = Budget::new(PixelSize::new(1, 1), RasterLimits::default()).unwrap();
        let error =
            flatten_curve(&mut Vec::new(), curve, 0.125, &mut budget, "test.wang").unwrap_err();
        assert_eq!(error.field, "geometry");
    }
}
