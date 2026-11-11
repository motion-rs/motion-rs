//! Brush reveal timing: recorded input fitting, smoothing and timed arc-length reveals.

use super::super::{BrushStroke, BrushStrokeBuilder, invalid};
use super::geometry::subpaths;
use super::{InputSample, validate_input};
use motion_core::{Duration, Paint, Path2D, Time, ValidationError};

/// Stateless arc timing with a monotone progress curve and pauses between subpaths.
#[derive(Clone, Debug)]
pub struct BrushTiming {
    /// Total moving duration, excluding lift pauses.
    pub duration: Duration,
    /// Ordered normalized time/progress controls, spanning (0,0)..=(1,1).
    pub curve: Vec<(f64, f64)>,
    /// Pen-up interval between each adjacent subpath.
    pub lift_pause: Duration,
}

impl BrushTiming {
    /// Creates linear arc timing with no pen-up interval.
    pub fn new(duration: Duration) -> Self {
        Self {
            duration,
            curve: vec![(0.0, 0.0), (1.0, 1.0)],
            lift_pause: Duration::ZERO,
        }
    }
}

impl BrushStrokeBuilder {
    /// Fits recorded positions with Catmull-Rom cubic segments and retains pressure/time.
    pub fn from_samples(
        samples: &[InputSample],
        width: f64,
        paint: impl Into<Paint>,
    ) -> Result<Self, ValidationError> {
        validate_input(samples)?;
        let mut path = Path2D::new();
        path.try_move_to(samples[0].position)?;
        let mut cumulative = vec![0.0];
        for index in 0..samples.len() - 1 {
            let previous = samples[index.saturating_sub(1)].position;
            let first = samples[index].position;
            let second = samples[index + 1].position;
            let next = samples[(index + 2).min(samples.len() - 1)].position;
            if first == second {
                cumulative.push(cumulative[index]);
                continue;
            }
            let control_first = first + (second - previous) * (1.0 / 6.0);
            let control_second = second - (next - first) * (1.0 / 6.0);
            let mut segment = Path2D::new();
            segment.try_move_to(first)?;
            segment.try_cubic_to(control_first, control_second, second)?;
            cumulative.push(cumulative.last().copied().unwrap_or(0.0) + segment.try_length()?);
            path.try_cubic_to(control_first, control_second, second)?;
        }
        let total = cumulative.last().copied().unwrap_or(0.0);
        if total <= 0.0 {
            return Err(invalid("input", "Recording must contain nonzero travel"));
        }
        let mut curve: Vec<(f64, f64)> = Vec::new();
        for (length, sample) in cumulative.iter().zip(samples) {
            let amount = *length / total;
            if let Some(control) = curve.last_mut().filter(|control| control.0 == amount) {
                control.1 = sample.pressure;
            } else {
                curve.push((amount, sample.pressure));
            }
        }
        Ok(
            BrushStroke::builder(path, width, paint).configure(|options| {
                options.input = samples.to_vec();
                options.input_arcs = cumulative.iter().map(|arc| arc / total).collect();
                options.pressure = curve;
            }),
        )
    }

    /// Smooths a sampled path by a bounded symmetric kernel; no frame history is retained.
    pub fn try_smooth(mut self, strength: f64) -> Result<Self, ValidationError> {
        if !strength.is_finite() || !(0.0..=1.0).contains(&strength) {
            return Err(invalid("smooth", "Require strength in 0..=1"));
        }
        let paths = subpaths(&self.config.path)?;
        let mut output = Path2D::new();
        for path in paths {
            let points = (0..=64)
                .map(|index| path.try_point_at(index as f64 / 64.0).map(|value| value.0))
                .collect::<Result<Vec<_>, _>>()?;
            for (index, point) in points.iter().enumerate() {
                let point = if index == 0 || index == 64 {
                    *point
                } else {
                    *point
                        + (points[index - 1] + (points[index + 1] - points[index - 1]) * 0.5
                            - *point)
                            * (strength * 0.5)
                };
                if index == 0 {
                    output.try_move_to(point)?;
                } else {
                    output.try_line_to(point)?;
                }
            }
        }
        self.config.path = output;
        Ok(self)
    }
}

impl BrushStroke {
    /// Reveals a recorded stroke at its original local sample clock, including pauses.
    pub fn write_with(self, time: Time) -> Result<Self, ValidationError> {
        let options = &self.config.advanced;
        if options.input.is_empty() {
            return Err(invalid("timing", "Recorded timing requires from_samples"));
        }
        validate_input(&options.input)?;
        if time < Time::ZERO {
            return Err(invalid("timing", "Reveal time must be nonnegative"));
        }
        let input = &options.input;
        let index = input
            .partition_point(|sample| sample.time <= time)
            .saturating_sub(1);
        let amount = if index + 1 >= input.len() {
            1.0
        } else {
            options.input_arcs[index]
                + (options.input_arcs[index + 1] - options.input_arcs[index])
                    * (time - input[index].time).as_seconds()
                    / (input[index + 1].time - input[index].time).as_seconds()
        };
        Ok(self.write(amount.clamp(0.0, 1.0)))
    }

    /// Reveals with a monotone speed curve, retaining full material across pen-up pauses.
    pub fn write_timed(self, time: Time, timing: &BrushTiming) -> Result<Self, ValidationError> {
        let stroke = self;
        if time < Time::ZERO
            || timing.duration <= Duration::ZERO
            || timing.duration.as_seconds() > 86400.0
            || timing.lift_pause.as_seconds() > 86400.0
            || !(2..=512).contains(&timing.curve.len())
            || timing.curve.first() != Some(&(0.0, 0.0))
            || timing.curve.last() != Some(&(1.0, 1.0))
            || timing.curve.iter().any(|(time, arc)| {
                !time.is_finite()
                    || !arc.is_finite()
                    || !(0.0..=1.0).contains(time)
                    || !(0.0..=1.0).contains(arc)
            })
            || timing
                .curve
                .windows(2)
                .any(|pair| pair[0].0 >= pair[1].0 || pair[0].1 > pair[1].1)
        {
            return Err(invalid(
                "timing",
                "Invalid duration, pause or monotone curve",
            ));
        }
        let paths = subpaths(&stroke.config.path)?;
        let lengths = paths
            .iter()
            .map(Path2D::try_length)
            .collect::<Result<Vec<_>, _>>()?;
        let total: f64 = lengths.iter().sum();
        if total <= 0.0 {
            return Err(invalid("timing", "Writing requires nonzero travel"));
        }
        let mut elapsed = 0.0;
        let mut arc = 0.0;
        let seconds = time.as_seconds();
        for length in lengths {
            let duration = timing.duration.as_seconds() * length / total;
            if seconds <= elapsed {
                return Ok(stroke.write(arc / total));
            }
            if seconds < elapsed + duration {
                let amount = (seconds - elapsed) / duration;
                let index = timing
                    .curve
                    .partition_point(|control| control.0 <= amount)
                    .saturating_sub(1)
                    .min(timing.curve.len() - 2);
                let first = timing.curve[index];
                let second = timing.curve[index + 1];
                let progress =
                    first.1 + (second.1 - first.1) * (amount - first.0) / (second.0 - first.0);
                return Ok(stroke.write((arc + length * progress) / total));
            }
            arc += length;
            elapsed += duration + timing.lift_pause.as_seconds();
        }
        Ok(stroke.write(1.0))
    }
}
