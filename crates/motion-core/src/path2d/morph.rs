//! Bounded path morphing with exact cubic source geometry.

use kurbo::ParamCurveArea;
use std::sync::Arc;

use crate::{Interpolate, MAX_PATH_VERBS, Path2D, PathVerb, Point, Rect, Shape, ValidationError};

const MAX_CONTOURS: usize = 64;

const DEFAULT_SAMPLES: usize = 32;

const MAX_SAMPLES: usize = 256;

const MAX_ALIGNMENT_WORK: usize = 16_777_216;

/// A checked path value for geometry interpolation and fill/stroke.
/// Original path snapshots are retained at exact animation endpoints.
///
/// ```
/// use motion_core::prelude::*;
/// use motion_core::{
///     DisplayList, RasterLimits, ResourcePool, ValidatedDisplayList, MAX_PATH_VERBS,
///     MAX_STROKE_VERBS, MAX_SVG_PATH_BYTES, sample_offsets,
/// };
/// let (from, to) = MorphPath::pair(
///     Circle::new(Point::new(8.0, 8.0), 6.0)?.path()?,
///     Star::new(Point::new(8.0, 8.0), 5, 6.0, 2.5)?.path()?,
/// )?;
/// let animation = Tween::new(from, to, 1.seconds());
/// let middle = animation.sample(500.milliseconds());
/// assert!(middle.bounding_box()?.is_finite());
/// # Ok::<(), motion_core::Error>(())
/// ```
#[derive(Clone, Debug)]
pub struct MorphPath {
    path: Path2D,
    contours: Arc<[Contour]>,
    pair: Option<(Arc<Pair>, bool)>,
}

#[derive(Clone, Debug)]
struct Pair {
    contours: Vec<MatchedContour>,
}

#[derive(Clone, Debug)]
struct MatchedContour {
    source: Vec<[Point; 4]>,
    target: Vec<[Point; 4]>,
    closed: bool,
}

#[derive(Clone, Copy, Debug)]
struct Measure {
    area: f64,
    rank: f64,
    length: f64,
}

#[derive(Clone, Debug)]
struct Contour {
    path: Path2D,
    closed: bool,
}

fn invalid(field: &str, message: &str) -> ValidationError {
    ValidationError::invalid("morph_path", field, message)
}

fn limit(message: &str) -> ValidationError {
    ValidationError::limit("morph_path", "geometry", message)
}

fn finish(
    contours: &mut Vec<Contour>,
    current: &mut Option<Path2D>,
    closed: bool,
) -> Result<(), ValidationError> {
    if let Some(path) = current.take() {
        if contours.len() >= MAX_CONTOURS {
            return Err(limit("Morph paths admit at most 64 contours"));
        }
        contours.push(Contour { path, closed });
    }
    Ok(())
}

impl MorphPath {
    /// Captures a path and promotes line/quadratic segments to cubic controls.
    /// At most 64 contours are accepted; all existing path limits still apply.
    pub fn new(path: Path2D) -> Result<Self, ValidationError> {
        let mut contours = Vec::new();
        let mut current = None;
        let mut position = Point::ORIGIN;
        let mut start = position;
        for verb in path.verbs() {
            if let PathVerb::MoveTo(value) = *verb {
                finish(&mut contours, &mut current, false)?;
                let mut contour = Path2D::new();
                contour.try_move_to(value)?;
                current = Some(contour);
                position = value;
                start = value;
                continue;
            }
            if matches!(verb, PathVerb::Close) && current.is_none() {
                continue;
            }
            if current.is_none() {
                let mut contour = Path2D::new();
                contour.try_move_to(position)?;
                start = position;
                current = Some(contour);
            }
            let contour = current.as_mut().expect("active morph contour");
            match *verb {
                PathVerb::LineTo(end) => {
                    contour.try_cubic_to(
                        position.interpolate(&end, 1.0 / 3.0),
                        position.interpolate(&end, 2.0 / 3.0),
                        end,
                    )?;
                    position = end;
                }
                PathVerb::QuadraticTo(control, end) => {
                    let cubic = kurbo::QuadBez::new(
                        (position.x, position.y),
                        (control.x, control.y),
                        (end.x, end.y),
                    )
                    .raise();
                    contour.try_cubic_to(
                        Point::new(cubic.p1.x, cubic.p1.y),
                        Point::new(cubic.p2.x, cubic.p2.y),
                        end,
                    )?;
                    position = end;
                }
                PathVerb::CubicTo(first, second, end) => {
                    contour.try_cubic_to(first, second, end)?;
                    position = end;
                }
                PathVerb::Close => {
                    if position != start {
                        contour.try_cubic_to(
                            position.interpolate(&start, 1.0 / 3.0),
                            position.interpolate(&start, 2.0 / 3.0),
                            start,
                        )?;
                    }
                    contour.try_close()?;
                    position = start;
                    finish(&mut contours, &mut current, true)?;
                }
                PathVerb::MoveTo(_) => unreachable!("move handled above"),
            }
        }
        finish(&mut contours, &mut current, false)?;
        Ok(Self {
            path,
            contours: contours.into(),
            pair: None,
        })
    }

    /// Prepares a pair once for repeated Tween sampling, using 32 uniform
    /// arc-length landmarks plus both paths' existing curve boundaries.
    pub fn pair(source: Path2D, target: Path2D) -> Result<(Self, Self), ValidationError> {
        Self::pair_with_samples(source, target, DEFAULT_SAMPLES)
    }

    /// Prepares 4..=256 arc-length landmarks per contour. Output remains capped
    /// by MAX_PATH_VERBS, with at most 64 matched contours and bounded alignment.
    pub fn pair_with_samples(
        source: Path2D,
        target: Path2D,
        samples: usize,
    ) -> Result<(Self, Self), ValidationError> {
        if !(4..=MAX_SAMPLES).contains(&samples) {
            return Err(invalid("samples", "Morph sample count must be in 4..=256"));
        }
        let mut source = Self::new(source)?;
        let mut target = Self::new(target)?;
        let pair = Arc::new(Pair::new(&source, &target, samples)?);
        source.pair = Some((pair.clone(), false));
        target.pair = Some((pair, true));
        Ok((source, target))
    }

    /// Checked, stateless interpolation. Finite overshoot is allowed; exact
    /// progress 0/1 returns the original verb snapshots. Unprepared values
    /// compute correspondence on demand; prefer pair for repeated sampling.
    pub fn try_interpolate(&self, target: &Self, progress: f64) -> Result<Self, ValidationError> {
        if !progress.is_finite() {
            return Err(invalid("progress", "Morph progress must be finite"));
        }
        if progress == 0.0 {
            return Ok(self.clone());
        }
        if progress == 1.0 {
            return Ok(target.clone());
        }
        if self.path == target.path {
            return Ok(self.clone());
        }
        let prepared;
        let (pair, reversed) = match (&self.pair, &target.pair) {
            (Some((first, side)), Some((second, target_side)))
                if Arc::ptr_eq(first, second) && side != target_side =>
            {
                (first.as_ref(), *side)
            }
            _ => {
                prepared = Pair::new(self, target, DEFAULT_SAMPLES)?;
                (&prepared, false)
            }
        };
        let mut path = Path2D::new();
        for contour in &pair.contours {
            let (source, target) = if reversed {
                (&contour.target, &contour.source)
            } else {
                (&contour.source, &contour.target)
            };
            for (index, (first, second)) in source.iter().zip(target).enumerate() {
                let points: [Point; 4] = std::array::from_fn(|control| {
                    if first[control] == second[control] {
                        first[control]
                    } else {
                        first[control].interpolate(&second[control], progress)
                    }
                });
                if index == 0 {
                    path.try_move_to(points[0])?;
                }
                path.try_cubic_to(points[1], points[2], points[3])?;
            }
            if contour.closed {
                path.try_close()?;
            }
        }
        Self::new(path)
    }
}

impl PartialEq for MorphPath {
    fn eq(&self, other: &Self) -> bool {
        self.path == other.path
    }
}

impl Interpolate for MorphPath {
    fn interpolate(&self, target: &Self, progress: f64) -> Self {
        self.try_interpolate(target, progress)
            .expect("MorphPath interpolation failed")
    }
}

fn cubics(path: &Path2D) -> Vec<[Point; 4]> {
    let mut position = Point::ORIGIN;
    let mut result = Vec::new();
    for verb in path.verbs() {
        match *verb {
            PathVerb::MoveTo(value) => position = value,
            PathVerb::CubicTo(first, second, end) => {
                result.push([position, first, second, end]);
                position = end;
            }
            _ => {}
        }
    }
    result
}

fn measure(contour: &Contour) -> Result<Measure, ValidationError> {
    let length = contour.path.try_length()?;
    let bounds = contour.path.bounding_box();
    let scale = bounds.width().max(bounds.height());
    if !contour.closed || scale == 0.0 {
        return Ok(Measure {
            area: 0.0,
            rank: f64::NEG_INFINITY,
            length,
        });
    }
    let center = bounds.center();
    let mut area = 0.0;
    for curve in cubics(&contour.path) {
        let controls = curve.map(|point| (point - center) / scale);
        let controls = controls.map(|point| kurbo::Point::new(point.x, point.y));
        area +=
            kurbo::CubicBez::new(controls[0], controls[1], controls[2], controls[3]).signed_area();
    }
    if !area.is_finite() {
        return Err(invalid("area", "Morph contour area must remain finite"));
    }
    let rank = if area == 0.0 {
        f64::NEG_INFINITY
    } else {
        libm::log(area.abs()) + 2.0 * libm::log(scale)
    };
    Ok(Measure { area, rank, length })
}

fn reverse(contour: &Contour) -> Result<Path2D, ValidationError> {
    let curves = cubics(&contour.path);
    let mut path = Path2D::new();
    let first = curves
        .last()
        .map(|curve| curve[3])
        .unwrap_or(contour.path.try_point_at(0.0)?.0);
    path.try_move_to(first)?;
    for curve in curves.iter().rev() {
        path.try_cubic_to(curve[2], curve[1], curve[0])?;
    }
    if contour.closed {
        path.try_close()?;
    }
    Ok(path)
}

fn rotate(path: &Path2D, phase: f64) -> Result<Path2D, ValidationError> {
    if phase == 0.0 {
        return Ok(path.clone());
    }
    let mut output = Path2D::new();
    for (index, piece) in [path.try_trim(phase, 1.0)?, path.try_trim(0.0, phase)?]
        .iter()
        .enumerate()
    {
        for verb in piece.verbs() {
            match *verb {
                PathVerb::MoveTo(value) if index == 0 => output.try_move_to(value)?,
                PathVerb::CubicTo(first, second, end) => output.try_cubic_to(first, second, end)?,
                PathVerb::MoveTo(_) | PathVerb::Close => {}
                _ => {
                    return Err(invalid(
                        "curve",
                        "Morph normalization requires cubic contours",
                    ));
                }
            }
        }
    }
    output.try_close()?;
    Ok(output)
}

fn phase(
    source: &Path2D,
    target: &Path2D,
    samples: usize,
    work: &mut usize,
) -> Result<f64, ValidationError> {
    *work += samples * samples;
    if *work > MAX_ALIGNMENT_WORK {
        return Err(limit("Morph alignment work allowance exhausted"));
    }
    let first = (0..samples)
        .map(|index| {
            source
                .try_point_at(index as f64 / samples as f64)
                .map(|value| value.0)
        })
        .collect::<Result<Vec<_>, _>>()?;
    let second = (0..samples)
        .map(|index| {
            target
                .try_point_at(index as f64 / samples as f64)
                .map(|value| value.0)
        })
        .collect::<Result<Vec<_>, _>>()?;
    let bounds = source.bounding_box().union(target.bounding_box());
    let scale = bounds.width().max(bounds.height());
    if !scale.is_finite() {
        return Err(invalid(
            "bounds",
            "Combined morph extent must remain finite",
        ));
    }
    if scale == 0.0 {
        return Ok(0.0);
    }
    let mut best = (f64::INFINITY, 0);
    for shift in 0..samples {
        let mut cost = 0.0;
        for index in 0..samples {
            let delta = (first[index] - second[(index + shift) % samples]) / scale;
            cost += delta.dot(delta);
        }
        if cost < best.0 {
            best = (cost, shift);
        }
    }
    Ok(best.1 as f64 / samples as f64)
}

impl Pair {
    fn new(
        source: &MorphPath,
        target: &MorphPath,
        samples: usize,
    ) -> Result<Self, ValidationError> {
        let mut minimum_verbs = 0;
        let mut count = 0;
        for closed in [true, false] {
            let matched = source
                .contours
                .iter()
                .filter(|contour| contour.closed == closed)
                .count()
                .max(
                    target
                        .contours
                        .iter()
                        .filter(|contour| contour.closed == closed)
                        .count(),
                );
            minimum_verbs += matched * (samples + 1 + usize::from(closed));
            count += matched;
        }
        if count > MAX_CONTOURS || minimum_verbs > MAX_PATH_VERBS {
            return Err(limit(
                "Requested morph topology exceeds the contour or verb allowance",
            ));
        }
        let source_measures = source
            .contours
            .iter()
            .map(measure)
            .collect::<Result<Vec<_>, _>>()?;
        let target_measures = target
            .contours
            .iter()
            .map(measure)
            .collect::<Result<Vec<_>, _>>()?;
        let largest = |measures: &[Measure]| {
            measures
                .iter()
                .max_by(|first, second| first.rank.total_cmp(&second.rank))
                .map_or(0.0, |value| value.area)
        };
        let source_area = largest(&source_measures);
        let target_area = largest(&target_measures);
        let reverse_target = source_area != 0.0
            && target_area != 0.0
            && source_area.is_sign_positive() != target_area.is_sign_positive();
        let mut contours = Vec::new();
        let mut work = 0;
        let mut verbs = 0;
        for closed in [true, false] {
            let sorted = |path: &MorphPath, measures: &[Measure]| {
                let mut indices = path
                    .contours
                    .iter()
                    .enumerate()
                    .filter(|(_, contour)| contour.closed == closed)
                    .map(|(index, _)| index)
                    .collect::<Vec<_>>();
                indices.sort_by(|first, second| {
                    measures[*second]
                        .rank
                        .total_cmp(&measures[*first].rank)
                        .then_with(|| measures[*second].length.total_cmp(&measures[*first].length))
                        .then_with(|| first.cmp(second))
                });
                indices
            };
            let source_indices = sorted(source, &source_measures);
            let target_indices = sorted(target, &target_measures);
            for index in 0..source_indices.len().max(target_indices.len()) {
                if contours.len() >= MAX_CONTOURS {
                    return Err(limit("Matched topology exceeds 64 contours"));
                }
                let first = source_indices
                    .get(index)
                    .map(|index| &source.contours[*index]);
                let second = target_indices
                    .get(index)
                    .map(|index| &target.contours[*index]);
                let collapse =
                    |owner: &MorphPath, fallback: &MorphPath| -> Result<Path2D, ValidationError> {
                        let mut path = Path2D::new();
                        let bounds = if owner.path.verbs().is_empty() {
                            fallback.path.bounding_box()
                        } else {
                            owner.path.bounding_box()
                        };
                        path.try_move_to(bounds.center())?;
                        Ok(path)
                    };
                let first_path = match first {
                    Some(contour) => contour.path.clone(),
                    None => collapse(source, target)?,
                };
                let mut second_path = match second {
                    Some(contour) if reverse_target => reverse(contour)?,
                    Some(contour) => contour.path.clone(),
                    None => collapse(target, source)?,
                };
                if closed {
                    second_path = rotate(
                        &second_path,
                        phase(&first_path, &second_path, samples, &mut work)?,
                    )?;
                }
                let mut knots = first_path.cubic_breakpoints()?;
                knots.extend(second_path.cubic_breakpoints()?);
                knots.extend((0..=samples).map(|index| index as f64 / samples as f64));
                knots.sort_by(f64::total_cmp);
                knots.dedup_by(|first, second| (*first - *second).abs() <= 1e-12);
                if let Some(last) = knots.last_mut() {
                    *last = 1.0;
                }
                verbs += knots.len() + usize::from(closed);
                if verbs > MAX_PATH_VERBS {
                    return Err(limit("Resampled morph exceeds the verb allowance"));
                }
                let mut first_segments = first_path.cubic_intervals(&knots)?;
                let mut second_segments = second_path.cubic_intervals(&knots)?;
                for segments in [&mut first_segments, &mut second_segments] {
                    for segment in 1..segments.len() {
                        segments[segment][0] = segments[segment - 1][3];
                    }
                    if closed {
                        let first = segments[0][0];
                        segments.last_mut().unwrap()[3] = first;
                    }
                }
                contours.push(MatchedContour {
                    source: first_segments,
                    target: second_segments,
                    closed,
                });
            }
        }
        Ok(Self { contours })
    }
}

impl Shape for MorphPath {
    fn path(&self) -> Result<Path2D, ValidationError> {
        Ok(self.path.clone())
    }

    fn bounding_box(&self) -> Result<Rect, ValidationError> {
        Ok(self.path.bounding_box())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ValidationErrorKind;

    #[test]
    fn exact_endpoints_and_nonuniform_curve_segmentation_preserve_geometry() {
        let source = Path2D::parse_svg("M0 0Q4 8 8 0").unwrap();
        let target = Path2D::parse_svg("M0 0Q2 4 4 4Q6 4 8 0").unwrap();
        let (first, second) = MorphPath::pair(source.clone(), target.clone()).unwrap();
        assert_eq!(first.interpolate(&second, 0.0).path().unwrap(), source);
        assert_eq!(first.interpolate(&second, 1.0).path().unwrap(), target);
        for progress in [0.01, 0.25, 0.5, 0.99] {
            let result = first.try_interpolate(&second, progress).unwrap();
            assert!((result.path().unwrap().length() - source.length()).abs() < 1e-8);
            for sample in [0.0, 0.25, 0.5, 0.75, 1.0] {
                assert!(
                    result
                        .path()
                        .unwrap()
                        .point_at(sample)
                        .0
                        .distance(source.point_at(sample).0)
                        < 1e-8
                );
            }
        }
        let line = Path2D::parse_svg("M0 0H8").unwrap();
        let curve = Path2D::parse_svg("M0 4Q4 12 8 4").unwrap();
        let (first, second) = MorphPath::pair(line, curve).unwrap();
        let middle = first.interpolate(&second, 0.5).path().unwrap();
        assert!(middle.point_at(0.5).0.distance(Point::new(4.0, 4.0)) < 1e-8);
        assert_eq!(middle.point_at(0.0).0, Point::new(0.0, 2.0));
        assert_eq!(middle.point_at(1.0).0, Point::new(8.0, 2.0));
    }

    #[test]
    fn closed_phase_alignment_and_area_pairing_are_not_input_order_zip() {
        let source = Path2D::parse_svg("M2 2H10V10H2Z").unwrap();
        let target = Path2D::parse_svg("M10 10H2V2H10Z").unwrap();
        let (first, second) = MorphPath::pair(source, target).unwrap();
        let middle = first.interpolate(&second, 0.5);
        assert_eq!(
            middle.bounding_box().unwrap(),
            Rect::new(2.0, 2.0, 10.0, 10.0)
        );
        let source = Path2D::parse_svg("M99 -1h2v2h-2Z M-3 -3h6v6h-6Z").unwrap();
        let target = Path2D::parse_svg("M16 -4h8v8h-8Z M199 -1h2v2h-2Z").unwrap();
        let (first, second) = MorphPath::pair(source, target).unwrap();
        let middle = first.interpolate(&second, 0.5);
        assert!((middle.contours[0].path.bounding_box().center().x - 10.0).abs() < 1e-8);
        assert!((middle.contours[1].path.bounding_box().center().x - 150.0).abs() < 1e-8);
    }

    #[test]
    fn missing_contours_and_holes_grow_from_points_with_winding_preserved() {
        let source = Path2D::from_rect(Rect::new(0.0, 0.0, 16.0, 16.0)).unwrap();
        let target = Path2D::parse_svg("M0 0H16V16H0Z M6 6V10H10V6Z").unwrap();
        let (first, second) = MorphPath::pair(source, target).unwrap();
        let middle = first.interpolate(&second, 0.5);
        assert_eq!(middle.contours.len(), 2);
        assert_eq!(
            middle.contours[1].path.bounding_box(),
            Rect::new(7.0, 7.0, 9.0, 9.0)
        );
        let (empty, filled) = MorphPath::pair(
            Path2D::new(),
            crate::Circle::new(Point::new(8.0, 8.0), 4.0)
                .unwrap()
                .path()
                .unwrap(),
        )
        .unwrap();
        assert!(empty.path().unwrap().verbs().is_empty());
        assert!(
            (empty
                .interpolate(&filled, 0.5)
                .bounding_box()
                .unwrap()
                .width()
                - 4.0)
                .abs()
                < 1e-8
        );
        assert!(
            filled
                .interpolate(&empty, 1.0)
                .path()
                .unwrap()
                .verbs()
                .is_empty()
        );
    }

    #[test]
    fn disappearing_and_offset_holes_follow_the_path_coordinate_frame() {
        let source = Path2D::parse_svg("M0 0H16V16H0Z M6 6V10H10V6Z").unwrap();
        let target = Path2D::parse_svg("M16 0H32V16H16Z").unwrap();
        let (first, second) = MorphPath::pair(source, target).unwrap();
        let middle = first.interpolate(&second, 0.5);
        let outer = middle.contours[0].path.bounding_box();
        let hole = middle.contours[1].path.bounding_box();
        assert_eq!(hole.center(), Point::new(16.0, 8.0));
        assert!(
            hole.left >= outer.left
                && hole.right <= outer.right
                && hole.top >= outer.top
                && hole.bottom <= outer.bottom
        );
        let target = Path2D::parse_svg("M0 0H16V16H0Z M2 2V4H4V2Z").unwrap();
        let (first, second) = MorphPath::pair(Path2D::new(), target).unwrap();
        let middle = first.interpolate(&second, 0.5);
        let outer = middle.contours[0].path.bounding_box();
        let hole = middle.contours[1].path.bounding_box();
        assert!(
            hole.left >= outer.left
                && hole.right <= outer.right
                && hole.top >= outer.top
                && hole.bottom <= outer.bottom
        );
    }

    #[test]
    fn prepared_morphs_support_tweens_reverse_queries_and_checked_failures() {
        use crate::{Animation, Time, Tween};
        let source = Path2D::parse_svg("M1 1H5V5H1Z").unwrap();
        let target = Path2D::parse_svg("M3 3H9V9H3Z").unwrap();
        let (first, second) = MorphPath::pair(source.clone(), target.clone()).unwrap();
        let animation = Tween::new(first.clone(), second.clone(), Time::from_ticks(100));
        let expected = animation.sample(Time::from_ticks(50));
        assert_eq!(
            MorphPath::new(source.clone())
                .unwrap()
                .try_interpolate(&MorphPath::new(target.clone()).unwrap(), 0.5)
                .unwrap(),
            expected
        );
        std::thread::scope(|scope| {
            let handles = (0..3)
                .map(|_| scope.spawn(|| first.interpolate(&second, 0.5)))
                .collect::<Vec<_>>();
            for handle in handles {
                assert_eq!(handle.join().unwrap(), expected);
            }
        });
        for ticks in [100, 25, 0, 75, 50] {
            let _result = animation.sample(Time::from_ticks(ticks));
        }
        assert_eq!(animation.sample(Time::from_ticks(50)), expected);
        assert_eq!(
            first.interpolate(&second, 0.25),
            second.interpolate(&first, 0.75)
        );
        assert!(first.try_interpolate(&second, f64::NAN).is_err());
        assert!(first.try_interpolate(&second, f64::MAX).is_err());
        let (stationary, growing) = MorphPath::pair(
            Path2D::parse_svg("M1 0H2").unwrap(),
            Path2D::parse_svg("M1 0H3").unwrap(),
        )
        .unwrap();
        assert_eq!(
            stationary
                .try_interpolate(&growing, 1e16)
                .unwrap()
                .path()
                .unwrap()
                .verbs()[0],
            PathVerb::MoveTo(Point::new(1.0, 0.0))
        );
        assert_eq!(first.path().unwrap(), source);
        assert_eq!(second.path().unwrap(), target);
        assert!(MorphPath::pair_with_samples(source, target, 257).is_err());
        let mut many = Path2D::new();
        for index in 0..65 {
            many.try_move_to(Point::new(index as f64, 0.0)).unwrap();
        }
        assert_eq!(
            MorphPath::new(many).unwrap_err().kind,
            ValidationErrorKind::StructuralLimit
        );
        let mut bounded = Path2D::new();
        for index in 0..64 {
            bounded.try_move_to(Point::new(index as f64, 0.0)).unwrap();
            bounded.try_close().unwrap();
        }
        assert_eq!(
            MorphPath::pair_with_samples(bounded.clone(), bounded, 256)
                .unwrap_err()
                .kind,
            ValidationErrorKind::StructuralLimit
        );
    }

    #[test]
    fn cubic_promotion_preserves_source_geometry_and_contour_boundaries() {
        let source = Path2D::parse_svg("M0 0L2 0Q3 2 4 0Z L1 1 M10 10").unwrap();
        let retained = source.clone();
        let morph = MorphPath::new(source).unwrap();
        assert_eq!(morph.path().unwrap(), retained);
        assert_eq!(morph.contours.len(), 3);
        assert!(morph.contours[0].closed);
        assert!(!morph.contours[1].closed);
        assert!(
            morph
                .contours
                .iter()
                .all(|contour| contour.path.verbs().iter().all(|verb| matches!(
                    verb,
                    PathVerb::MoveTo(_) | PathVerb::CubicTo(..) | PathVerb::Close
                )))
        );
        let quadratic = Path2D::parse_svg("M0 0Q1 1 2 0").unwrap();
        let promoted = MorphPath::new(quadratic.clone()).unwrap();
        assert!((promoted.contours[0].path.length() - quadratic.length()).abs() < 1e-10);
        for progress in [0.0, 0.25, 0.5, 0.75, 1.0] {
            assert!(
                promoted.contours[0]
                    .path
                    .point_at(progress)
                    .0
                    .distance(quadratic.point_at(progress).0)
                    < 1e-9
            );
        }
        assert!(MorphPath::new(Path2D::new()).unwrap().contours.is_empty());
    }
}
