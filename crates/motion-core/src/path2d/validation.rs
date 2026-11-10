//! Checked verb appends that validate limits, ordering and finite coordinates.

use super::{MAX_PATH_VERBS, Path2D, PathVerb, invalid};
use crate::{Point, Rect, ValidationError};
use std::sync::Arc;

impl Path2D {
    pub(super) fn push(&mut self, verb: PathVerb) -> Result<(), ValidationError> {
        if self.verbs().len() >= MAX_PATH_VERBS {
            return Err(ValidationError::limit(
                "path2d",
                "verbs",
                "path exceeds 16384 verbs",
            ));
        }
        if !matches!(verb, PathVerb::MoveTo(_)) && self.data.current.is_none() {
            return Err(invalid(
                "verbs",
                "Path segments and close require move_to first",
            ));
        }
        let points: &[Point] = match &verb {
            PathVerb::MoveTo(point) | PathVerb::LineTo(point) => std::slice::from_ref(point),
            PathVerb::QuadraticTo(_, _) | PathVerb::CubicTo(_, _, _) => &[],
            PathVerb::Close => &[],
        };
        let controls = match verb {
            PathVerb::QuadraticTo(control, end) => [Some(control), Some(end), None],
            PathVerb::CubicTo(first, second, end) => [Some(first), Some(second), Some(end)],
            _ => [None, None, None],
        };
        let mut hull = self.data.hull;
        for point in points.iter().copied().chain(controls.into_iter().flatten()) {
            if !point.x.is_finite() || !point.y.is_finite() {
                return Err(invalid("coordinates", "Path coordinates must be finite"));
            }
            hull = Some(
                hull.map_or(Rect::new(point.x, point.y, point.x, point.y), |rect| {
                    Rect::new(
                        rect.left.min(point.x),
                        rect.top.min(point.y),
                        rect.right.max(point.x),
                        rect.bottom.max(point.y),
                    )
                }),
            );
        }
        if hull.is_some_and(|rect| !rect.width().is_finite() || !rect.height().is_finite()) {
            return Err(invalid("bounds", "Path control extents must remain finite"));
        }
        let data = Arc::make_mut(&mut self.data);
        match verb {
            PathVerb::MoveTo(point) => {
                data.current = Some(point);
                data.start = Some(point);
            }
            PathVerb::LineTo(end)
            | PathVerb::QuadraticTo(_, end)
            | PathVerb::CubicTo(_, _, end) => data.current = Some(end),
            PathVerb::Close => data.current = data.start,
        }
        data.hull = hull;
        data.bounds.take();
        data.fingerprint.take();
        data.metrics.take();
        data.strokes.take();
        Arc::make_mut(&mut data.verbs).push(verb);
        Ok(())
    }
}
