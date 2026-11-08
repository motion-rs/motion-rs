//! Validated one-dimensional data-to-drawing coordinates, independent of rendering.

use motion_core::ValidationError;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Mapping {
    Linear,
    Log10,
}

/// A stateless `f64` data domain mapped to an `f64` drawing-coordinate range.
///
/// Domain and range may ascend or descend but must have distinct finite endpoints.
/// Logarithmic domains/values must be positive and their log endpoints distinct.
/// Values extrapolate by default; [`Self::clamp`] bounds both mapping directions.
/// Units are authored by the caller: this type does not convert meters to AU or
/// add perspective, an orbital model, a clock, or previous-frame state.
///
/// ```
/// use motion_content::diagram::Scale;
/// let scale = Scale::log10((0.1, 1000.0), (100.0, 860.0))?;
/// assert!((scale.map(10.0)? - 480.0).abs() < 0.001);
/// assert!((scale.invert(480.0)? - 10.0).abs() < 1e-10);
/// # Ok::<(), motion_core::ValidationError>(())
/// ```
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Scale {
    domain: (f64, f64),
    transformed: (f64, f64),
    range: (f64, f64),
    mapping: Mapping,
    clamped: bool,
}

impl Scale {
    /// Validates a linear mapping; endpoints may run in either direction.
    pub fn linear(domain: (f64, f64), range: (f64, f64)) -> Result<Self, ValidationError> {
        Self::new(domain, range, Mapping::Linear)
    }

    /// Validates a base-ten logarithmic mapping with positive domain endpoints.
    pub fn log10(domain: (f64, f64), range: (f64, f64)) -> Result<Self, ValidationError> {
        Self::new(domain, range, Mapping::Log10)
    }

    /// Replaces clamping for both map and invert; disabled by default.
    /// Nonpositive log inputs remain invalid even with clamping enabled.
    pub fn clamp(mut self, enabled: bool) -> Self {
        self.clamped = enabled;
        self
    }

    /// Returns the original, ordered data endpoints, without unit conversion.
    pub fn domain(self) -> (f64, f64) {
        self.domain
    }

    /// Returns the ordered drawing-coordinate endpoints.
    pub fn range(self) -> (f64, f64) {
        self.range
    }

    /// Maps a finite data value to drawing coordinates, preserving endpoints exactly.
    /// Extrapolation that cannot be represented as a finite `f64` returns an error.
    pub fn map(self, mut value: f64) -> Result<f64, ValidationError> {
        if !value.is_finite() || (self.mapping == Mapping::Log10 && value <= 0.0) {
            return Err(invalid());
        }
        if self.clamped {
            value = value.clamp(
                self.domain.0.min(self.domain.1),
                self.domain.0.max(self.domain.1),
            );
        }
        if value == self.domain.0 {
            return Ok(self.range.0);
        }
        if value == self.domain.1 {
            return Ok(self.range.1);
        }
        let value = match self.mapping {
            Mapping::Linear => value,
            Mapping::Log10 => libm::log10(value),
        };
        let fraction = normalize(value, self.transformed);
        let mapped = interpolate(self.range, fraction);
        if mapped.is_finite() {
            Ok(mapped)
        } else {
            Err(invalid())
        }
    }

    /// Maps finite drawing coordinates back into data units, with the same clamp policy.
    pub fn invert(self, mut coordinate: f64) -> Result<f64, ValidationError> {
        if !coordinate.is_finite() {
            return Err(invalid());
        }
        if self.clamped {
            coordinate = coordinate.clamp(
                self.range.0.min(self.range.1),
                self.range.0.max(self.range.1),
            );
        }
        let fraction = normalize(coordinate, self.range);
        self.value_at(fraction)
    }

    /// Returns 2 through 256 uniformly spaced positions in transformed data space.
    /// Linear values are arithmetic; log values are geometric. Endpoints are exact.
    /// These are not automatically rounded "nice" ticks. Requests whose data
    /// values collapse at `f64` precision fail rather than returning duplicates.
    pub fn ticks(self, count: usize) -> Result<Vec<f64>, ValidationError> {
        if !(2..=256).contains(&count) {
            return Err(invalid());
        }
        let mut ticks = Vec::with_capacity(count);
        for index in 0..count {
            let value = self.value_at(index as f64 / (count - 1) as f64)?;
            if ticks.last().is_some_and(|previous| {
                if self.domain.1 > self.domain.0 {
                    value <= *previous
                } else {
                    value >= *previous
                }
            }) {
                return Err(invalid());
            }
            ticks.push(value);
        }
        Ok(ticks)
    }

    /// Returns at most `max_count` aligned ticks without changing domain or range.
    /// Accepts `2..=256`. Linear steps use 1, 2 or 5 times a power of ten;
    /// logarithmic steps use integer powers of ten, skipping decades when needed.
    /// Log domains containing fewer than two decade powers use linear nice steps.
    /// Endpoints are not forced; a narrow domain may yield zero or one tick.
    /// Unrepresentable steps/indices or collapsed values return an error.
    pub fn nice_ticks(self, max_count: usize) -> Result<Vec<f64>, ValidationError> {
        if !(2..=256).contains(&max_count) {
            return Err(invalid());
        }
        let lower = self.domain.0.min(self.domain.1);
        let upper = self.domain.0.max(self.domain.1);
        let log_lower = self.transformed.0.min(self.transformed.1);
        let log_upper = self.transformed.0.max(self.transformed.1);
        let logarithmic =
            self.mapping == Mapping::Log10 && log_upper.floor() - log_lower.ceil() >= 1.0;
        let mut ticks = if logarithmic {
            let step = ((log_upper.floor() - log_lower.ceil()) / (max_count - 1) as f64)
                .ceil()
                .max(1.0);
            tick_grid((log_lower, log_upper), step, max_count)?
                .into_iter()
                .map(|exponent| libm::pow(10.0, exponent))
                .filter(|value| *value >= lower && *value <= upper)
                .collect::<Vec<_>>()
        } else {
            let intervals = (max_count - 1) as f64;
            let span = upper - lower;
            let raw = if span.is_finite() {
                span / intervals
            } else {
                upper / intervals - lower / intervals
            }
            .max(f64::from_bits(1));
            let base = libm::pow(10.0, libm::log10(raw).floor()).max(f64::from_bits(1));
            let ratio = raw / base;
            let factor = if ratio <= 1.0 {
                1.0
            } else if ratio <= 2.0 {
                2.0
            } else if ratio <= 5.0 {
                5.0
            } else {
                10.0
            };
            let mut step = factor * base;
            if (upper / step).floor() - (lower / step).ceil() + 1.0 > max_count as f64 {
                step = if factor == 1.0 {
                    2.0
                } else if factor == 2.0 {
                    5.0
                } else if factor == 5.0 {
                    10.0
                } else {
                    20.0
                } * base;
            }
            tick_grid((lower, upper), step, max_count)?
        };
        if ticks.iter().any(|value| !value.is_finite())
            || ticks.windows(2).any(|pair| pair[0] >= pair[1])
        {
            return Err(invalid());
        }
        if self.domain.0 > self.domain.1 {
            ticks.reverse();
        }
        Ok(ticks)
    }

    fn new(
        domain: (f64, f64),
        range: (f64, f64),
        mapping: Mapping,
    ) -> Result<Self, ValidationError> {
        if ![domain.0, domain.1, range.0, range.1]
            .iter()
            .all(|value| value.is_finite())
            || domain.0 == domain.1
            || range.0 == range.1
            || (mapping == Mapping::Log10 && (domain.0 <= 0.0 || domain.1 <= 0.0))
        {
            return Err(invalid());
        }
        let transformed = match mapping {
            Mapping::Linear => domain,
            Mapping::Log10 => (libm::log10(domain.0), libm::log10(domain.1)),
        };
        if transformed.0 == transformed.1 {
            return Err(invalid());
        }
        Ok(Self {
            domain,
            transformed,
            range,
            mapping,
            clamped: false,
        })
    }

    fn value_at(self, fraction: f64) -> Result<f64, ValidationError> {
        if !fraction.is_finite() {
            return Err(invalid());
        }
        if fraction == 0.0 {
            return Ok(self.domain.0);
        }
        if fraction == 1.0 {
            return Ok(self.domain.1);
        }
        let transformed = interpolate(self.transformed, fraction);
        let value = match self.mapping {
            Mapping::Linear => transformed,
            Mapping::Log10 => libm::pow(10.0, transformed),
        };
        if value.is_finite() && (self.mapping == Mapping::Linear || value > 0.0) {
            Ok(value)
        } else {
            Err(invalid())
        }
    }
}

fn tick_grid(bounds: (f64, f64), step: f64, max_count: usize) -> Result<Vec<f64>, ValidationError> {
    if !step.is_finite() || step <= 0.0 {
        return Err(invalid());
    }
    let first = (bounds.0 / step).ceil();
    let last = (bounds.1 / step).floor();
    if !first.is_finite()
        || !last.is_finite()
        || first.abs() > 9007199254740992.0
        || last.abs() > 9007199254740992.0
    {
        return Err(invalid());
    }
    if first > last {
        return Ok(Vec::new());
    }
    let count = last - first + 1.0;
    if count > max_count as f64 {
        return Err(invalid());
    }
    let mut ticks = Vec::with_capacity(count as usize);
    for offset in 0..count as usize {
        let value = (first + offset as f64) * step;
        if value >= bounds.0 && value <= bounds.1 {
            ticks.push(if value == 0.0 { 0.0 } else { value });
        }
    }
    Ok(ticks)
}

fn normalize(value: f64, endpoints: (f64, f64)) -> f64 {
    let span = endpoints.1 - endpoints.0;
    let offset = value - endpoints.0;
    if span.is_finite() && offset.is_finite() {
        offset / span
    } else {
        (value * 0.5 - endpoints.0 * 0.5) / (endpoints.1 * 0.5 - endpoints.0 * 0.5)
    }
}

fn interpolate(endpoints: (f64, f64), fraction: f64) -> f64 {
    let span = endpoints.1 - endpoints.0;
    if span.is_finite() {
        endpoints.0 + span * fraction
    } else {
        endpoints.0 * (1.0 - fraction) + endpoints.1 * fraction
    }
}

fn invalid() -> ValidationError {
    ValidationError::invalid(
        "scale",
        "domain",
        "Scale requires distinct finite endpoints, positive log data, representable results/steps/indices and a tick count in 2..=256",
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn nice_linear_ticks_use_aligned_steps_without_forcing_endpoints() -> Result<(), ValidationError>
    {
        assert_eq!(
            Scale::linear((3.0, 97.0), (0.0, 100.0))?.nice_ticks(6)?,
            [20.0, 40.0, 60.0, 80.0]
        );
        assert_eq!(
            Scale::linear((-97.0, -3.0), (0.0, 100.0))?.nice_ticks(6)?,
            [-80.0, -60.0, -40.0, -20.0]
        );
        let ticks = Scale::linear((-3.0, 9.0), (0.0, 100.0))?.nice_ticks(5)?;
        assert_eq!(ticks, [0.0, 5.0]);
        assert_eq!(ticks[0].to_bits(), 0.0_f64.to_bits());
        Ok(())
    }

    #[test]
    fn nice_log_ticks_select_decades_and_fall_back_for_narrow_domains()
    -> Result<(), ValidationError> {
        let log = Scale::log10((0.1, 1000.0), (100.0, 860.0))?;
        assert_eq!(log.nice_ticks(5)?, [0.1, 1.0, 10.0, 100.0, 1000.0]);
        assert_eq!(log.nice_ticks(3)?, [1.0, 100.0]);
        assert_eq!(
            Scale::log10((2.0, 8.0), (0.0, 100.0))?.nice_ticks(5)?,
            [2.0, 4.0, 6.0, 8.0]
        );
        Ok(())
    }

    #[test]
    fn nice_ticks_follow_domain_order_not_range_clamp_or_evaluation_order()
    -> Result<(), ValidationError> {
        let forward = Scale::linear((3.0, 97.0), (100.0, 0.0))?;
        let backward = Scale::linear((97.0, 3.0), (0.0, 100.0))?;
        assert_eq!(backward.nice_ticks(6)?, [80.0, 60.0, 40.0, 20.0]);
        let original = forward.nice_ticks(6)?;
        for count in [2, 256, 3, 6] {
            forward.nice_ticks(count)?;
        }
        assert_eq!(forward.clamp(true).nice_ticks(6)?, original);
        assert_eq!(forward.nice_ticks(6)?, original);
        assert_eq!(forward.domain(), (3.0, 97.0));
        assert_eq!(forward.range(), (100.0, 0.0));
        Ok(())
    }

    #[test]
    fn nice_ticks_are_bounded_finite_ordered_and_inside_extreme_domains()
    -> Result<(), ValidationError> {
        for scale in [
            Scale::linear((-1e100, 1e100), (0.0, 100.0))?,
            Scale::linear((-0.07, 0.23), (0.0, 100.0))?,
            Scale::linear((1e-250, 9e-250), (0.0, 100.0))?,
            Scale::linear((f64::from_bits(1), f64::from_bits(5)), (0.0, 100.0))?,
            Scale::log10((1e-300, 1e300), (0.0, 100.0))?,
        ] {
            for count in 2..=256 {
                let ticks = scale.nice_ticks(count)?;
                assert!(ticks.len() <= count);
                assert!(ticks.windows(2).all(|pair| pair[0] < pair[1]));
                assert!(ticks.iter().all(|value| value.is_finite()
                    && *value >= scale.domain().0
                    && *value <= scale.domain().1));
            }
        }
        Ok(())
    }

    #[test]
    fn nice_ticks_reject_invalid_budgets_and_unrepresentable_grid_steps_or_indices()
    -> Result<(), ValidationError> {
        let scale = Scale::linear((0.0, 100.0), (0.0, 100.0))?;
        assert!(scale.nice_ticks(1).is_err());
        assert!(scale.nice_ticks(257).is_err());
        assert!(
            Scale::linear((1.0, 1.0 + f64::EPSILON), (0.0, 100.0))?
                .nice_ticks(5)
                .is_err()
        );
        assert!(
            Scale::linear((0.0, f64::MAX), (0.0, 100.0))?
                .nice_ticks(2)
                .is_err()
        );
        Ok(())
    }

    #[test]
    fn linear_and_logarithmic_mappings_have_exact_endpoints_and_correct_midpoints()
    -> Result<(), ValidationError> {
        let linear = Scale::linear((0.0, 100.0), (20.0, 220.0))?;
        assert_eq!(linear.map(50.0)?, 120.0);
        assert_eq!(linear.invert(120.0)?, 50.0);
        let log = Scale::log10((0.1, 1000.0), (100.0, 860.0))?;
        assert_eq!(log.map(0.1)?, 100.0);
        assert_eq!(log.map(1000.0)?, 860.0);
        assert_eq!(log.invert(100.0)?, 0.1);
        assert_eq!(log.invert(860.0)?, 1000.0);
        assert!((log.map(10.0)? - 480.0).abs() < 0.001);
        assert!((log.invert(480.0)? - 10.0).abs() < 1e-10);
        Ok(())
    }

    #[test]
    fn reversed_domains_ranges_and_clamping_are_explicit_and_replaceable()
    -> Result<(), ValidationError> {
        let scale = Scale::linear((100.0, 0.0), (0.0, 200.0))?;
        assert_eq!(scale.map(75.0)?, 50.0);
        assert_eq!(scale.map(150.0)?, -100.0);
        assert_eq!(scale.clamp(true).map(150.0)?, 0.0);
        assert_eq!(scale.clamp(true).invert(300.0)?, 0.0);
        assert_eq!(scale.clamp(true).clamp(false), scale);
        let log = Scale::log10((1.0, 100.0), (200.0, 0.0))?;
        assert!((log.map(10.0)? - 100.0).abs() < 0.001);
        assert_eq!(log.clamp(true).map(1000.0)?, 0.0);
        assert_eq!(log.clamp(true).invert(300.0)?, 1.0);
        assert!(log.clamp(true).map(0.0).is_err());
        Ok(())
    }

    #[test]
    fn huge_small_and_opposite_signed_domains_do_not_need_f32_data_coordinates()
    -> Result<(), ValidationError> {
        let scale = Scale::linear((-f64::MAX, f64::MAX), (0.0, 100.0))?;
        assert_eq!(scale.map(0.0)?, 50.0);
        assert_eq!(scale.invert(50.0)?, 0.0);
        let scale = Scale::linear((0.0, 1e30), (0.0, 1000.0))?;
        assert!((scale.map(5e29)? - 500.0).abs() < 0.001);
        let scale = Scale::log10((1e-300, 1e300), (0.0, 600.0))?;
        assert_eq!(scale.map(1.0)?, 300.0);
        assert_eq!(scale.invert(300.0)?, 1.0);
        let scale = Scale::linear((f64::from_bits(1), f64::from_bits(2)), (0.0, 100.0))?;
        assert_eq!(scale.map(f64::from_bits(1))?, 0.0);
        assert_eq!(scale.invert(100.0)?, f64::from_bits(2));
        Ok(())
    }

    #[test]
    fn ticks_are_bounded_ordered_data_values_not_rounded_or_pixel_quantized()
    -> Result<(), ValidationError> {
        let linear = Scale::linear((0.0, 100.0), (20.0, 220.0))?;
        assert_eq!(linear.ticks(5)?, [0.0, 25.0, 50.0, 75.0, 100.0]);
        let descending = Scale::linear((100.0, 0.0), (20.0, 220.0))?;
        assert_eq!(descending.ticks(3)?, [100.0, 50.0, 0.0]);
        let log = Scale::log10((1.0, 10000.0), (0.0, 1.0))?;
        assert_eq!(log.ticks(5)?, [1.0, 10.0, 100.0, 1000.0, 10000.0]);
        assert!(linear.ticks(1).is_err());
        assert!(linear.ticks(257).is_err());
        let narrow = Scale::linear((1.0, 1.0 + f64::EPSILON), (0.0, 100.0))?;
        assert!(narrow.ticks(3).is_err());
        Ok(())
    }

    #[test]
    fn finite_round_trips_replay_in_any_order() -> Result<(), ValidationError> {
        for scale in [
            Scale::linear((-20.0, 500.0), (40.0, 800.0))?,
            Scale::log10((0.01, 1e6), (800.0, 40.0))?,
        ] {
            for coordinate in [420.0, 40.0, 800.0, 200.0, 420.0] {
                let value = scale.invert(coordinate)?;
                assert!((scale.map(value)? - coordinate).abs() < 0.001);
            }
        }
        Ok(())
    }

    #[test]
    fn drawing_coordinates_retain_precision_beyond_f32() -> Result<(), ValidationError> {
        let scale = Scale::linear((0.0, 1.0), (100_000_000.125, 100_000_000.625))?;
        assert_eq!(scale.map(0.5)?, 100_000_000.375);
        assert_eq!(scale.invert(100_000_000.375)?, 0.5);
        assert_eq!(scale.map(0.5)?, scale.map(0.5)?);
        Ok(())
    }

    #[test]
    fn invalid_domains_inputs_and_unrepresentable_extrapolation_fail() -> Result<(), ValidationError>
    {
        assert!(Scale::linear((1.0, 1.0), (0.0, 100.0)).is_err());
        assert!(Scale::linear((0.0, f64::INFINITY), (0.0, 100.0)).is_err());
        assert!(Scale::linear((0.0, 1.0), (2.0, 2.0)).is_err());
        assert!(Scale::linear((0.0, 1.0), (f64::NAN, 2.0)).is_err());
        assert!(Scale::log10((0.0, 100.0), (0.0, 100.0)).is_err());
        assert!(Scale::log10((-1.0, 100.0), (0.0, 100.0)).is_err());
        let scale = Scale::linear((0.0, 1.0), (0.0, 100.0))?;
        assert!(scale.map(f64::NAN).is_err());
        assert!(scale.map(f64::MAX).is_err());
        assert!(scale.invert(f64::INFINITY).is_err());
        assert!(
            Scale::log10((1.0, 10.0), (0.0, 1.0))?
                .invert(1000.0)
                .is_err()
        );
        Ok(())
    }
}
