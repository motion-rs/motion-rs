//! Double-precision path values, with copy-on-write geometry.

use std::sync::{Arc, Mutex, OnceLock};

use kurbo::Shape as _;

use crate::{Pivot, Point, Rect, Transform, ValidationError, Vector2};

mod construction;
mod editing;
mod metrics;
mod morph;
#[cfg(test)]
mod tests;
mod validation;

pub use morph::MorphPath;

/// Maximum stored verbs in one path; mutations beyond it fail atomically.
pub const MAX_PATH_VERBS: usize = 16_384;

/// Maximum UTF-8 bytes accepted by one SVG path-data import.
pub const MAX_SVG_PATH_BYTES: usize = 1_048_576;

/// An absolute logical-coordinate path operation.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum PathVerb {
    /// Starts a subpath at a point without joining the previous subpath.
    MoveTo(Point),
    /// Adds a line from the current point.
    LineTo(Point),
    /// Adds a quadratic Bezier with one control and an endpoint.
    QuadraticTo(Point, Point),
    /// Adds a cubic Bezier with two controls and an endpoint.
    CubicTo(Point, Point, Point),
    /// Closes to the subpath's first point; subsequent segments resume there.
    Close,
}

impl PathVerb {
    /// Returns endpoint/control coordinates for validation without flattening this verb.
    /// Unused entries are `None`; Close has no coordinates.
    pub fn points(self) -> [Option<Point>; 3] {
        match self {
            Self::MoveTo(point) | Self::LineTo(point) => [Some(point), None, None],
            Self::QuadraticTo(control, end) => [Some(control), Some(end), None],
            Self::CubicTo(first, second, end) => [Some(first), Some(second), Some(end)],
            Self::Close => [None, None, None],
        }
    }
}

/// A cheaply cloned, checked f64 path with lazily cached geometric bounds.
///
/// Chainable mutations retain their first diagnostic for compile-time validation.
/// The `try_` methods return failures immediately and leave the snapshot unchanged.
/// Bounds include move positions and curve extrema, not the curve control hull.
/// Empty paths have zero bounds at the origin. SVG path data imports through
/// [`Self::parse_svg`]. Arc lengths use cached bounded Gaussian integration;
/// partial curves retain their Bezier representation rather than flattening.
/// [`crate::Shape::stroke`] expands bounded curves into fill outlines;
/// Styled trim/dash share the same arc-length parameterization.
///
/// ```
/// use motion_core::prelude::*;
/// use motion_core::{
///     DisplayList, RasterLimits, ResourcePool, ValidatedDisplayList, MAX_PATH_VERBS,
///     MAX_STROKE_VERBS, MAX_SVG_PATH_BYTES, sample_offsets,
/// };
/// let mut path = Path2D::new();
/// path.move_to(Point::ORIGIN)
///     .quadratic_to(Point::new(10.0, 20.0), Point::new(20.0, 0.0))
///     .close();
/// assert_eq!(path.bounding_box(), Rect::new(0.0, 0.0, 20.0, 10.0));
/// let node = path
///     .fill(Color::from_hex(0x24a49b))
///     .fill_rule(FillRule::EvenOdd)
///     .at(Point::new(100.0, 80.0));
/// assert_eq!(
///     node.measure(ValidationLimits::default())?,
///     Rect::new(90.0, 75.0, 110.0, 85.0)
/// );
/// let list = node.compile(
///     Size::new(200.0, 160.0),
///     PixelSize::new(200, 160),
///     ValidationLimits::default(),
/// )?;
/// assert_eq!(list.commands.len(), 1);
/// assert!(matches!(
///     list.commands[0],
///     DrawCommand::Fill {
///         rule: FillRule::EvenOdd,
///         ..
///     }
/// ));
/// # Ok::<(), motion_core::ValidationError>(())
/// ```
#[derive(Clone, Debug, Default)]
pub struct Path2D {
    data: Arc<PathData>,
}

type StrokeCache = Arc<Mutex<Vec<([u64; 4], PathData)>>>;

#[derive(Clone, Debug, Default)]
struct PathData {
    error: Option<ValidationError>,
    verbs: Arc<Vec<PathVerb>>,
    hull: Option<Rect>,
    current: Option<Point>,
    start: Option<Point>,
    bounds: OnceLock<Rect>,
    fingerprint: OnceLock<u64>,
    metrics: OnceLock<Result<Arc<metrics::Metrics>, ValidationError>>,
    strokes: OnceLock<StrokeCache>,
}

impl PartialEq for Path2D {
    fn eq(&self, other: &Self) -> bool {
        self.verbs() == other.verbs() && self.data.error == other.data.error
    }
}

impl Path2D {
    /// Creates an empty chainable path without paint.
    pub fn new() -> Self {
        Self::default()
    }

    /// Returns the immutable absolute f64 verbs.
    pub fn verbs(&self) -> &[PathVerb] {
        &self.data.verbs
    }

    /// Returns cached f64 geometric bounds, retaining zero-area geometry.
    pub fn bounding_box(&self) -> Rect {
        *self.data.bounds.get_or_init(|| self.compute_bounds())
    }

    /// Maps all endpoints and controls without flattening or modifying this path.
    pub fn transformed(&self, transform: &Transform) -> Result<Self, ValidationError> {
        self.validate()?;
        if !transform.is_finite() {
            return Err(invalid("transform", "Path transform must be finite"));
        }
        let mut result = Self::new();
        for verb in self.verbs() {
            let mapped = match *verb {
                PathVerb::MoveTo(point) => PathVerb::MoveTo(transform.apply(point)),
                PathVerb::LineTo(point) => PathVerb::LineTo(transform.apply(point)),
                PathVerb::QuadraticTo(control, end) => {
                    PathVerb::QuadraticTo(transform.apply(control), transform.apply(end))
                }
                PathVerb::CubicTo(first, second, end) => PathVerb::CubicTo(
                    transform.apply(first),
                    transform.apply(second),
                    transform.apply(end),
                ),
                PathVerb::Close => PathVerb::Close,
            };
            result.push(mapped)?;
        }
        Ok(result)
    }

    /// Shares only the immutable verb snapshot, without retaining metric caches.
    /// Arc::make_mut on a returned handle cannot edit this path's snapshot.
    pub fn shared_verbs(&self) -> Arc<Vec<PathVerb>> {
        self.data.verbs.clone()
    }

    /// Bounds of a single nondegenerate four-edge axis-aligned closed contour.
    /// Curves, extra contours, retraced edges and nonrectangular paths return None.
    pub fn axis_aligned_rectangle(&self) -> Option<Rect> {
        if self.data.error.is_some() {
            return None;
        }
        let [
            PathVerb::MoveTo(first),
            PathVerb::LineTo(second),
            PathVerb::LineTo(third),
            PathVerb::LineTo(fourth),
            PathVerb::Close,
        ] = self.verbs()
        else {
            return None;
        };
        let horizontal = first.y == second.y
            && second.x == third.x
            && third.y == fourth.y
            && fourth.x == first.x;
        let vertical = first.x == second.x
            && second.y == third.y
            && third.x == fourth.x
            && fourth.y == first.y;
        if !(horizontal || vertical) || first.x == third.x || first.y == third.y {
            return None;
        }
        Some(Rect::new(
            first.x.min(third.x),
            first.y.min(third.y),
            first.x.max(third.x),
            first.y.max(third.y),
        ))
    }

    /// Stable content fingerprint shared by clones and invalidated by successful edits.
    /// This is a lookup hint, not a collision-free geometry identity.
    pub fn fingerprint(&self) -> u64 {
        *self.data.fingerprint.get_or_init(|| {
            let mut value = 14695981039346656037_u64;
            let mut write = |bytes: &[u8]| {
                for byte in bytes {
                    value = (value ^ u64::from(*byte)).wrapping_mul(1099511628211);
                }
            };
            write(&(self.verbs().len() as u64).to_le_bytes());
            for verb in self.verbs() {
                write(&[match verb {
                    PathVerb::MoveTo(_) => 0,
                    PathVerb::LineTo(_) => 1,
                    PathVerb::QuadraticTo(..) => 2,
                    PathVerb::CubicTo(..) => 3,
                    PathVerb::Close => 4,
                }]);
                for point in verb.points().into_iter().flatten() {
                    write(&point.x.to_bits().to_le_bytes());
                    write(&point.y.to_bits().to_le_bytes());
                }
            }
            value
        })
    }

    fn compute_bounds(&self) -> Rect {
        let Some(hull) = self.data.hull else {
            return Rect::default();
        };
        let normalize = |point: Point| {
            (
                if hull.width() == 0.0 {
                    0.0
                } else {
                    (point.x - hull.left) / hull.width()
                },
                if hull.height() == 0.0 {
                    0.0
                } else {
                    (point.y - hull.top) / hull.height()
                },
            )
        };
        let mut path = kurbo::BezPath::new();
        for verb in self.verbs() {
            match *verb {
                PathVerb::MoveTo(point) => path.move_to(normalize(point)),
                PathVerb::LineTo(point) => path.line_to(normalize(point)),
                PathVerb::QuadraticTo(control, end) => {
                    path.quad_to(normalize(control), normalize(end))
                }
                PathVerb::CubicTo(first, second, end) => {
                    path.curve_to(normalize(first), normalize(second), normalize(end))
                }
                PathVerb::Close => path.close_path(),
            }
        }
        let bounds = path.bounding_box();
        if ![bounds.x0, bounds.y0, bounds.x1, bounds.y1]
            .iter()
            .all(|value| value.is_finite())
        {
            return hull;
        }
        let first = hull.anchor(Pivot::Relative(
            bounds.x0.clamp(0.0, 1.0),
            bounds.y0.clamp(0.0, 1.0),
        ));
        let last = hull.anchor(Pivot::Relative(
            bounds.x1.clamp(0.0, 1.0),
            bounds.y1.clamp(0.0, 1.0),
        ));
        let mut result = Rect::new(first.x, first.y, last.x, last.y);
        for verb in self.verbs() {
            if let PathVerb::MoveTo(point) = verb {
                result = Rect::new(
                    result.left.min(point.x),
                    result.top.min(point.y),
                    result.right.max(point.x),
                    result.bottom.max(point.y),
                );
            }
        }
        result
    }

    pub(crate) fn cached_stroke(&self, key: [u64; 4]) -> Option<Path2D> {
        let mut entries = self.data.strokes.get()?.lock().ok()?;
        let index = entries.iter().position(|entry| entry.0 == key)?;
        let entry = entries.remove(index);
        let result = Self {
            data: Arc::new(entry.1.clone()),
        };
        entries.push(entry);
        Some(result)
    }

    pub(crate) fn retain_stroke(&self, key: [u64; 4], outline: &Path2D) {
        let bytes = |data: &PathData| {
            data.verbs.len() * std::mem::size_of::<PathVerb>()
                + std::mem::size_of::<([u64; 4], PathData)>()
        };
        let cost = bytes(&outline.data);
        if cost > 1024 * 1024 {
            return;
        }
        outline.bounding_box();
        outline.fingerprint();
        let mut snapshot = (*outline.data).clone();
        snapshot.metrics.take();
        snapshot.strokes.take();
        let Ok(mut entries) = self
            .data
            .strokes
            .get_or_init(|| Arc::new(Mutex::new(Vec::new())))
            .lock()
        else {
            return;
        };
        if entries.iter().any(|entry| entry.0 == key) {
            return;
        }
        while !entries.is_empty()
            && (entries.len() >= 4
                || entries.iter().map(|entry| bytes(&entry.1)).sum::<usize>() + cost > 1024 * 1024)
        {
            entries.remove(0);
        }
        if entries.try_reserve(1).is_ok() {
            entries.push((key, snapshot));
        }
    }
}

fn invalid(field: &str, message: &str) -> ValidationError {
    ValidationError::invalid("path2d", field, message)
}
