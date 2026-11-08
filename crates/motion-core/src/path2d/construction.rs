//! Path constructors for rectangles, polylines and SVG path-data import.

use super::{MAX_PATH_VERBS, MAX_SVG_PATH_BYTES, Path2D, PathVerb, invalid};
use crate::{Point, Rect, ValidationError, Vector2};

impl Path2D {
    /// Builds a closed rectangle, rejecting invalid bounds before any mutation.
    pub fn from_rect(bounds: Rect) -> Result<Self, ValidationError> {
        crate::drawing::validate_rect(bounds, "path2d", "bounds")?;
        let mut path = Self::new();
        path.try_move_to(Point::new(bounds.left, bounds.top))?;
        path.try_line_to(Point::new(bounds.right, bounds.top))?;
        path.try_line_to(Point::new(bounds.right, bounds.bottom))?;
        path.try_line_to(Point::new(bounds.left, bounds.bottom))?;
        path.try_close()?;
        Ok(path)
    }

    /// Builds a bounded polyline, retaining invalid-input diagnostics for compilation.
    pub fn polyline(points: impl IntoIterator<Item = Point>) -> Self {
        let mut path = Self::new();
        for (index, point) in points.into_iter().take(MAX_PATH_VERBS + 1).enumerate() {
            if index == 0 {
                path.move_to(point);
            } else {
                path.line_to(point);
            }
            if path.data.error.is_some() {
                break;
            }
        }
        path
    }

    /// Imports SVG path data, not an XML document, without rasterizing geometry.
    ///
    /// Supports absolute/relative M/L/H/V/C/S/Q/T/A/Z, including repeated argument
    /// groups and reflected smooth controls. Empty data produces an empty path.
    /// A/a arcs become bounded cubic segments; zero radii become lines and
    /// coincident endpoints emit nothing. Syntax, nonfinite
    /// arithmetic and overflowing control extents return InvalidValue; input
    /// beyond MAX_SVG_PATH_BYTES or MAX_PATH_VERBS returns StructuralLimit.
    /// Parsed commands and expanded verbs have independent MAX_PATH_VERBS caps. Errors
    /// discard the entire result; there is no partial-path recovery, I/O or cache.
    ///
    /// ```
    /// use motion_core::prelude::*;
    /// use motion_core::{
    ///     DisplayList, RasterLimits, ResourcePool, ValidatedDisplayList, MAX_PATH_VERBS,
    ///     MAX_STROKE_VERBS, MAX_SVG_PATH_BYTES, sample_offsets,
    /// };
    /// let path = Path2D::parse_svg("M1 1h6v6H1z")?;
    /// assert_eq!(path, Path2D::from_rect(Rect::new(1.0, 1.0, 7.0, 7.0))?);
    /// let frame = Node::from(path.fill(Color::WHITE)).compile(
    ///     Size::new(8.0, 8.0),
    ///     PixelSize::new(8, 8),
    ///     ValidationLimits::default(),
    /// )?;
    /// # Ok::<(), motion_core::ValidationError>(())
    /// ```
    pub fn parse_svg(path_data: &str) -> Result<Self, ValidationError> {
        if path_data.len() > MAX_SVG_PATH_BYTES {
            return Err(ValidationError::limit(
                "path2d.svg",
                "svg_bytes",
                "SVG path data exceeds 1048576 bytes",
            ));
        }
        let mut result = Self::new();
        let mut previous_cubic = false;
        let mut previous_quadratic = false;
        for (index, segment) in svgtypes::PathParser::from(path_data).enumerate() {
            let path = format!("path2d.svg.commands[{index}]");
            if index >= MAX_PATH_VERBS {
                return Err(ValidationError::limit(
                    path,
                    "commands",
                    "SVG path data exceeds 16384 commands",
                ));
            }
            let segment = segment
                .map_err(|error| ValidationError::invalid(&path, "data", error.to_string()))?;
            let current = result.data.current.unwrap_or(Point::ORIGIN);
            let absolute = segment.is_abs();
            let point = |x, y| {
                if absolute {
                    Point::new(x, y)
                } else {
                    Point::new(current.x + x, current.y + y)
                }
            };
            let reflect = |control: Point| {
                Point::new(
                    current.x + (current.x - control.x),
                    current.y + (current.y - control.y),
                )
            };
            let verb =
                match segment {
                    svgtypes::PathSegment::MoveTo { x, y, .. } => PathVerb::MoveTo(point(x, y)),
                    svgtypes::PathSegment::LineTo { x, y, .. } => PathVerb::LineTo(point(x, y)),
                    svgtypes::PathSegment::HorizontalLineTo { x, .. } => PathVerb::LineTo(
                        Point::new(if absolute { x } else { current.x + x }, current.y),
                    ),
                    svgtypes::PathSegment::VerticalLineTo { y, .. } => PathVerb::LineTo(
                        Point::new(current.x, if absolute { y } else { current.y + y }),
                    ),
                    svgtypes::PathSegment::CurveTo {
                        x1,
                        y1,
                        x2,
                        y2,
                        x,
                        y,
                        ..
                    } => PathVerb::CubicTo(point(x1, y1), point(x2, y2), point(x, y)),
                    svgtypes::PathSegment::SmoothCurveTo { x2, y2, x, y, .. } => {
                        let control = match result.verbs().last() {
                            Some(PathVerb::CubicTo(_, previous, _)) if previous_cubic => {
                                reflect(*previous)
                            }
                            _ => current,
                        };
                        PathVerb::CubicTo(control, point(x2, y2), point(x, y))
                    }
                    svgtypes::PathSegment::Quadratic { x1, y1, x, y, .. } => {
                        PathVerb::QuadraticTo(point(x1, y1), point(x, y))
                    }
                    svgtypes::PathSegment::SmoothQuadratic { x, y, .. } => {
                        let control = match result.verbs().last() {
                            Some(PathVerb::QuadraticTo(previous, _)) if previous_quadratic => {
                                reflect(*previous)
                            }
                            _ => current,
                        };
                        PathVerb::QuadraticTo(control, point(x, y))
                    }
                    svgtypes::PathSegment::ClosePath { .. } => PathVerb::Close,
                    svgtypes::PathSegment::EllipticalArc {
                        rx,
                        ry,
                        x_axis_rotation,
                        large_arc,
                        sweep,
                        x,
                        y,
                        ..
                    } => {
                        let converted = (|| -> Result<(), ValidationError> {
                            if result.data.current.is_none() {
                                return Err(invalid("data", "SVG arcs require a current point"));
                            }
                            if ![rx, ry, x_axis_rotation, x, y]
                                .iter()
                                .all(|value| value.is_finite())
                            {
                                return Err(invalid("data", "SVG arc parameters must be finite"));
                            }
                            let end = point(x, y);
                            let arc = kurbo::SvgArc {
                                from: kurbo::Point::new(current.x, current.y),
                                to: kurbo::Point::new(end.x, end.y),
                                radii: kurbo::Vec2::new(rx.abs(), ry.abs()),
                                x_rotation: x_axis_rotation.to_radians(),
                                large_arc,
                                sweep,
                            };
                            if current != end {
                                if let Some(arc) = kurbo::Arc::from_svg_arc(&arc) {
                                    let mut converted = Self::new();
                                    converted.elliptical_arc(
                                        Point::new(arc.center.x, arc.center.y),
                                        Vector2::new(arc.radii.x, arc.radii.y),
                                        arc.start_angle,
                                        arc.sweep_angle,
                                        arc.x_rotation,
                                    )?;
                                    for (index, verb) in
                                        converted.verbs().iter().copied().enumerate().skip(1)
                                    {
                                        let verb = if index + 1 == converted.verbs().len() {
                                            match verb {
                                                PathVerb::CubicTo(first, second, _) => {
                                                    PathVerb::CubicTo(first, second, end)
                                                }
                                                _ => verb,
                                            }
                                        } else {
                                            verb
                                        };
                                        result.push(verb)?;
                                    }
                                } else {
                                    result.try_line_to(end)?;
                                }
                            }
                            Ok(())
                        })();
                        converted.map_err(|mut error| {
                            error.path = path;
                            error
                        })?;
                        previous_cubic = false;
                        previous_quadratic = false;
                        continue;
                    }
                };
            previous_cubic = matches!(verb, PathVerb::CubicTo(..));
            previous_quadratic = matches!(verb, PathVerb::QuadraticTo(..));
            result.push(verb).map_err(|mut error| {
                error.path = path;
                error
            })?;
        }
        Ok(result)
    }
}
