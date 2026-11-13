//! Checked seeded random-access trajectory programs, independent of graphics APIs.

use super::{InstanceKind, MAX_INSTANCE_COUNT};
use crate::ValidationError;
use std::sync::Arc;

/// Version-one analytic header length, in four-byte words.
pub const ANALYTIC_HEADER_WORDS: usize = 20;

/// Version-one initial-state stride, in four-byte words.
pub const ANALYTIC_STATE_WORDS: usize = 8;

/// Longest admitted analytic age and lifetime, in seconds.
const MAX_ANALYTIC_SECONDS: f32 = 3600.0;

/// Largest absolute model parameter or initial spatial value admitted to f32 evaluation.
const MAX_ANALYTIC_VALUE: f32 = 1e6;

/// Largest resolved temporal or noise lattice phase in the portable f32 contract.
const MAX_ANALYTIC_PHASE: f32 = 8192.0;

/// Closed analytic models; fixed-step simulations are deliberately not represented.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(u32)]
pub enum AnalyticTrajectoryKind {
    /// Constant acceleration with coupled linear damping.
    Ballistic = 1,
    /// Smoothstep interpolation from position to target.
    Converge,
    /// Circular motion with seeded initial phase.
    Orbit,
    /// Linear drift plus quintic value-noise displacement.
    NoiseDrift,
}

/// Seeded initial facts for one stable particle, before evaluating time.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct AnalyticInstanceState {
    /// Position at birth.
    pub position: [f32; 2],
    /// Velocity at birth.
    pub velocity: [f32; 2],
    /// Target coordinates, orbital phase, or independent noise phases.
    pub custom: [f32; 2],
    /// Stable particle ID, used as the noise lattice key.
    pub index: u32,
    /// Initial sprite angle in radians.
    pub rotation: f32,
}

/// Immutable bounded analytic program evaluated into ordinary twelve-word primitives.
///
/// The seed identifies the keyed initial-state stream; initial facts are explicitly supplied
/// rather than depending on a renderer's random implementation. Time is age in seconds.
/// Colors, sizes, images and blend policy remain in the ordinary [`super::InstanceUpload`].
#[derive(Clone, Debug, PartialEq)]
pub struct AnalyticInstanceUpload {
    words: Arc<[f32]>,
}

impl AnalyticInstanceUpload {
    /// Validates a model, explicit seed, age, lifetime, parameters and initial states.
    ///
    /// Parameters use the builtin models' eight-word schema: ballistic origin/acceleration/
    /// damping/reserved/speed range; converge reserves them; orbit center/radius/angular
    /// velocity; noise drift origin/velocity/amplitude/frequency. Unsupported precision
    /// ranges fail explicitly, allowing a caller to retain evaluated CPU primitives.
    ///
    /// Returns [`crate::ValidationErrorKind::Unsupported`] outside the portable f32 parameter
    /// and phase range, [`crate::ValidationErrorKind::InvalidValue`] for malformed inputs,
    /// and [`crate::ValidationErrorKind::StructuralLimit`] when upload allocation fails.
    #[expect(
        clippy::too_many_arguments,
        reason = "The versioned model has explicit time, seed and primitive inputs"
    )]
    pub fn new(
        kind: AnalyticTrajectoryKind,
        seed: u64,
        age: f32,
        lifetime: f32,
        parameters: [f32; 8],
        primitive: InstanceKind,
        align_to_velocity: bool,
        states: impl ExactSizeIterator<Item = AnalyticInstanceState>,
    ) -> Result<Self, ValidationError> {
        let invalid = || {
            ValidationError::invalid(
                "instances.analytic",
                "program",
                "Invalid analytic model, time, initial state or precision range",
            )
        };
        let count = states.len();
        if count > MAX_INSTANCE_COUNT
            || !age.is_finite()
            || !lifetime.is_finite()
            || !(0.0..=MAX_ANALYTIC_SECONDS).contains(&age)
            || lifetime <= 0.0
            || lifetime > MAX_ANALYTIC_SECONDS
            || age >= lifetime
            || parameters.iter().any(|value| !value.is_finite())
            || !matches!(
                primitive,
                InstanceKind::Disc | InstanceKind::Glow | InstanceKind::Sprite
            )
            || (kind == AnalyticTrajectoryKind::Ballistic
                && (parameters[4] < 0.0 || parameters[6] < 0.0 || parameters[7] < parameters[6]))
            || (kind == AnalyticTrajectoryKind::Orbit && parameters[2] < 0.0)
            || (kind == AnalyticTrajectoryKind::NoiseDrift
                && (parameters[4] < 0.0 || parameters[5] < 0.0))
        {
            return Err(invalid());
        }
        let precision = || {
            ValidationError::unsupported(
                "instances.analytic",
                "precision",
                "Trajectory exceeds the bounded f32 analytic precision range",
            )
        };
        if parameters
            .iter()
            .any(|value| value.abs() > MAX_ANALYTIC_VALUE)
            || (kind == AnalyticTrajectoryKind::Orbit
                && (parameters[3] * age).abs() > MAX_ANALYTIC_PHASE)
            || (kind == AnalyticTrajectoryKind::NoiseDrift
                && parameters[5] * age > MAX_ANALYTIC_PHASE)
        {
            return Err(precision());
        }
        let expected = ANALYTIC_HEADER_WORDS + count * ANALYTIC_STATE_WORDS;
        let mut words = Vec::new();
        words.try_reserve_exact(expected).map_err(|_| {
            ValidationError::limit(
                "instances.analytic",
                "max_bytes",
                "Could not reserve analytic upload",
            )
        })?;
        words.extend([
            kind as u32 as f32,
            age,
            lifetime,
            primitive as u32 as f32,
            u32::from(align_to_velocity) as f32,
            count as f32,
            f32::from_bits(seed as u32),
            f32::from_bits((seed >> 32) as u32),
        ]);
        words.extend(parameters);
        words.extend([0.0; 4]);
        for state in states {
            if words.len() >= expected
                || state
                    .position
                    .into_iter()
                    .chain(state.velocity)
                    .chain(state.custom)
                    .chain([state.rotation])
                    .any(|value| !value.is_finite())
            {
                return Err(invalid());
            }
            if state
                .position
                .into_iter()
                .chain(state.velocity)
                .chain(state.custom)
                .chain([state.rotation])
                .any(|value| value.abs() > MAX_ANALYTIC_VALUE)
                || (kind == AnalyticTrajectoryKind::NoiseDrift
                    && state
                        .custom
                        .iter()
                        .any(|value| value.abs() > MAX_ANALYTIC_PHASE))
            {
                return Err(precision());
            }
            words.extend(state.position);
            words.extend(state.velocity);
            words.extend(state.custom);
            words.extend([f32::from_bits(state.index), state.rotation]);
        }
        if words.len() != expected {
            return Err(invalid());
        }
        Ok(Self {
            words: words.into(),
        })
    }

    /// Number of ordinary primitives produced by evaluation.
    pub fn len(&self) -> usize {
        self.words[5] as usize
    }

    /// Whether this program has no initial states.
    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    /// Exact immutable upload size in bytes.
    pub fn bytes(&self) -> u64 {
        self.words.len() as u64 * 4
    }

    /// Version-one program words; seed and IDs are integer bit patterns.
    pub fn words(&self) -> &[f32] {
        &self.words
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn analytic_upload_validates_time_finiteness_and_exact_layout() {
        let state = AnalyticInstanceState {
            position: [0.0; 2],
            velocity: [1.0; 2],
            custom: [0.0; 2],
            index: u32::MAX,
            rotation: 0.0,
        };
        let make = |age, parameters, state| {
            AnalyticInstanceUpload::new(
                AnalyticTrajectoryKind::Ballistic,
                u64::MAX,
                age,
                2.0,
                parameters,
                InstanceKind::Disc,
                false,
                [state].into_iter(),
            )
        };
        let upload = make(1.0, [0.0; 8], state).unwrap();
        assert_eq!(upload.len(), 1);
        assert_eq!(
            upload.bytes(),
            ((ANALYTIC_HEADER_WORDS + ANALYTIC_STATE_WORDS) * 4) as u64
        );
        assert_eq!(upload.words()[6].to_bits(), u32::MAX);
        assert!(make(2.0, [0.0; 8], state).is_err());
        assert!(make(f32::NAN, [0.0; 8], state).is_err());
        assert!(make(1.0, [f32::INFINITY; 8], state).is_err());
        assert!(
            make(
                1.0,
                [0.0; 8],
                AnalyticInstanceState {
                    rotation: f32::NAN,
                    ..state
                }
            )
            .is_err()
        );
    }
}
