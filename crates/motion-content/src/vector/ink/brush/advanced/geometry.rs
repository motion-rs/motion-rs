//! Brush stroke geometry: subpath splitting, pressure evaluation, nib sampling and outlines.

use super::super::{BrushStroke, invalid};
use super::{BrushCap, NibMode};
use motion_core::{Path2D, PathVerb, Point, ValidationError, Vector2};

fn append(path: &mut Path2D, verb: PathVerb) -> Result<(), ValidationError> {
    match verb {
        PathVerb::MoveTo(point) => path.try_move_to(point),
        PathVerb::LineTo(point) => path.try_line_to(point),
        PathVerb::QuadraticTo(control, end) => path.try_quadratic_to(control, end),
        PathVerb::CubicTo(first, second, end) => path.try_cubic_to(first, second, end),
        PathVerb::Close => path.try_close(),
    }
}

pub(super) fn subpaths(path: &Path2D) -> Result<Vec<Path2D>, ValidationError> {
    let mut paths = Vec::new();
    let mut current = Path2D::new();
    for verb in path.verbs() {
        if matches!(verb, PathVerb::MoveTo(_)) && !current.verbs().is_empty() {
            paths.push(current);
            current = Path2D::new();
        }
        append(&mut current, *verb)?;
    }
    if !current.verbs().is_empty() {
        paths.push(current);
    }
    if paths.len() > 64 {
        return Err(invalid("subpaths", "At most 64 brush subpaths"));
    }
    Ok(paths)
}

#[derive(Clone, Copy, Debug)]
pub(super) struct Sample {
    pub(super) point: Point,
    pub(super) radius: f64,
    pub(super) arc: f64,
    pub(super) tangent: Vector2,
    pub(super) angle: f64,
}

fn pressure(stroke: &BrushStroke, amount: f64) -> Result<f64, ValidationError> {
    let options = &stroke.advanced;
    let value = if let Some(function) = &options.function {
        function.0(amount)
    } else if !options.pressure.is_empty() {
        let index = options
            .pressure
            .partition_point(|control| control.0 <= amount)
            .saturating_sub(1)
            .min(options.pressure.len() - 2);
        let first = options.pressure[index];
        let second = options.pressure[index + 1];
        let amount = ((amount - first.0) / (second.0 - first.0)).clamp(0.0, 1.0);
        let amount = amount * amount * (3.0 - 2.0 * amount);
        first.1 + (second.1 - first.1) * amount
    } else if amount <= 0.5 {
        stroke.pressure[0] + (stroke.pressure[1] - stroke.pressure[0]) * amount * 2.0
    } else {
        stroke.pressure[1] + (stroke.pressure[2] - stroke.pressure[1]) * (amount - 0.5) * 2.0
    };
    if !value.is_finite() || !(0.0..=4.0).contains(&value) {
        return Err(invalid(
            "pressure_function",
            "Pressure function must return finite 0..=4",
        ));
    }
    Ok(value)
}

pub(super) fn samples(
    stroke: &BrushStroke,
    progress: f64,
) -> Result<Vec<Vec<Sample>>, ValidationError> {
    let options = &stroke.advanced;
    let paths = subpaths(&stroke.path)?;
    let lengths = paths
        .iter()
        .map(Path2D::try_length)
        .collect::<Result<Vec<_>, _>>()?;
    let total: f64 = lengths.iter().sum();
    let mut consumed = 0.0;
    let mut groups = Vec::new();
    for (path, length) in paths.iter().zip(lengths) {
        let reveal = (progress * total - consumed).clamp(0.0, length);
        if reveal == 0.0 {
            consumed += length;
            continue;
        }
        let intervals = ((length / 4.0).ceil() as usize).clamp(2, options.samples);
        let end = if length > 0.0 { reveal / length } else { 0.0 };
        let mut amounts = (0..=intervals)
            .map(|index| index as f64 / intervals as f64)
            .take_while(|amount| *amount <= end)
            .collect::<Vec<_>>();
        if amounts.last().copied() != Some(end) {
            amounts.push(end);
        }
        for bristle in 0..options.bristles {
            let mut group = Vec::new();
            for amount in &amounts {
                let (mut point, tangent) = path.try_point_at(*amount)?;
                let global = if total > 0.0 {
                    (consumed + amount * length) / total
                } else {
                    0.0
                };
                let mut pressure = pressure(stroke, global)?;
                for (distance, cap) in [
                    (amount * length, options.caps[0]),
                    ((1.0 - amount) * length, options.caps[1]),
                ] {
                    let extent = match cap {
                        BrushCap::Round | BrushCap::Flat => 0.0,
                        BrushCap::Tapered(length) => length,
                    };
                    if extent > 0.0 {
                        let amount = (distance / extent).clamp(0.0, 1.0);
                        pressure *= amount * amount * (3.0 - 2.0 * amount);
                    }
                }
                let index = options
                    .input_arcs
                    .partition_point(|arc| *arc <= global)
                    .saturating_sub(1)
                    .min(options.input.len().saturating_sub(2));
                let speed = if index + 1 < options.input.len() {
                    (options.input_arcs[index + 1] - options.input_arcs[index]) * total
                        / (options.input[index + 1].time - options.input[index].time).as_seconds()
                } else {
                    total
                };
                pressure /= 1.0 + options.speed_response * speed;
                let angle = match options.nib {
                    NibMode::Fixed(angle) => angle,
                    NibMode::FollowPath(offset) => libm::atan2(tangent.y, tangent.x) + offset,
                    NibMode::Tilt => {
                        let first = options.input[index];
                        let second = options.input[index + 1];
                        let interval = options.input_arcs[index + 1] - options.input_arcs[index];
                        let amount = if interval > 0.0 {
                            ((global - options.input_arcs[index]) / interval).clamp(0.0, 1.0)
                        } else {
                            1.0
                        };
                        let delta = (second.tilt - first.tilt + std::f64::consts::PI)
                            .rem_euclid(std::f64::consts::TAU)
                            - std::f64::consts::PI;
                        first.tilt + delta * amount
                    }
                };
                if let Some((amplitude, frequency, seed)) = options.wobble {
                    point.x += motion_core::noise::value2([global * total * frequency, 0.0], seed)
                        * amplitude;
                    point.y += motion_core::noise::value2(
                        [global * total * frequency, 0.0],
                        seed.wrapping_add(1),
                    ) * amplitude;
                }
                if options.bristles > 1 {
                    let offset =
                        (bristle as f64 / (options.bristles - 1) as f64 - 0.5) * options.spread;
                    point = point + Vector2::new(-tangent.y, tangent.x) * offset;
                    pressure *= 1.0
                        + motion_core::noise::value2([global * 8.0, bristle as f64], options.seed)
                            * options.jitter;
                }
                group.push(Sample {
                    point,
                    radius: stroke.width * 0.5 * pressure / options.bristles as f64,
                    arc: consumed + amount * length,
                    tangent,
                    angle,
                });
            }
            groups.push(group);
        }
        consumed += length;
    }
    Ok(groups)
}

pub(super) fn rotated(horizontal: f64, vertical: f64, angle: f64) -> Vector2 {
    let cosine = libm::cos(angle);
    let sine = libm::sin(angle);
    Vector2::new(
        horizontal * cosine - vertical * sine,
        horizontal * sine + vertical * cosine,
    )
}

pub(super) fn outline(
    stroke: &BrushStroke,
    groups: &[Vec<Sample>],
) -> Result<Path2D, ValidationError> {
    let padding = stroke
        .ink
        .map_or(Ok(0.0), |ink| ink.padding().map(f64::from))
        .map_err(|error| invalid("ink", &error.to_string()))?;
    let mut path = Path2D::new();
    for group in groups {
        let add = |path: &mut Path2D, mut polygon: Vec<Point>| -> Result<(), ValidationError> {
            for (cap, endpoint, direction) in [
                (&stroke.advanced.caps[0], group.first(), 1.0),
                (&stroke.advanced.caps[1], group.last(), -1.0),
            ] {
                if matches!(cap, BrushCap::Flat)
                    && let Some(endpoint) = endpoint
                {
                    let side = |point: Point| {
                        let offset = point - endpoint.point;
                        (offset.x * endpoint.tangent.x + offset.y * endpoint.tangent.y) * direction
                    };
                    let mut clipped = Vec::new();
                    for index in 0..polygon.len() {
                        let first = polygon[index];
                        let second = polygon[(index + 1) % polygon.len()];
                        let left = side(first);
                        let right = side(second);
                        if left >= 0.0 {
                            clipped.push(first);
                        }
                        if (left >= 0.0) != (right >= 0.0) {
                            clipped.push(first + (second - first) * (left / (left - right)));
                        }
                    }
                    polygon = clipped;
                }
            }
            if polygon.len() >= 3 {
                path.try_move_to(polygon[0])?;
                for point in &polygon[1..] {
                    path.try_line_to(*point)?;
                }
                path.try_close()?;
            }
            Ok(())
        };
        for sample in group {
            let polygon = (0..24)
                .map(|index| {
                    let angle = index as f64 * std::f64::consts::TAU / 24.0;
                    sample.point
                        + rotated(
                            (sample.radius + padding) * libm::cos(angle),
                            (sample.radius + padding) * stroke.aspect * libm::sin(angle),
                            sample.angle,
                        )
                })
                .collect();
            add(&mut path, polygon)?;
        }
        for pair in group.windows(2) {
            let delta = pair[1].point - pair[0].point;
            let norm = libm::sqrt(delta.x * delta.x + delta.y * delta.y);
            if norm == 0.0 {
                continue;
            }
            let normal = Vector2::new(-delta.y / norm, delta.x / norm);
            let support = |sample: Sample| {
                let local = rotated(normal.x, normal.y, -sample.angle);
                let denominator = libm::sqrt(
                    local.x * local.x + stroke.aspect * stroke.aspect * local.y * local.y,
                );
                rotated(
                    local.x / denominator,
                    stroke.aspect * stroke.aspect * local.y / denominator,
                    sample.angle,
                ) * (sample.radius + padding)
            };
            let first = support(pair[0]);
            let second = support(pair[1]);
            add(
                &mut path,
                vec![
                    pair[0].point + first,
                    pair[0].point - first,
                    pair[1].point - second,
                    pair[1].point + second,
                ],
            )?;
        }
    }
    Ok(path)
}
