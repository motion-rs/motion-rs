//! Arc-length path metrics for length, sampling, trimming, dashing and cubic intervals.

use kurbo::{ParamCurve, ParamCurveDeriv, PathSeg};

use super::*;

const BINS: usize = 16;

const MAX_WORK: usize = 16_777_216;

#[derive(Clone, Debug)]
pub(super) struct Metrics {
    segments: Vec<Segment>,
    total: f64,
}

#[derive(Clone, Debug)]
struct Segment {
    curve: PathSeg,
    start: f64,
    length: f64,
    table: [f64; BINS + 1],
    contour_start: f64,
    contour: usize,
    close: bool,
}

fn point(value: kurbo::Point) -> Point {
    Point::new(value.x, value.y)
}

fn subcurve(curve: PathSeg, lower: f64, upper: f64) -> PathSeg {
    if lower == 0.0 && upper == 1.0 {
        curve
    } else {
        let controls = match curve {
            PathSeg::Line(line) => [line.p0, line.p1, line.p1, line.p1],
            PathSeg::Quad(quad) => [quad.p0, quad.p1, quad.p2, quad.p2],
            PathSeg::Cubic(cubic) => [cubic.p0, cubic.p1, cubic.p2, cubic.p3],
        };
        let constant_x = controls.iter().all(|point| point.x == controls[0].x);
        let constant_y = controls.iter().all(|point| point.y == controls[0].y);
        let mut result = curve.subsegment(lower..upper);
        let preserve = |point: &mut kurbo::Point| {
            if constant_x {
                point.x = controls[0].x;
            }
            if constant_y {
                point.y = controls[0].y;
            }
        };
        match &mut result {
            PathSeg::Line(line) => {
                preserve(&mut line.p0);
                preserve(&mut line.p1);
            }
            PathSeg::Quad(quad) => {
                preserve(&mut quad.p0);
                preserve(&mut quad.p1);
                preserve(&mut quad.p2);
            }
            PathSeg::Cubic(cubic) => {
                preserve(&mut cubic.p0);
                preserve(&mut cubic.p1);
                preserve(&mut cubic.p2);
                preserve(&mut cubic.p3);
            }
        }
        result
    }
}

fn derivative(curve: PathSeg, parameter: f64) -> Vector2 {
    let value = match curve {
        PathSeg::Line(line) => line.deriv().eval(parameter),
        PathSeg::Quad(quad) => quad.deriv().eval(parameter),
        PathSeg::Cubic(cubic) => cubic.deriv().eval(parameter),
    };
    Vector2::new(value.x, value.y)
}

fn gauss(curve: PathSeg, start: f64, end: f64, work: &mut usize) -> Result<f64, ValidationError> {
    *work += 8;
    if *work > MAX_WORK {
        return Err(ValidationError::limit(
            "path2d.metrics",
            "work",
            "Arc-length integration exhausted its work allowance",
        ));
    }
    let midpoint = (start + end) * 0.5;
    let radius = (end - start) * 0.5;
    let mut sum = 0.0;
    for (node, weight) in [
        (0.1834346424956498, 0.362683783378362),
        (0.525532409916329, 0.3137066458778873),
        (0.7966664774136267, 0.2223810344533745),
        (0.9602898564975363, 0.1012285362903763),
    ] {
        sum += weight
            * (derivative(curve, midpoint - radius * node).length()
                + derivative(curve, midpoint + radius * node).length());
    }
    let length = sum * radius;
    if !length.is_finite() {
        return Err(invalid("length", "Arc length must remain finite"));
    }
    Ok(length)
}

fn integrate(
    curve: PathSeg,
    start: f64,
    end: f64,
    depth: usize,
    work: &mut usize,
) -> Result<f64, ValidationError> {
    if start == end {
        return Ok(0.0);
    }
    let midpoint = (start + end) * 0.5;
    let coarse = gauss(curve, start, end, work)?;
    let fine = gauss(curve, start, midpoint, work)? + gauss(curve, midpoint, end, work)?;
    let scale = match curve {
        PathSeg::Line(line) => point(line.p0).distance(point(line.p1)),
        PathSeg::Quad(quad) => {
            point(quad.p0).distance(point(quad.p1)) + point(quad.p1).distance(point(quad.p2))
        }
        PathSeg::Cubic(cubic) => {
            point(cubic.p0).distance(point(cubic.p1))
                + point(cubic.p1).distance(point(cubic.p2))
                + point(cubic.p2).distance(point(cubic.p3))
        }
    };
    if !scale.is_finite() {
        return Err(invalid("length", "Curve metric scale must remain finite"));
    }
    if fine == 0.0 && scale > 0.0 {
        return Err(invalid(
            "length",
            "Curve length is below integration precision",
        ));
    }
    if (fine - coarse).abs() <= fine * 1e-11 + scale * 1e-13 || fine == 0.0 {
        return Ok(fine);
    }
    if depth == 32 {
        return Err(ValidationError::limit(
            "path2d.metrics",
            "precision",
            "Arc-length integration could not meet its relative tolerance",
        ));
    }
    Ok(integrate(curve, start, midpoint, depth + 1, work)?
        + integrate(curve, midpoint, end, depth + 1, work)?)
}

impl Segment {
    fn parameter(&self, distance: f64) -> Result<f64, ValidationError> {
        if distance <= 0.0 {
            return Ok(0.0);
        }
        if distance >= self.length {
            return Ok(1.0);
        }
        if matches!(self.curve, PathSeg::Line(_)) {
            return Ok(distance / self.length);
        }
        let bin = self
            .table
            .partition_point(|value| *value <= distance)
            .saturating_sub(1)
            .min(BINS - 1);
        let origin = bin as f64 / BINS as f64;
        let mut lower = origin;
        let mut upper = (bin + 1) as f64 / BINS as f64;
        let mut parameter = lower
            + (upper - lower) * (distance - self.table[bin])
                / (self.table[bin + 1] - self.table[bin]);
        let mut work = 0;
        for _ in 0..48 {
            let error = self.table[bin] + integrate(self.curve, origin, parameter, 0, &mut work)?
                - distance;
            if error.abs() <= self.length * 2e-11 {
                return Ok(parameter);
            }
            if error < 0.0 {
                lower = parameter;
            } else {
                upper = parameter;
            }
            let candidate = parameter - error / derivative(self.curve, parameter).length();
            parameter = if candidate.is_finite() && candidate > lower && candidate < upper {
                candidate
            } else {
                (lower + upper) * 0.5
            };
        }
        Ok((lower + upper) * 0.5)
    }
}

impl Metrics {
    fn new(path: &Path2D) -> Result<Self, ValidationError> {
        let mut result = Self {
            segments: Vec::new(),
            total: 0.0,
        };
        let mut current = kurbo::Point::ZERO;
        let mut start = current;
        let mut contour = 0;
        let mut contour_start = 0.0;
        let mut work = 0;
        for verb in path.verbs() {
            let to_kurbo = |value: Point| kurbo::Point::new(value.x, value.y);
            let curve = match *verb {
                PathVerb::MoveTo(value) => {
                    current = to_kurbo(value);
                    start = current;
                    contour += 1;
                    contour_start = result.total;
                    continue;
                }
                PathVerb::LineTo(value) => {
                    PathSeg::Line(kurbo::Line::new(current, to_kurbo(value)))
                }
                PathVerb::QuadraticTo(control, end) => PathSeg::Quad(kurbo::QuadBez::new(
                    current,
                    to_kurbo(control),
                    to_kurbo(end),
                )),
                PathVerb::CubicTo(first, second, end) => PathSeg::Cubic(kurbo::CubicBez::new(
                    current,
                    to_kurbo(first),
                    to_kurbo(second),
                    to_kurbo(end),
                )),
                PathVerb::Close => PathSeg::Line(kurbo::Line::new(current, start)),
            };
            let mut table = [0.0; BINS + 1];
            let line_length = if let PathSeg::Line(line) = curve {
                Some(point(line.p0).distance(point(line.p1)))
            } else {
                None
            };
            for bin in 0..BINS {
                table[bin + 1] = if let Some(length) = line_length {
                    length * ((bin + 1) as f64 / BINS as f64)
                } else {
                    table[bin]
                        + integrate(
                            curve,
                            bin as f64 / BINS as f64,
                            (bin + 1) as f64 / BINS as f64,
                            0,
                            &mut work,
                        )?
                };
            }
            let length = table[BINS];
            result.segments.push(Segment {
                curve,
                start: result.total,
                length,
                table,
                contour_start,
                contour,
                close: matches!(verb, PathVerb::Close),
            });
            let total = result.total + length;
            if length > 0.0 && total == result.total {
                return Err(invalid(
                    "length",
                    "Segment length is below cumulative metric precision",
                ));
            }
            result.total = total;
            if !result.total.is_finite() {
                return Err(invalid("length", "Total path length must remain finite"));
            }
            current = curve.end();
        }
        Ok(result)
    }

    fn append(
        &self,
        start: f64,
        end: f64,
        output: &mut Path2D,
        connect: bool,
    ) -> Result<(), ValidationError> {
        let first = self
            .segments
            .partition_point(|segment| segment.start + segment.length <= start);
        let mut previous = None;
        for segment in &self.segments[first..] {
            if segment.start > end || (segment.start == end && segment.length > 0.0) {
                break;
            }
            if segment.length == 0.0 {
                if segment.close
                    && start <= segment.contour_start
                    && previous == Some(segment.contour)
                {
                    output.try_close()?;
                }
                continue;
            }
            let lower = segment.parameter((start - segment.start).max(0.0))?;
            let upper = segment.parameter((end - segment.start).min(segment.length))?;
            if lower >= upper {
                continue;
            }
            let curve = subcurve(segment.curve, lower, upper);
            let first_point = point(curve.start());
            if (previous != Some(segment.contour)
                && !(connect && previous.is_none() && output.data.current == Some(first_point)))
                || output.data.current != Some(first_point)
            {
                output.try_move_to(first_point)?;
            }
            if segment.close && lower == 0.0 && upper == 1.0 && start <= segment.contour_start {
                output.try_close()?;
            } else {
                match curve {
                    PathSeg::Line(line) => output.try_line_to(point(line.p1))?,
                    PathSeg::Quad(quad) => {
                        output.try_quadratic_to(point(quad.p1), point(quad.p2))?
                    }
                    PathSeg::Cubic(cubic) => {
                        output.try_cubic_to(point(cubic.p1), point(cubic.p2), point(cubic.p3))?
                    }
                }
            }
            previous = Some(segment.contour);
        }
        Ok(())
    }
}

impl Path2D {
    /// Returns normalized arc-length boundaries for curve morphing.
    pub fn cubic_breakpoints(&self) -> Result<Vec<f64>, ValidationError> {
        let metrics = self.metrics()?;
        let mut points = vec![0.0];
        if metrics.total > 0.0 {
            for segment in &metrics.segments {
                if segment.length > 0.0 {
                    let fraction = (segment.start + segment.length) / metrics.total;
                    if fraction > *points.last().unwrap() {
                        points.push(fraction.min(1.0));
                    }
                }
            }
        }
        if points.last() != Some(&1.0) {
            points.push(1.0);
        }
        Ok(points)
    }

    /// Extracts cubic representations of checked normalized arc-length intervals.
    pub fn cubic_intervals(&self, fractions: &[f64]) -> Result<Vec<[Point; 4]>, ValidationError> {
        if fractions.len() < 2
            || fractions.len() > MAX_PATH_VERBS
            || fractions
                .iter()
                .any(|value| !value.is_finite() || !(0.0..=1.0).contains(value))
            || fractions.windows(2).any(|pair| pair[0] >= pair[1])
        {
            return Err(invalid(
                "segments",
                "Cubic interval boundaries must be finite, increasing and bounded",
            ));
        }
        let metrics = self.metrics()?;
        if metrics.total == 0.0 {
            return Ok(vec![[self.try_point_at(0.0)?.0; 4]; fractions.len() - 1]);
        }
        let mut output = Vec::with_capacity(fractions.len() - 1);
        for pair in fractions.windows(2) {
            let middle = ((pair[0] + pair[1]) * 0.5) * metrics.total;
            let index = metrics
                .segments
                .partition_point(|segment| segment.start + segment.length <= middle);
            let segment = metrics
                .segments
                .get(index)
                .ok_or_else(|| invalid("segments", "Cubic interval has no measurable segment"))?;
            let start = pair[0] * metrics.total;
            let end = pair[1] * metrics.total;
            let slack = metrics.total * 2e-12;
            if start < segment.start - slack || end > segment.start + segment.length + slack {
                return Err(invalid(
                    "segments",
                    "Cubic interval crosses an original curve boundary",
                ));
            }
            let curve = subcurve(
                segment.curve,
                segment.parameter((start - segment.start).clamp(0.0, segment.length))?,
                segment.parameter((end - segment.start).clamp(0.0, segment.length))?,
            );
            output.push(match curve {
                PathSeg::Line(line) => {
                    let first = point(line.p0);
                    let last = point(line.p1);
                    [
                        first,
                        first.interpolate(&last, 1.0 / 3.0),
                        first.interpolate(&last, 2.0 / 3.0),
                        last,
                    ]
                }
                PathSeg::Quad(quad) => {
                    let cubic = quad.raise();
                    [
                        point(cubic.p0),
                        point(cubic.p1),
                        point(cubic.p2),
                        point(cubic.p3),
                    ]
                }
                PathSeg::Cubic(cubic) => [
                    point(cubic.p0),
                    point(cubic.p1),
                    point(cubic.p2),
                    point(cubic.p3),
                ],
            });
        }
        Ok(output)
    }

    fn metrics(&self) -> Result<&Metrics, ValidationError> {
        self.data
            .metrics
            .get_or_init(|| Metrics::new(self).map(Arc::new))
            .as_ref()
            .map(Arc::as_ref)
            .map_err(Clone::clone)
    }

    /// Returns cached total contour length, including explicit closing edges.
    /// Panics if integration overflows or exhausts its bounded precision/work.
    pub fn length(&self) -> f64 {
        self.try_length()
            .expect("Path arc-length calculation failed")
    }

    /// Checked arc length with bounded adaptive eight-point Gaussian integration.
    pub fn try_length(&self) -> Result<f64, ValidationError> {
        Ok(self.metrics()?.total)
    }

    /// Samples normalized arc distance and its unit forward tangent.
    /// Progress clamps to 0..=1; nonfinite inputs or metric failures panic.
    pub fn point_at(&self, progress: f64) -> (Point, Vector2) {
        self.try_point_at(progress)
            .expect("Path arc-length sampling failed")
    }

    /// Checked sampling. Empty/move-only paths have a zero tangent. At a cusp,
    /// uses the forward one-sided direction, or backward direction at the end.
    pub fn try_point_at(&self, progress: f64) -> Result<(Point, Vector2), ValidationError> {
        if !progress.is_finite() {
            return Err(invalid("progress", "Path progress must be finite"));
        }
        let metrics = self.metrics()?;
        if metrics.total == 0.0 {
            return Ok((
                self.verbs()
                    .iter()
                    .find_map(|verb| {
                        if let PathVerb::MoveTo(value) = verb {
                            Some(*value)
                        } else {
                            None
                        }
                    })
                    .unwrap_or(Point::ORIGIN),
                Vector2::ZERO,
            ));
        }
        let distance = progress.clamp(0.0, 1.0) * metrics.total;
        let segment = metrics
            .segments
            .iter()
            .find(|segment| segment.length > 0.0 && segment.start + segment.length > distance)
            .or_else(|| {
                metrics
                    .segments
                    .iter()
                    .rev()
                    .find(|segment| segment.length > 0.0)
            })
            .expect("positive total length");
        let parameter = segment.parameter((distance - segment.start).clamp(0.0, segment.length))?;
        let position = point(segment.curve.eval(parameter));
        let mut direction = derivative(segment.curve, parameter);
        if direction == Vector2::ZERO {
            direction = if parameter < 1.0 {
                point(segment.curve.eval((parameter + 1e-6).min(1.0))) - position
            } else {
                position - point(segment.curve.eval((parameter - 1e-6).max(0.0)))
            };
        }
        Ok((position, direction.normalize()))
    }

    /// Extracts a normalized arc-length interval, preserving Bezier segments.
    /// Clamps finite endpoints; empty/reversed intervals produce an empty path.
    /// Panics on invalid inputs or numerical/resource failure; see try_trim.
    pub fn trim(&self, start: f64, end: f64) -> Self {
        self.try_trim(start, end).expect("Path trimming failed")
    }

    /// Checked de Casteljau extraction. Full range preserves all original verbs.
    pub fn try_trim(&self, start: f64, end: f64) -> Result<Self, ValidationError> {
        if !start.is_finite() || !end.is_finite() {
            return Err(invalid("trim", "Trim endpoints must be finite"));
        }
        let start = start.clamp(0.0, 1.0);
        let end = end.clamp(0.0, 1.0);
        if start >= end {
            return Ok(Self::new());
        }
        if start == 0.0 && end == 1.0 {
            return Ok(self.clone());
        }
        let metrics = self.metrics()?;
        let mut output = Self::new();
        metrics.append(
            start * metrics.total,
            end * metrics.total,
            &mut output,
            false,
        )?;
        Ok(output)
    }

    /// Cuts positive finite dash/gap lengths in logical units. Odd patterns
    /// repeat twice; phase restarts per contour and positive offsets advance it.
    /// Empty patterns retain the path. At most 256 entries and 8192 visible
    /// intervals per contour are admitted; generated verbs remain bounded.
    pub fn dashed(&self, pattern: &[f64], offset: f64) -> Result<Self, ValidationError> {
        if !offset.is_finite()
            || pattern
                .iter()
                .any(|value| !value.is_finite() || *value <= 0.0)
        {
            return Err(invalid(
                "dash",
                "Dash lengths must be finite and positive; offset must be finite",
            ));
        }
        if pattern.len() > 256 {
            return Err(ValidationError::limit(
                "path2d",
                "dash",
                "Dash pattern exceeds 256 entries",
            ));
        }
        if pattern.is_empty() {
            return Ok(self.clone());
        }
        let count = pattern.len()
            * if pattern.len().is_multiple_of(2) {
                1
            } else {
                2
            };
        let cycle = pattern.iter().sum::<f64>() * if count == pattern.len() { 1.0 } else { 2.0 };
        if !cycle.is_finite() {
            return Err(invalid("dash", "Dash cycle must remain finite"));
        }
        let metrics = self.metrics()?;
        let mut output = Self::new();
        let mut first = 0;
        while first < metrics.segments.len() {
            let contour = metrics.segments[first].contour;
            let end_index = first
                + metrics.segments[first..].partition_point(|segment| segment.contour == contour);
            let last = &metrics.segments[end_index - 1];
            let origin = metrics.segments[first].start;
            let length = last.start + last.length - origin;
            let mut phase = offset.rem_euclid(cycle);
            let mut index = 0;
            while phase >= pattern[index % pattern.len()] {
                phase -= pattern[index % pattern.len()];
                index = (index + 1) % count;
            }
            let mut remaining = pattern[index % pattern.len()] - phase;
            let mut position = 0.0;
            let mut ranges = Vec::new();
            while position < length {
                let next = (position + remaining).min(length);
                if next <= position {
                    return Err(invalid(
                        "dash",
                        "Dash interval is below local floating-point precision",
                    ));
                }
                if index % 2 == 0 {
                    if ranges.len() >= MAX_PATH_VERBS / 2 {
                        return Err(ValidationError::limit(
                            "path2d",
                            "dash",
                            "Dash interval allowance exhausted",
                        ));
                    }
                    ranges.push((origin + position, origin + next));
                }
                position = next;
                index = (index + 1) % count;
                remaining = pattern[index % pattern.len()];
            }
            if last.close
                && ranges.len() > 1
                && ranges[0].0 == origin
                && ranges.last().unwrap().1 == origin + length
            {
                let tail = ranges.pop().unwrap();
                metrics.append(tail.0, tail.1, &mut output, false)?;
                let head = ranges.remove(0);
                metrics.append(head.0, head.1, &mut output, true)?;
            }
            for (start, end) in ranges {
                metrics.append(start, end, &mut output, false)?;
            }
            first = end_index;
        }
        Ok(output)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn kurbo_subdivide_retains_exact_control_polygons() {
        let curve = PathSeg::Cubic(kurbo::CubicBez::new(
            (0.0, 0.0),
            (2.0, 4.0),
            (6.0, 4.0),
            (8.0, 0.0),
        ));
        let (left, right) = curve.subdivide();
        assert_eq!(
            left,
            PathSeg::Cubic(kurbo::CubicBez::new(
                (0.0, 0.0),
                (1.0, 2.0),
                (2.5, 3.0),
                (4.0, 3.0)
            ))
        );
        assert_eq!(
            right,
            PathSeg::Cubic(kurbo::CubicBez::new(
                (4.0, 3.0),
                (5.5, 3.0),
                (7.0, 2.0),
                (8.0, 0.0)
            ))
        );
    }

    #[test]
    fn kurbo_subsegments_preserve_parameters_and_full_snapshots() {
        let curves = [
            PathSeg::Line(kurbo::Line::new((0.0, 0.0), (12.0, 9.0))),
            PathSeg::Quad(kurbo::QuadBez::new((0.0, 0.0), (4.0, 12.0), (12.0, 9.0))),
            PathSeg::Cubic(kurbo::CubicBez::new(
                (0.0, 0.0),
                (2.0, 14.0),
                (10.0, -4.0),
                (12.0, 9.0),
            )),
        ];
        for curve in curves {
            assert_eq!(subcurve(curve, 0.0, 1.0), curve);
            for (lower, upper) in [(0.0, 0.5), (0.25, 1.0), (0.13, 0.77)] {
                let part = subcurve(curve, lower, upper);
                for parameter in [0.0, 0.125, 0.5, 0.875, 1.0] {
                    let expected = curve.eval(lower + (upper - lower) * parameter);
                    assert!(part.eval(parameter).distance(expected) < 1e-12);
                }
            }
        }
        let line = Path2D::parse_svg("M0 0L12 9").unwrap();
        let controls = line.cubic_intervals(&[0.0, 1.0]).unwrap()[0];
        assert_eq!(
            controls,
            [
                Point::ORIGIN,
                Point::new(4.0, 3.0),
                Point::new(8.0, 6.0),
                Point::new(12.0, 9.0)
            ]
        );
    }
}
