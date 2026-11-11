//! Deterministic path boolean operations over flattened polygons.

use std::collections::BTreeMap;
use std::mem::size_of;

use super::{Path2D, PathVerb};
use crate::budget::Budget;
use crate::{FillRule, PixelSize, Point, RasterLimits, ValidationError};

#[cfg(test)]
mod tests;

/// Maximum flattened edges accepted from both operands of one boolean operation.
pub const MAX_BOOLEAN_EDGES: usize = 8_192;

/// Relative tolerance used to snap intersections onto segment endpoints.
const SNAP: f64 = 1e-12;

/// Relative distance of the side probes from an edge, scaled by the operand extent.
const PROBE: f64 = 1e-9;

const LABEL: &str = "path_boolean";

/// Region combination applied by [`Path2D::boolean`].
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PathBooleanOp {
    /// Area covered by either operand.
    Union,
    /// Area covered by both operands.
    Intersection,
    /// Area covered by the first operand and not by the second.
    Difference,
    /// Area covered by exactly one operand.
    Xor,
}

impl PathBooleanOp {
    fn apply(self, first: bool, second: bool) -> bool {
        match self {
            Self::Union => first || second,
            Self::Intersection => first && second,
            Self::Difference => first && !second,
            Self::Xor => first != second,
        }
    }
}

/// Fill interpretation, curve precision and admission limits for a boolean operation.
#[derive(Clone, Copy, Debug)]
pub struct PathBooleanOptions {
    /// Rule that decides which side of each operand contour is inside.
    pub fill_rule: FillRule,
    /// Maximum curve flattening error in path units; finite and positive.
    pub tolerance: f64,
    /// Storage and work allowances for the whole operation.
    pub limits: RasterLimits,
}

impl Default for PathBooleanOptions {
    fn default() -> Self {
        Self {
            fill_rule: FillRule::NonZero,
            tolerance: 0.05,
            limits: RasterLimits::default(),
        }
    }
}

impl Path2D {
    /// Combines two filled paths into a new path of closed line-only contours.
    ///
    /// Curves are flattened to `options.tolerance` first, so the result is polygonal.
    /// Each result contour keeps the covered area on its left (counter-clockwise in
    /// y-up coordinates) and is meant to be filled with [`FillRule::NonZero`]. Open
    /// subpaths are implicitly closed. Empty or zero-area results are empty paths.
    /// The result depends only on the inputs, and degenerate or self-intersecting
    /// operands are interpreted through `options.fill_rule`.
    ///
    /// # Errors
    ///
    /// Fails for a non-finite or non-positive tolerance, more than
    /// [`MAX_BOOLEAN_EDGES`] flattened edges, or exceeded `options.limits`.
    ///
    /// ```
    /// use motion_core::{Path2D, PathBooleanOp, PathBooleanOptions, Rect};
    /// let left = Path2D::from_rect(Rect::new(0.0, 0.0, 10.0, 10.0))?;
    /// let right = Path2D::from_rect(Rect::new(5.0, 0.0, 15.0, 10.0))?;
    /// let overlap = left.boolean(&right, PathBooleanOp::Intersection, PathBooleanOptions::default())?;
    /// assert_eq!(overlap.bounding_box(), Rect::new(5.0, 0.0, 10.0, 10.0));
    /// # Ok::<(), motion_core::ValidationError>(())
    /// ```
    pub fn boolean(
        &self,
        other: &Self,
        op: PathBooleanOp,
        options: PathBooleanOptions,
    ) -> Result<Self, ValidationError> {
        if !options.tolerance.is_finite() || options.tolerance <= 0.0 {
            return Err(ValidationError::invalid(
                LABEL,
                "tolerance",
                "Tolerance must be finite and positive",
            ));
        }
        let mut budget = Budget::new(PixelSize::new(1, 1), options.limits)?;
        let mut first = Operand::flatten(self, options.tolerance, &mut budget)?;
        let second = Operand::flatten(other, options.tolerance, &mut budget)?;
        first.segments.extend_from_slice(&second.segments);
        if first.segments.len() > MAX_BOOLEAN_EDGES {
            return Err(ValidationError::limit(
                LABEL,
                "edges",
                "Flattened operands exceed the boolean edge limit",
            ));
        }
        let segments = first.segments;
        let contours = [first.contours, second.contours];
        let pieces = split_segments(&segments, &mut budget)?;
        let edges = unique_edges(pieces, &mut budget)?;
        let scale = extent(&segments);
        let probe = (scale * PROBE).max(f64::MIN_POSITIVE);
        let work = (edges.len() as u64).saturating_mul(segments.len() as u64 * 2);
        budget.work(work, LABEL)?;
        let mut kept = Vec::new();
        for (start, end) in edges {
            let middle = Point::new((start.x + end.x) * 0.5, (start.y + end.y) * 0.5);
            let (dx, dy) = (end.x - start.x, end.y - start.y);
            let length = libm::hypot(dx, dy);
            let normal = (-dy / length * probe, dx / length * probe);
            let side = |sign: f64| {
                let point = Point::new(middle.x + sign * normal.0, middle.y + sign * normal.1);
                op.apply(
                    contains(&contours[0], point, options.fill_rule),
                    contains(&contours[1], point, options.fill_rule),
                )
            };
            let (left, right) = (side(1.0), side(-1.0));
            if left != right {
                let edge = if left { (start, end) } else { (end, start) };
                budget.push(&mut kept, edge, LABEL)?;
            }
        }
        build_path(&kept, &mut budget)
    }
}

struct Operand {
    segments: Vec<(Point, Point)>,
    contours: Vec<Vec<Point>>,
}

impl Operand {
    fn flatten(
        path: &Path2D,
        tolerance: f64,
        budget: &mut Budget,
    ) -> Result<Self, ValidationError> {
        let mut bez = kurbo::BezPath::new();
        for verb in path.verbs() {
            match *verb {
                PathVerb::MoveTo(point) => bez.move_to((point.x, point.y)),
                PathVerb::LineTo(point) => bez.line_to((point.x, point.y)),
                PathVerb::QuadraticTo(control, end) => {
                    bez.quad_to((control.x, control.y), (end.x, end.y))
                }
                PathVerb::CubicTo(first, second, end) => {
                    bez.curve_to((first.x, first.y), (second.x, second.y), (end.x, end.y))
                }
                PathVerb::Close => bez.close_path(),
            }
        }
        let mut contours: Vec<Vec<Point>> = Vec::new();
        let mut count = 0usize;
        kurbo::flatten(bez, tolerance, |element| {
            count += 1;
            if count > MAX_BOOLEAN_EDGES * 2 {
                return;
            }
            match element {
                kurbo::PathEl::MoveTo(point) => contours.push(vec![Point::new(point.x, point.y)]),
                kurbo::PathEl::LineTo(point) => {
                    if let Some(contour) = contours.last_mut() {
                        contour.push(Point::new(point.x, point.y));
                    }
                }
                _ => {}
            }
        });
        if count > MAX_BOOLEAN_EDGES * 2 {
            return Err(ValidationError::limit(
                LABEL,
                "edges",
                "Flattened operand exceeds the boolean edge limit",
            ));
        }
        let mut segments = Vec::new();
        contours.retain_mut(|contour| {
            contour.dedup();
            while contour.len() > 1 && contour.first() == contour.last() {
                contour.pop();
            }
            contour.len() >= 3
        });
        for contour in &contours {
            for (index, start) in contour.iter().enumerate() {
                let end = contour[(index + 1) % contour.len()];
                budget.push(&mut segments, (*start, end), LABEL)?;
            }
        }
        Ok(Self { segments, contours })
    }
}

fn extent(segments: &[(Point, Point)]) -> f64 {
    segments
        .iter()
        .flat_map(|(start, end)| [start.x, start.y, end.x, end.y])
        .fold(0.0f64, |scale, value| scale.max(value.abs()))
        .max(1e-6)
}

/// Splits every segment at its intersections with every other segment.
fn split_segments(
    segments: &[(Point, Point)],
    budget: &mut Budget,
) -> Result<Vec<Vec<Point>>, ValidationError> {
    let count = segments.len() as u64;
    budget.work(count.saturating_mul(count), LABEL)?;
    budget.bytes(count.saturating_mul(size_of::<Vec<Point>>() as u64), LABEL)?;
    let mut splits: Vec<Vec<Point>> = vec![Vec::new(); segments.len()];
    for first in 0..segments.len() {
        for second in first + 1..segments.len() {
            let points = intersect(segments[first], segments[second]);
            for (point, on_first, on_second) in points.into_iter().flatten() {
                if on_first {
                    budget.push(&mut splits[first], point, LABEL)?;
                }
                if on_second {
                    budget.push(&mut splits[second], point, LABEL)?;
                }
            }
        }
    }
    let mut pieces = Vec::with_capacity(segments.len());
    for (&(start, end), mut points) in segments.iter().zip(splits) {
        let (dx, dy) = (end.x - start.x, end.y - start.y);
        let along = |point: &Point| (point.x - start.x) * dx + (point.y - start.y) * dy;
        points.retain(|point| *point != start && *point != end);
        points.sort_by(|left, right| along(left).total_cmp(&along(right)));
        points.dedup();
        let mut chain = Vec::with_capacity(points.len() + 2);
        chain.push(start);
        chain.extend(points);
        chain.push(end);
        pieces.push(chain);
    }
    Ok(pieces)
}

/// Returns up to two shared points with flags saying which segment must split there.
fn intersect(first: (Point, Point), second: (Point, Point)) -> [Option<(Point, bool, bool)>; 4] {
    let mut found = [None; 4];
    let (p, q) = (first.0, second.0);
    let r = (first.1.x - p.x, first.1.y - p.y);
    let s = (second.1.x - q.x, second.1.y - q.y);
    let reach = [
        (first.0.x.min(first.1.x), first.0.x.max(first.1.x)),
        (second.0.x.min(second.1.x), second.0.x.max(second.1.x)),
        (first.0.y.min(first.1.y), first.0.y.max(first.1.y)),
        (second.0.y.min(second.1.y), second.0.y.max(second.1.y)),
    ];
    if reach[0].1 < reach[1].0
        || reach[1].1 < reach[0].0
        || reach[2].1 < reach[3].0
        || reach[3].1 < reach[2].0
    {
        return found;
    }
    let denominator = r.0 * s.1 - r.1 * s.0;
    let scale = libm::hypot(r.0, r.1) * libm::hypot(s.0, s.1);
    let gap = (q.x - p.x, q.y - p.y);
    if denominator.abs() > SNAP * scale {
        let t = (gap.0 * s.1 - gap.1 * s.0) / denominator;
        let u = (gap.0 * r.1 - gap.1 * r.0) / denominator;
        if (-SNAP..=1.0 + SNAP).contains(&t) && (-SNAP..=1.0 + SNAP).contains(&u) {
            let point = if t <= SNAP {
                first.0
            } else if t >= 1.0 - SNAP {
                first.1
            } else if u <= SNAP {
                second.0
            } else if u >= 1.0 - SNAP {
                second.1
            } else {
                Point::new(p.x + t * r.0, p.y + t * r.1)
            };
            found[0] = Some((point, true, true));
        }
        return found;
    }
    for (slot, (point, on_first)) in [
        (second.0, true),
        (second.1, true),
        (first.0, false),
        (first.1, false),
    ]
    .into_iter()
    .enumerate()
    {
        let (base, direction) = if on_first { (p, r) } else { (q, s) };
        let length = libm::hypot(direction.0, direction.1);
        let offset = (point.x - base.x, point.y - base.y);
        let distance = (offset.0 * direction.1 - offset.1 * direction.0).abs() / length;
        let along = (offset.0 * direction.0 + offset.1 * direction.1) / (length * length);
        if distance <= SNAP * length.max(1.0) && along > 0.0 && along < 1.0 {
            found[slot] = Some((point, on_first, !on_first));
        }
    }
    found
}

/// Orders and deduplicates the undirected sub-edges of all split segments.
fn unique_edges(
    pieces: Vec<Vec<Point>>,
    budget: &mut Budget,
) -> Result<Vec<(Point, Point)>, ValidationError> {
    let mut keyed: Vec<([u64; 4], (Point, Point))> = Vec::new();
    for chain in pieces {
        for pair in chain.windows(2) {
            let (start, end) = (pair[0], pair[1]);
            if start == end {
                continue;
            }
            let (a, b) = (bits(start), bits(end));
            let key = if a <= b {
                [a[0], a[1], b[0], b[1]]
            } else {
                [b[0], b[1], a[0], a[1]]
            };
            budget.push(&mut keyed, (key, (start, end)), LABEL)?;
        }
    }
    keyed.sort_by_key(|entry| entry.0);
    keyed.dedup_by_key(|entry| entry.0);
    Ok(keyed.into_iter().map(|entry| entry.1).collect())
}

fn bits(point: Point) -> [u64; 2] {
    [(point.x + 0.0).to_bits(), (point.y + 0.0).to_bits()]
}

fn contains(contours: &[Vec<Point>], point: Point, rule: FillRule) -> bool {
    let mut winding = 0i32;
    for contour in contours {
        for (index, start) in contour.iter().enumerate() {
            let end = contour[(index + 1) % contour.len()];
            if (start.y <= point.y) == (end.y <= point.y) {
                continue;
            }
            let t = (point.y - start.y) / (end.y - start.y);
            if start.x + t * (end.x - start.x) > point.x {
                winding += if end.y > start.y { 1 } else { -1 };
            }
        }
    }
    match rule {
        FillRule::NonZero => winding != 0,
        FillRule::EvenOdd => winding % 2 != 0,
    }
}

/// Chains directed edges into closed contours and drops collinear vertices.
fn build_path(edges: &[(Point, Point)], budget: &mut Budget) -> Result<Path2D, ValidationError> {
    let mut outgoing: BTreeMap<[u64; 2], Vec<usize>> = BTreeMap::new();
    for (index, (start, _)) in edges.iter().enumerate() {
        outgoing.entry(bits(*start)).or_default().push(index);
    }
    let mut used = vec![false; edges.len()];
    let mut path = Path2D::new();
    for first in 0..edges.len() {
        if used[first] {
            continue;
        }
        used[first] = true;
        budget.work(1, LABEL)?;
        let origin = edges[first].0;
        let mut ring = vec![origin];
        let mut current = edges[first].1;
        while current != origin {
            ring.push(current);
            budget.work(1, LABEL)?;
            let next = outgoing
                .get(&bits(current))
                .and_then(|list| list.iter().copied().find(|index| !used[*index]));
            let Some(next) = next else { break };
            used[next] = true;
            current = edges[next].1;
        }
        let ring = simplify(ring);
        if ring.len() < 3 {
            continue;
        }
        path.try_move_to(ring[0])?;
        for point in &ring[1..] {
            path.try_line_to(*point)?;
        }
        path.try_close()?;
    }
    Ok(path)
}

fn simplify(ring: Vec<Point>) -> Vec<Point> {
    let mut points = ring;
    let mut index = 0;
    while points.len() >= 3 && index < points.len() {
        let count = points.len();
        let (before, here, after) = (
            points[(index + count - 1) % count],
            points[index],
            points[(index + 1) % count],
        );
        let a = (here.x - before.x, here.y - before.y);
        let b = (after.x - here.x, after.y - here.y);
        let cross = a.0 * b.1 - a.1 * b.0;
        let scale = libm::hypot(a.0, a.1) * libm::hypot(b.0, b.1);
        if cross.abs() <= SNAP * scale && a.0 * b.0 + a.1 * b.1 > 0.0 {
            points.remove(index);
        } else {
            index += 1;
        }
    }
    points
}
