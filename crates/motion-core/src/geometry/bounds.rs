//! Double-precision authoring dimensions, bounds and anchors.

use super::Point;

/// Double-precision dimensions without implicit clamping or pixel conversion.
///
/// Raw values, including negative animation overshoot, are preserved. Consumers
/// decide which dimensions are valid for their operation.
///
/// ```
/// use motion_core::prelude::*;
/// use motion_core::{
///     DisplayList, RasterLimits, ResourceStore, ValidatedDisplayList, MAX_PATH_VERBS,
///     MAX_STROKE_VERBS, MAX_SVG_PATH_BYTES, sample_offsets,
/// };
/// let size = Size::new(40.0, 30.0);
/// assert_eq!(size.to_rect(), Rect::new(0.0, 0.0, 40.0, 30.0));
/// assert_eq!(size.width, 40.0_f64);
/// ```
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Size {
    /// Horizontal extent in design coordinates.
    pub width: f64,
    /// Vertical extent in design coordinates.
    pub height: f64,
}

impl Size {
    /// Preserves the supplied dimensions without normalization.
    pub const fn new(width: f64, height: f64) -> Self {
        Self { width, height }
    }

    /// Returns bounds starting at the origin with these dimensions.
    pub fn to_rect(self) -> Rect {
        Rect::from_size(self)
    }
}

/// A local-space anchor used by authoring placement and transforms.
///
/// Relative coordinates are not clamped; explicit points need not lie inside
/// bounds. Baseline falls back to BottomLeft without text baseline metrics.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub enum Pivot {
    /// Upper-left corner.
    TopLeft,
    /// Midpoint of the upper edge.
    Top,
    /// Upper-right corner.
    TopRight,
    /// Midpoint of the left edge.
    Left,
    /// Bounds midpoint; the default authoring pivot.
    #[default]
    Center,
    /// Midpoint of the right edge.
    Right,
    /// Lower-left corner.
    BottomLeft,
    /// Midpoint of the lower edge.
    Bottom,
    /// Lower-right corner.
    BottomRight,
    /// Text baseline origin, or BottomLeft without baseline metrics.
    Baseline,
    /// Fractions of the horizontal and vertical bounds, without clamping.
    Relative(f64, f64),
    /// An explicit point in local coordinates.
    Point(Point),
}

/// Double-precision bounds specified by opposite corners, not width and height.
///
/// Coordinates are preserved, not sorted or clamped. Inverted or zero-area bounds
/// are empty. Positive inset shrinks; excessive inset produces inverted bounds.
/// Containment is half-open: left/top are included and right/bottom are excluded.
///
/// ```
/// use motion_core::prelude::*;
/// use motion_core::{
///     DisplayList, RasterLimits, ResourceStore, ValidatedDisplayList, MAX_PATH_VERBS,
///     MAX_STROKE_VERBS, MAX_SVG_PATH_BYTES, sample_offsets,
/// };
/// let bounds = Rect::new(10.0, 20.0, 40.0, 30.0);
/// assert_eq!(bounds.size(), Size::new(30.0, 10.0));
/// assert_eq!(bounds.center(), Point::new(25.0, 25.0));
/// assert!(bounds.contains(Point::new(10.0, 20.0)));
/// assert!(!bounds.contains(Point::new(40.0, 30.0)));
/// ```
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Rect {
    /// Horizontal coordinate of the first corner.
    pub left: f64,
    /// Vertical coordinate of the first corner.
    pub top: f64,
    /// Horizontal coordinate of the opposite corner.
    pub right: f64,
    /// Vertical coordinate of the opposite corner.
    pub bottom: f64,
}

impl Rect {
    /// Preserves two corners without sorting, clamping or changing their meaning.
    pub const fn new(left: f64, top: f64, right: f64, bottom: f64) -> Self {
        Self {
            left,
            top,
            right,
            bottom,
        }
    }

    /// Creates bounds from a top-left position and signed dimensions.
    pub fn from_origin_size(origin: Point, size: Size) -> Self {
        Self::new(
            origin.x,
            origin.y,
            origin.x + size.width,
            origin.y + size.height,
        )
    }

    /// Creates bounds around a center with signed dimensions.
    pub fn from_center_size(center: Point, size: Size) -> Self {
        Self::new(
            center.x - size.width * 0.5,
            center.y - size.height * 0.5,
            center.x + size.width * 0.5,
            center.y + size.height * 0.5,
        )
    }

    /// Creates bounds at the coordinate origin with these dimensions.
    pub fn from_size(size: Size) -> Self {
        Self::from_origin_size(Point::ORIGIN, size)
    }

    /// Returns the signed horizontal span; subtraction can overflow for extremes.
    pub fn width(self) -> f64 {
        self.right - self.left
    }

    /// Returns the signed vertical span; subtraction can overflow for extremes.
    pub fn height(self) -> f64 {
        self.bottom - self.top
    }

    /// Returns signed dimensions without normalization.
    pub fn size(self) -> Size {
        Size::new(self.width(), self.height())
    }

    /// Returns the midpoint without adding two unscaled large coordinates.
    pub fn center(self) -> Point {
        Point::new(
            self.left.midpoint(self.right),
            self.top.midpoint(self.bottom),
        )
    }

    /// Resolves an anchor without normalizing or clamping the stored bounds.
    ///
    /// Baseline uses the non-text BottomLeft fallback. Relative coordinates can
    /// extrapolate; invalid or unrepresentable arithmetic remains observable.
    pub fn anchor(self, pivot: Pivot) -> Point {
        fn relative(start: f64, end: f64, fraction: f64) -> f64 {
            match fraction {
                0.0 => start,
                1.0 => end,
                0.5 => start.midpoint(end),
                _ if start.is_sign_negative() != end.is_sign_negative() => {
                    let middle = start.midpoint(end);
                    if fraction < 0.5 {
                        start + (middle - start) * (fraction * 2.0)
                    } else {
                        end + (middle - end) * ((1.0 - fraction) * 2.0)
                    }
                }
                _ => start + (end - start) * fraction,
            }
        }
        let center = self.center();
        match pivot {
            Pivot::TopLeft => Point::new(self.left, self.top),
            Pivot::Top => Point::new(center.x, self.top),
            Pivot::TopRight => Point::new(self.right, self.top),
            Pivot::Left => Point::new(self.left, center.y),
            Pivot::Center => center,
            Pivot::Right => Point::new(self.right, center.y),
            Pivot::BottomLeft | Pivot::Baseline => Point::new(self.left, self.bottom),
            Pivot::Bottom => Point::new(center.x, self.bottom),
            Pivot::BottomRight => Point::new(self.right, self.bottom),
            Pivot::Relative(horizontal, vertical) => Point::new(
                relative(self.left, self.right, horizontal),
                relative(self.top, self.bottom, vertical),
            ),
            Pivot::Point(point) => point,
        }
    }

    /// Whether all four stored coordinates are finite; spans may still overflow.
    pub fn is_finite(self) -> bool {
        [self.left, self.top, self.right, self.bottom]
            .iter()
            .all(|value| value.is_finite())
    }

    /// Whether a finite rectangle has no positive area, including inverted bounds.
    /// Nonfinite coordinates are invalid, not a usable empty rectangle.
    pub fn is_empty(self) -> bool {
        self.is_finite() && (self.right <= self.left || self.bottom <= self.top)
    }

    /// Tests a finite point with half-open edge ownership. Invalid bounds reject it.
    pub fn contains(self, point: Point) -> bool {
        self.is_finite()
            && point.x.is_finite()
            && point.y.is_finite()
            && point.x >= self.left
            && point.x < self.right
            && point.y >= self.top
            && point.y < self.bottom
    }

    /// Returns enclosing bounds, ignoring empty operands. If both are empty,
    /// returns the second operand. A nonfinite operand is propagated unchanged.
    pub fn union(self, other: Self) -> Self {
        if !self.is_finite() {
            return self;
        }
        if !other.is_finite() {
            return other;
        }
        if self.is_empty() {
            return other;
        }
        if other.is_empty() {
            return self;
        }
        Self::new(
            self.left.min(other.left),
            self.top.min(other.top),
            self.right.max(other.right),
            self.bottom.max(other.bottom),
        )
    }

    /// Returns overlapping bounds, clamping each disjoint span to zero at the
    /// greater origin. A nonfinite operand is propagated unchanged.
    pub fn intersect(self, other: Self) -> Self {
        if !self.is_finite() {
            return self;
        }
        if !other.is_finite() {
            return other;
        }
        let left = self.left.max(other.left);
        let top = self.top.max(other.top);
        Self::new(
            left,
            top,
            self.right.min(other.right).max(left),
            self.bottom.min(other.bottom).max(top),
        )
    }

    /// Moves every edge inward by a signed amount; negative values expand.
    /// Excessive positive amounts preserve inverted, empty bounds.
    pub fn inset(self, amount: f64) -> Self {
        Self::new(
            self.left + amount,
            self.top + amount,
            self.right - amount,
            self.bottom - amount,
        )
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn pivot_anchors_cover_corners_edges_center_and_nontext_baseline() {
        use super::{Pivot, Rect};
        use crate::Point;
        let bounds = Rect::new(10.0, 20.0, 50.0, 80.0);
        for (pivot, expected) in [
            (Pivot::TopLeft, (10.0, 20.0)),
            (Pivot::Top, (30.0, 20.0)),
            (Pivot::TopRight, (50.0, 20.0)),
            (Pivot::Left, (10.0, 50.0)),
            (Pivot::Center, (30.0, 50.0)),
            (Pivot::Right, (50.0, 50.0)),
            (Pivot::BottomLeft, (10.0, 80.0)),
            (Pivot::Bottom, (30.0, 80.0)),
            (Pivot::BottomRight, (50.0, 80.0)),
            (Pivot::Baseline, (10.0, 80.0)),
        ] {
            assert_eq!(bounds.anchor(pivot), Point::new(expected.0, expected.1));
        }
        assert_eq!(Pivot::default(), Pivot::Center);
        assert_eq!(
            bounds.anchor(Pivot::Relative(-0.25, 1.5)),
            Point::new(0.0, 110.0)
        );
        assert_eq!(
            bounds.anchor(Pivot::Point(Point::new(-8.0, 5.0))),
            Point::new(-8.0, 5.0)
        );
    }

    #[test]
    fn pivot_relative_positions_preserve_extreme_midpoints_and_subnormals() {
        use super::{Pivot, Rect};
        use crate::Point;
        let wide = Rect::new(-f64::MAX, 0.0, f64::MAX, f64::from_bits(1));
        assert_eq!(
            wide.anchor(Pivot::Relative(0.5, 1.0)),
            Point::new(0.0, f64::from_bits(1))
        );
        assert_eq!(wide.anchor(Pivot::Relative(0.25, 0.0)).x, -f64::MAX * 0.5);
        let tiny = f64::from_bits(1);
        let equal = Rect::new(tiny, tiny, tiny, tiny);
        assert_eq!(
            equal.anchor(Pivot::Relative(0.5, 0.5)),
            Point::new(tiny, tiny)
        );
        let inverted = Rect::new(50.0, 80.0, 10.0, 20.0);
        assert_eq!(inverted.anchor(Pivot::TopLeft), Point::new(50.0, 80.0));
        assert_eq!(
            inverted.anchor(Pivot::Relative(0.25, 0.5)),
            Point::new(40.0, 50.0)
        );
    }

    use super::*;
    use crate::{Interpolate, Rect, Size};

    #[test]
    fn constructors_use_opposite_corners_and_preserve_double_precision() {
        let bounds = Rect::new(10.0, 20.0, 40.0, 30.0);
        assert_eq!(bounds.size(), Size::new(30.0, 10.0));
        assert_eq!(
            Rect::from_origin_size(Point::new(10.0, 20.0), bounds.size()),
            bounds
        );
        assert_eq!(
            Rect::from_center_size(Point::new(25.0, 25.0), bounds.size()),
            bounds
        );
        assert_eq!(bounds.size().to_rect(), Rect::new(0.0, 0.0, 30.0, 10.0));
        let origin = 1_000_000_000.0;
        let precise = Rect::new(origin, 0.0, origin + 0.25, 0.5);
        assert_eq!(precise.width(), 0.25);
        assert_eq!(precise.center(), Point::new(origin + 0.125, 0.25));
        assert_eq!(precise.left as f32, precise.right as f32);
        assert_eq!(Size::new(-1.0, 2.0).to_rect().width(), -1.0);
    }

    #[test]
    fn containment_intersection_union_and_inset_have_explicit_empty_edges() {
        let bounds = Rect::new(10.0, 20.0, 40.0, 30.0);
        assert!(bounds.contains(Point::new(10.0, 20.0)));
        assert!(!bounds.contains(Point::new(40.0, 20.0)));
        assert!(!bounds.contains(Point::new(10.0, 30.0)));
        assert!(!bounds.contains(Point::new(f64::NAN, 25.0)));
        let adjacent = Rect::new(40.0, 20.0, 50.0, 30.0);
        assert_eq!(
            bounds.intersect(adjacent),
            Rect::new(40.0, 20.0, 40.0, 30.0)
        );
        assert!(bounds.intersect(adjacent).is_empty());
        assert_eq!(bounds.union(adjacent), Rect::new(10.0, 20.0, 50.0, 30.0));
        assert_eq!(bounds.union(Rect::default()), bounds);
        assert_eq!(Rect::default().union(bounds), bounds);
        assert_eq!(bounds.inset(2.0), Rect::new(12.0, 22.0, 38.0, 28.0));
        assert_eq!(bounds.inset(-2.0), Rect::new(8.0, 18.0, 42.0, 32.0));
        let inverted = bounds.inset(20.0);
        assert!(inverted.is_empty());
        assert!(!inverted.contains(inverted.center()));
        assert!(bounds.intersect(inverted).is_empty());
        assert_eq!(bounds.union(inverted), bounds);
        assert_eq!(
            bounds.intersect(Rect::new(50.0, 50.0, 60.0, 60.0)),
            Rect::new(50.0, 50.0, 50.0, 50.0)
        );
    }

    #[test]
    fn extreme_midpoints_remain_finite_and_nonfinite_bounds_are_not_silently_discarded() {
        let large = Rect::new(1e308, -1.4e308, 1.4e308, -1e308);
        assert_eq!(large.center(), Point::new(1.2e308, -1.2e308));
        assert_eq!(
            Rect::new(-f64::MAX, 0.0, f64::MAX, 2.0).center(),
            Point::new(0.0, 1.0)
        );
        let smallest = f64::from_bits(1);
        assert_eq!(
            Rect::new(smallest, smallest, smallest, smallest).center(),
            Point::new(smallest, smallest)
        );
        assert_eq!(
            Rect::new(smallest, smallest, smallest * 2.0, smallest * 2.0).center(),
            Point::new(smallest * 2.0, smallest * 2.0)
        );
        let valid = Rect::new(0.0, 0.0, 10.0, 10.0);
        for invalid in [
            Rect::new(f64::NAN, 0.0, 10.0, 10.0),
            Rect::new(0.0, 0.0, f64::INFINITY, 10.0),
        ] {
            assert!(!invalid.is_finite());
            assert!(!invalid.is_empty());
            assert!(!invalid.contains(Point::ORIGIN));
            assert!(!valid.union(invalid).is_finite());
            assert!(!invalid.union(valid).is_finite());
            assert!(!valid.intersect(invalid).is_finite());
            assert!(!invalid.intersect(valid).is_finite());
        }
    }

    #[test]
    fn interpolation_retains_f64_progress_and_signed_extrapolated_values() {
        let progress = 0.500_000_000_1;
        let size = Size::default().interpolate(&Size::new(1.0, 2.0), progress);
        assert_eq!(size, Size::new(progress, 2.0 * progress));
        assert_ne!(size.width, f64::from(progress as f32));
        let bounds = Rect::default().interpolate(&Rect::new(1.0, 2.0, 3.0, 4.0), progress);
        assert_eq!(
            bounds,
            Rect::new(progress, 2.0 * progress, 3.0 * progress, 4.0 * progress)
        );
        let inverted = Rect::new(0.0, 0.0, 10.0, 20.0).interpolate(&Rect::default(), 2.0);
        assert_eq!(inverted.size(), Size::new(-10.0, -20.0));
        assert!(inverted.is_empty());
        assert_eq!(
            Size::new(10.0, 20.0).interpolate(&Size::default(), 2.0),
            Size::new(-10.0, -20.0)
        );
    }
}
