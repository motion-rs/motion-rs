//! Stroke styles and expansion of stroked paths into fill geometry.

use crate::{Path2D, PathVerb, Point, Rect, ValidationError};

/// Maximum input verbs in one path/curve stroke expansion.
pub const MAX_STROKE_VERBS: usize = 256;

/// Shape of the ends of an open stroke.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum LineCap {
    /// End at the endpoint without an extension.
    #[default]
    Butt,
    /// Add a semicircular end with radius half the width.
    Round,
    /// Extend the end by half the width.
    Square,
}

/// Connection between adjacent stroke segments.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum LineJoin {
    /// Extend edges to their intersection, beveling beyond the radius ratio.
    Miter {
        /// Finite miter-length to half-width ratio in 1..=1024.
        limit: f64,
    },
    /// Connect outer edges with a circular arc.
    Round,
    /// Connect outer edges with a straight segment.
    Bevel,
}

/// logical-unit stroke style, with explicit caps, joins and dash geometry.
///
/// Defaults to width one, butt caps and a miter join with limit four. Width must
/// be finite and nonnegative; zero emits no outline. This bounded first slice
/// accepts paths of at most MAX_STROKE_VERBS verbs. Curves expand directly into
/// fill outlines; Styled trim/dash use exact source-curve splitting first.
/// Nonzero widths and segment lengths require 1e-30..=1e30; coordinates require
/// absolute magnitude at most 1e30, and the width must exceed local rounding noise.
/// Round outlines use kurbo's cubic circular approximation, not exact circles.
/// Move-only and zero-length subpaths emit no dot, including with round caps.
///
/// ```
/// use motion_core::prelude::*;
/// use motion_core::{
///     DisplayList, RasterLimits, ResourcePool, ValidatedDisplayList, MAX_PATH_VERBS,
///     MAX_STROKE_VERBS, MAX_SVG_PATH_BYTES, sample_offsets,
/// };
/// let outline = Rect::new(1.0, 1.0, 7.0, 7.0)
///     .fill(Color::WHITE)
///     .stroke(2.0, Color::BLACK)
///     .stroke_style(StrokeStyle {
///         width: 2.0,
///         cap: LineCap::Round,
///         join: LineJoin::Round,
///     });
/// let list = Node::from(outline).opacity(0.5).compile(
///     Size::new(8.0, 8.0),
///     PixelSize::new(8, 8),
///     ValidationLimits::default(),
/// )?;
/// let frame = list;
/// # Ok::<(), motion_core::ValidationError>(())
/// ```
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct StrokeStyle {
    /// Full logical width; zero preserves layout but draws no stroke.
    pub width: f64,
    /// The same cap for both ends of each open subpath.
    pub cap: LineCap,
    /// Connection at each nondegenerate corner, including closed seams.
    pub join: LineJoin,
}

impl Default for StrokeStyle {
    fn default() -> Self {
        Self {
            width: 1.0,
            cap: LineCap::Butt,
            join: LineJoin::Miter { limit: 4.0 },
        }
    }
}

impl StrokeStyle {
    /// Validates the finite width and supported miter limit.
    pub fn validate(&self, path: &str) -> Result<(), ValidationError> {
        if !self.width.is_finite() || self.width < 0.0 {
            return Err(ValidationError::invalid(
                path,
                "width",
                "Stroke width must be finite and nonnegative",
            ));
        }
        if let LineJoin::Miter { limit } = self.join
            && (!limit.is_finite() || !(1.0..=1024.0).contains(&limit))
        {
            return Err(ValidationError::invalid(
                path,
                "miter_limit",
                "Require a finite miter limit in 1..=1024",
            ));
        }
        Ok(())
    }

    pub(crate) fn bounds(&self, bounds: Rect) -> Rect {
        let join = match self.join {
            LineJoin::Miter { limit } => limit,
            _ => 1.0,
        };
        let cap = if self.cap == LineCap::Square {
            std::f64::consts::SQRT_2
        } else {
            1.0
        };
        let margin = self.width * 0.5 * join.max(cap);
        Rect::new(
            bounds.left - margin,
            bounds.top - margin,
            bounds.right + margin,
            bounds.bottom + margin,
        )
    }

    /// Expands checked centerline geometry into a bounded filled outline.
    pub fn outline(&self, source: &Path2D, path: &str) -> Result<Path2D, ValidationError> {
        self.validate(path)?;
        source.validate()?;
        if source.verbs().len() > MAX_STROKE_VERBS {
            return Err(ValidationError::limit(
                path,
                "stroke_verbs",
                "stroke input exceeds 256 verbs",
            ));
        }
        let (join, limit) = match self.join {
            LineJoin::Miter { limit } => (0, limit.to_bits()),
            LineJoin::Round => (1, 0),
            LineJoin::Bevel => (2, 0),
        };
        let key = [self.width.to_bits(), self.cap as u64, join, limit];
        if let Some(outline) = source.cached_stroke(key) {
            return Ok(outline);
        }
        let unsupported = |message| ValidationError::unsupported(path, "stroke_geometry", message);
        let mut current = None;
        let mut start = None;
        let mut scale = self.width;
        let mut has_length = false;
        for verb in source.verbs() {
            for point in verb.points().into_iter().flatten() {
                scale = scale.max(point.x.abs()).max(point.y.abs());
                if point.x.abs() > 1e30 || point.y.abs() > 1e30 {
                    return Err(unsupported(
                        "Stroke coordinates exceed the supported numerical range",
                    ));
                }
            }
            let endpoint = match *verb {
                PathVerb::MoveTo(point) => {
                    start = Some(point);
                    current = None;
                    point
                }
                PathVerb::LineTo(point) => point,
                PathVerb::Close => start.expect("checked subpath"),
                PathVerb::QuadraticTo(_, end) | PathVerb::CubicTo(_, _, end) => end,
            };
            scale = scale.max(endpoint.x.abs()).max(endpoint.y.abs());
            if endpoint.x.abs() > 1e30 || endpoint.y.abs() > 1e30 {
                return Err(unsupported(
                    "Stroke coordinates exceed the supported numerical range",
                ));
            }
            if let Some(previous) = current {
                let previous: Point = previous;
                has_length |= endpoint != previous
                    || verb
                        .points()
                        .into_iter()
                        .flatten()
                        .any(|point| point != previous);
                let length = libm::hypot(endpoint.x - previous.x, endpoint.y - previous.y);
                if length != 0.0 && !(1e-30..=1e30).contains(&length) {
                    return Err(unsupported(
                        "Stroke segment length exceeds the supported numerical range",
                    ));
                }
            }
            current = Some(endpoint);
        }
        if self.width == 0.0 {
            return Ok(Path2D::new());
        }
        if !(1e-30..=1e30).contains(&self.width) || self.width < scale * (128.0 * f64::EPSILON) {
            return Err(unsupported(
                "Stroke width exceeds the supported numerical range or local precision",
            ));
        }
        if !has_length {
            return Ok(Path2D::new());
        }
        let cap = match self.cap {
            LineCap::Butt => kurbo::Cap::Butt,
            LineCap::Round => kurbo::Cap::Round,
            LineCap::Square => kurbo::Cap::Square,
        };
        let (join, limit) = match self.join {
            LineJoin::Miter { limit } => (kurbo::Join::Miter, limit),
            LineJoin::Round => (kurbo::Join::Round, 4.0),
            LineJoin::Bevel => (kurbo::Join::Bevel, 4.0),
        };
        let style = kurbo::Stroke::new(self.width)
            .with_caps(cap)
            .with_join(join)
            .with_miter_limit(limit);
        let point = |point: Point| kurbo::Point::new(point.x, point.y);
        let mut current = Point::ORIGIN;
        let mut start = current;
        let elements = source.verbs().iter().filter_map(|verb| {
            Some(match *verb {
                PathVerb::MoveTo(value) => {
                    current = value;
                    start = value;
                    kurbo::PathEl::MoveTo(point(value))
                }
                PathVerb::LineTo(value) => {
                    if value == current {
                        return None;
                    }
                    current = value;
                    kurbo::PathEl::LineTo(point(value))
                }
                PathVerb::QuadraticTo(control, end) => {
                    if control == current && end == current {
                        return None;
                    }
                    current = end;
                    kurbo::PathEl::QuadTo(point(control), point(end))
                }
                PathVerb::CubicTo(first, second, end) => {
                    if first == current && second == current && end == current {
                        return None;
                    }
                    current = end;
                    kurbo::PathEl::CurveTo(point(first), point(second), point(end))
                }
                PathVerb::Close => {
                    current = start;
                    kurbo::PathEl::ClosePath
                }
            })
        });
        let tolerance = 1e-3_f64
            .min(self.width * 1e-3)
            .max(scale * (128.0 * f64::EPSILON));
        let expanded = kurbo::stroke(elements, &style, &kurbo::StrokeOpts::default(), tolerance);
        let mut outline = Path2D::new();
        let point = |point: kurbo::Point| Point::new(point.x, point.y);
        for element in expanded.elements() {
            match *element {
                kurbo::PathEl::MoveTo(value) => outline.try_move_to(point(value))?,
                kurbo::PathEl::LineTo(value) => outline.try_line_to(point(value))?,
                kurbo::PathEl::QuadTo(control, end) => {
                    outline.try_quadratic_to(point(control), point(end))?
                }
                kurbo::PathEl::CurveTo(first, second, end) => {
                    outline.try_cubic_to(point(first), point(second), point(end))?
                }
                kurbo::PathEl::ClosePath => outline.try_close()?,
            }
        }
        source.retain_stroke(key, &outline);
        Ok(outline)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ValidationErrorKind;

    #[test]
    fn static_stroke_cache_reuses_verbs_and_invalidates_only_edited_snapshot()
    -> Result<(), ValidationError> {
        use std::sync::Arc;
        let mut source = Path2D::parse_svg("M0 0C5 12 15 -4 20 4")?;
        let style = StrokeStyle {
            width: 2.0,
            cap: LineCap::Round,
            join: LineJoin::Round,
        };
        let first = style.outline(&source, "first")?;
        let again = style.outline(&source.clone(), "again")?;
        assert!(Arc::ptr_eq(&first.shared_verbs(), &again.shared_verbs()));
        let original = source.clone();
        assert!(source.try_line_to(Point::new(f64::NAN, 1.0)).is_err());
        assert!(Arc::ptr_eq(
            &first.shared_verbs(),
            &style.outline(&source, "failed edit")?.shared_verbs()
        ));
        source.try_line_to(Point::new(25.0, 3.0))?;
        let edited = style.outline(&source, "edited")?;
        assert_ne!(first, edited);
        assert!(Arc::ptr_eq(
            &first.shared_verbs(),
            &style.outline(&original, "old snapshot")?.shared_verbs()
        ));
        let different = StrokeStyle {
            width: 3.0,
            ..style
        }
        .outline(&original, "style")?;
        assert_ne!(first, different);
        for width in 4..12 {
            StrokeStyle {
                width: f64::from(width),
                ..style
            }
            .outline(&original, "evict")?;
        }
        assert_eq!(style.outline(&original, "retry")?, first);
        assert!(
            StrokeStyle {
                width: f64::NAN,
                ..style
            }
            .outline(&original, "invalid")
            .is_err()
        );
        Ok(())
    }

    use crate::{
        Canvas, Color, DrawCommand, DrawContext, Drawable, Node, NodeModifiers, PixelSize, Shape,
        Size, ValidationLimits,
    };

    fn line() -> Path2D {
        let mut path = Path2D::new();
        path.try_move_to(Point::new(1.0, 1.0)).unwrap();
        path.try_line_to(Point::new(5.0, 1.0)).unwrap();
        path
    }

    #[test]
    fn source_and_outline_charge_exact_shared_structural_limits() {
        let drawable = line().stroke(2.0, Color::from_rgb_float(2.0, -0.1, 0.5));
        let recorded = DrawContext::default()
            .record(&drawable, ValidationLimits::default())
            .unwrap();
        let DrawCommand::Fill { path: geometry, .. } = &recorded[0] else {
            panic!("expected fill")
        };
        let limits = ValidationLimits {
            max_nodes: 1 + line().verbs().len() + 1 + geometry.verbs().len(),
            max_depth: 2,
        };
        assert_eq!(
            DrawContext::default().record(&drawable, limits).unwrap(),
            recorded
        );
        let error = DrawContext::default()
            .record(
                &drawable,
                ValidationLimits {
                    max_nodes: limits.max_nodes - 1,
                    ..limits
                },
            )
            .unwrap_err();
        assert_eq!(error.field, "max_nodes");
        let mut source = Path2D::new();
        source.try_move_to(Point::ORIGIN).unwrap();
        for index in 1..MAX_STROKE_VERBS {
            source.try_line_to(Point::new(index as f64, 0.0)).unwrap();
        }
        let style = StrokeStyle::default();
        assert!(style.outline(&source, "test").is_ok());
        let retained = source.clone();
        source.try_line_to(Point::new(256.0, 0.0)).unwrap();
        assert_eq!(
            style.outline(&source, "test").unwrap_err().kind,
            ValidationErrorKind::StructuralLimit
        );
        assert_eq!(retained.verbs().len(), MAX_STROKE_VERBS);
    }

    #[test]
    fn zero_width_and_degenerate_subpaths_are_empty_but_invalid_input_is_not_skipped() {
        for width in [0.0, 1.0] {
            let mut source = Path2D::new();
            source.try_move_to(Point::ORIGIN).unwrap();
            source.try_line_to(Point::ORIGIN).unwrap();
            source.try_close().unwrap();
            assert!(
                StrokeStyle {
                    width,
                    cap: LineCap::Round,
                    ..StrokeStyle::default()
                }
                .outline(&source, "test")
                .unwrap()
                .verbs()
                .is_empty()
            );
        }
        assert!(
            DrawContext::default()
                .record(
                    &line().stroke(0.0, Color::WHITE),
                    ValidationLimits::default()
                )
                .unwrap()
                .is_empty()
        );
        let invalid = Rect::new(4.0, 0.0, 2.0, 2.0).stroke(10.0, Color::WHITE);
        assert!(
            Node::from(invalid)
                .measure(ValidationLimits::default())
                .is_err()
        );
        for width in [-1.0, f64::NAN, f64::INFINITY] {
            assert_eq!(
                StrokeStyle {
                    width,
                    ..StrokeStyle::default()
                }
                .outline(&line(), "test")
                .unwrap_err()
                .kind,
                ValidationErrorKind::InvalidValue
            );
        }
        for limit in [0.0, 1025.0, f64::NAN, f64::INFINITY] {
            assert!(
                StrokeStyle {
                    width: 0.0,
                    join: LineJoin::Miter { limit },
                    ..StrokeStyle::default()
                }
                .outline(&line(), "test")
                .is_err()
            );
        }
        let mut curve = line();
        curve
            .try_quadratic_to(Point::new(6.0, 0.0), Point::new(7.0, 1.0))
            .unwrap();
        for width in [0.0, 2.0] {
            let node = Node::from(curve.clone().stroke(width, Color::WHITE)).opacity(0.0);
            assert!(
                node.compile(
                    Size::new(8.0, 8.0),
                    PixelSize::new(8, 8),
                    ValidationLimits::default()
                )
                .is_ok()
            );
        }
    }

    #[test]
    fn ignored_canvas_stroke_failures_poison_the_whole_recording() {
        struct Ignores;

        impl Drawable for Ignores {
            fn bounds(&self, _: &DrawContext) -> Rect {
                Rect::new(0.0, 0.0, 8.0, 8.0)
            }

            fn draw(
                &self,
                _: &DrawContext,
                canvas: &mut Canvas<'_>,
            ) -> Result<(), ValidationError> {
                let _ = canvas.stroke(
                    &line(),
                    &StrokeStyle {
                        width: -1.0,
                        ..StrokeStyle::default()
                    },
                    Color::WHITE,
                );
                let _ = canvas.fill_rect(Rect::new(0.0, 0.0, 1.0, 1.0), Color::WHITE);
                Ok(())
            }
        }
        assert_eq!(
            DrawContext::default()
                .record(&Ignores, ValidationLimits::default())
                .unwrap_err()
                .field,
            "width"
        );
    }

    #[test]
    fn close_resumption_and_multiple_subpaths_keep_their_own_origins() {
        let mut source = Path2D::from_rect(Rect::new(1.0, 1.0, 3.0, 3.0)).unwrap();
        source.try_line_to(Point::new(6.0, 1.0)).unwrap();
        let mut explicit = Path2D::from_rect(Rect::new(1.0, 1.0, 3.0, 3.0)).unwrap();
        explicit.try_move_to(Point::new(1.0, 1.0)).unwrap();
        explicit.try_line_to(Point::new(6.0, 1.0)).unwrap();
        let style = StrokeStyle::default();
        assert_eq!(
            style.outline(&source, "test").unwrap(),
            style.outline(&explicit, "test").unwrap()
        );
    }

    #[test]
    fn unsupported_numeric_ranges_fail_before_expansion_and_leave_source_reusable() {
        let style = StrokeStyle::default();
        for (start, end) in [
            (Point::ORIGIN, Point::new(1e-31, 0.0)),
            (Point::new(1e31, 0.0), Point::new(1e31, 1.0)),
            (Point::new(1e20, 0.0), Point::new(1e20, 1.0)),
        ] {
            let mut source = Path2D::new();
            source.try_move_to(start).unwrap();
            source.try_line_to(end).unwrap();
            let snapshot = source.clone();
            assert_eq!(
                style.outline(&source, "test").unwrap_err().kind,
                ValidationErrorKind::Unsupported
            );
            assert_eq!(source, snapshot);
        }
        for width in [1e-31, 1e31] {
            assert_eq!(
                StrokeStyle { width, ..style }
                    .outline(&line(), "test")
                    .unwrap_err()
                    .kind,
                ValidationErrorKind::Unsupported
            );
        }
        assert!(style.outline(&line(), "test").is_ok());
    }
}
