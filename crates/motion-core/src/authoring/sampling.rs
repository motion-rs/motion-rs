//! Integer-tick centered shutter sampling and linear accumulation.
//!
//! ```
//! use motion_core::prelude::*;
//! let cache = SceneCache::default();
//! let context = SceneContext::new(
//!     Time::from_frame(1, fps!(30)),
//!     fps!(30),
//!     Size::new(8.0, 4.0),
//!     &cache,
//! );
//! let node = context.motion_blur(2.milliseconds(), 2, |sample| {
//!     Node::from(Rect::new(sample.seconds() * 4.0, 0.0, 4.0, 4.0).fill(Color::WHITE))
//! });
//! node.compile(
//!     context.size(),
//!     PixelSize::new(8, 4),
//!     ValidationLimits::default(),
//! )?;
//! # Ok::<(), motion_core::Error>(())
//! ```

use crate::{Duration, FrameRate, Node, SceneContext, TimeOffset, ValidationError};

/// Global shutter configuration; with exact centered tick offsets.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct MotionBlur {
    /// Exposure angle in 0..=360 degrees; only final integer ticks are rounded.
    pub shutter_angle: f64,
    /// Equally weighted midpoint captures, in 1..=64.
    pub samples: u32,
}

impl Default for MotionBlur {
    fn default() -> Self {
        Self {
            shutter_angle: 180.0,
            samples: 8,
        }
    }
}

impl MotionBlur {
    /// Validates without evaluating scenes or initializing rendering resources.
    pub fn validate(self) -> Result<(), ValidationError> {
        if !self.shutter_angle.is_finite()
            || !(0.0..=360.0).contains(&self.shutter_angle)
            || !(1..=64).contains(&self.samples)
        {
            Err(invalid(
                "configuration",
                "Require finite shutter angle in 0..=360 and 1..=64 samples",
            ))
        } else {
            Ok(())
        }
    }

    /// Derives exposure from the exact rational rate, rounded once to nearest tick.
    pub fn shutter(self, rate: FrameRate) -> Result<Duration, ValidationError> {
        self.validate()?;
        let bits = self.shutter_angle.to_bits();
        let exponent = ((bits >> 52) & 0x7ff) as u32;
        let mantissa = (bits & ((1u64 << 52) - 1)) | if exponent == 0 { 0 } else { 1u64 << 52 };
        let numerator = i128::from(crate::TICKS_PER_SECOND)
            * i128::from(rate.denominator())
            * i128::from(mantissa);
        let shift = if exponent == 0 { 1074 } else { 1075 - exponent };
        let Some(denominator) = 1i128
            .checked_shl(shift)
            .and_then(|scale| scale.checked_mul(i128::from(rate.numerator()) * 360))
            .filter(|value| *value > 0)
        else {
            return Ok(Duration::ZERO);
        };
        let ticks = (numerator + denominator / 2) / denominator;
        Ok(Duration::from_ticks(
            i64::try_from(ticks).map_err(|_| invalid("shutter", "Exposure exceeds signed ticks"))?
                as u64,
        ))
    }
}

fn invalid(field: &str, message: &str) -> ValidationError {
    ValidationError::invalid("motion_blur", field, message)
}

/// Normalized temporal exposure profile.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum ShutterCurve {
    /// Uniform rectangular exposure.
    #[default]
    Box,
    /// Symmetric triangular exposure.
    Triangle,
    /// Smooth raised-cosine exposure.
    Cosine,
}

/// Explicit deterministic jitter, exposure weights and adaptive screen-space sampling.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct ShutterSampling {
    /// None uses centered integer midpoint offsets; Some enables stratified jitter.
    pub jitter_seed: Option<u64>,
    /// Estimate endpoint geometry displacement and select 8..=64 samples.
    pub adaptive: bool,
    /// Temporal exposure profile, normalized over the selected samples.
    pub curve: ShutterCurve,
}

impl ShutterSampling {
    /// Captures a full scene or an explicitly selected subtree. Nested shutter captures
    /// are rejected. Endpoint estimates cannot detect motion returning to its starting pose.
    pub fn capture(
        self,
        blur: MotionBlur,
        context: &SceneContext<'_>,
        output: crate::PixelSize,
        mut build: impl FnMut(&SceneContext<'_>) -> Node,
    ) -> Result<Node, ValidationError> {
        let shutter = blur.shutter(context.frame_rate())?;
        output.mapping(context.size())?;
        if context.render_quality() == crate::RenderQuality::Draft {
            return Ok(build(context));
        }
        let count = if self.adaptive && shutter > Duration::ZERO {
            let half = TimeOffset::from_ticks((shutter.ticks() / 2) as i64);
            let left = context.resampled(-half)?;
            let right = context.resampled(half)?;
            let drawing = crate::DrawContext::default().with_assets(context.assets());
            let first = build(&left).compile_with_context(
                context.size(),
                output,
                crate::ValidationLimits::default(),
                &drawing,
            )?;
            let last = build(&right).compile_with_context(
                context.size(),
                output,
                crate::ValidationLimits::default(),
                &drawing,
            )?;
            displacement_samples(first.display_list(), last.display_list())
        } else {
            blur.samples
        };
        let mut offsets = sample_offsets(shutter, count)?;
        if let Some(seed) = self.jitter_seed {
            let frame = context.time().ticks() as u64;
            for (index, offset) in offsets.iter_mut().enumerate() {
                let random = shutter_random(seed ^ frame.rotate_left(17) ^ index as u64);
                let numerator = (index as f64 + random) / f64::from(count) - 0.5;
                *offset =
                    TimeOffset::from_ticks((numerator * shutter.ticks() as f64).round() as i64);
            }
        }
        let weights = (0..count)
            .map(|index| {
                let phase = (f64::from(index) + 0.5) / f64::from(count);
                match self.curve {
                    ShutterCurve::Box => 1.0,
                    ShutterCurve::Triangle => 1.0 - (phase * 2.0 - 1.0).abs(),
                    ShutterCurve::Cosine => 0.5 - 0.5 * libm::cos(std::f64::consts::TAU * phase),
                }
            })
            .collect::<Vec<_>>();
        let total: f64 = weights.iter().sum();
        let mut nodes = Vec::with_capacity(count as usize);
        for (offset, weight) in offsets.into_iter().zip(weights) {
            let node = build(&context.resampled(offset)?);
            if node.contains_shutter(0) {
                return Err(invalid(
                    "scope",
                    "Do not combine full-scene and subtree shutter captures",
                ));
            }
            nodes.push((node, weight / total));
        }
        Ok(Node::weighted_sum(nodes)?.shutter_sampled())
    }
}

pub(crate) fn shutter_random(mut value: u64) -> f64 {
    value = (value ^ (value >> 30)).wrapping_mul(0xbf58476d1ce4e5b9);
    value = (value ^ (value >> 27)).wrapping_mul(0x94d049bb133111eb);
    value ^= value >> 31;
    (value >> 11) as f64 / (1u64 << 53) as f64
}

fn displacement_samples(first: &crate::DisplayList, last: &crate::DisplayList) -> u32 {
    use crate::DrawCommand;
    if first.commands.len() != last.commands.len() {
        return 64;
    }
    let mut maximum = 0.0f64;
    for (first, last) in first.commands.iter().zip(&last.commands) {
        let points = |command: &DrawCommand| -> Option<Vec<crate::Point>> {
            match command {
                DrawCommand::FillRect {
                    bounds, transform, ..
                } => Some(
                    [
                        crate::Point::new(bounds.left, bounds.top),
                        crate::Point::new(bounds.right, bounds.top),
                        crate::Point::new(bounds.right, bounds.bottom),
                        crate::Point::new(bounds.left, bounds.bottom),
                    ]
                    .map(|point| transform.apply(point))
                    .to_vec(),
                ),
                DrawCommand::Fill {
                    path, transform, ..
                } => Some(
                    path.verbs()
                        .iter()
                        .flat_map(|verb| verb.points().into_iter().flatten())
                        .map(|point| transform.apply(point))
                        .collect(),
                ),
                _ => None,
            }
        };
        match (points(first), points(last)) {
            (Some(first), Some(last)) if first.len() == last.len() => {
                for (first, last) in first.into_iter().zip(last) {
                    maximum = maximum.max(libm::hypot(first.x - last.x, first.y - last.y));
                }
            }
            (None, None) if first == last => {}
            _ => return 64,
        }
    }
    (maximum.ceil() as u32).clamp(8, 64)
}

/// Centered integer midpoint offsets, nearest-tick ties away from zero.
pub fn sample_offsets(shutter: Duration, samples: u32) -> Result<Vec<TimeOffset>, ValidationError> {
    if !(1..=64).contains(&samples) {
        return Err(invalid("samples", "Require 1..=64 samples"));
    }
    let denominator = i128::from(samples) * 2;
    Ok((0..samples)
        .map(|index| {
            let numerator =
                (i128::from(index) * 2 + 1 - i128::from(samples)) * i128::from(shutter.ticks());
            TimeOffset::from_ticks(
                (((numerator.abs() + denominator / 2) / denominator) * numerator.signum()) as i64,
            )
        })
        .collect())
}

pub(crate) fn capture(
    context: &SceneContext<'_>,
    shutter: Duration,
    samples: u32,
    mut build: impl FnMut(&SceneContext<'_>) -> Node,
) -> Result<Node, ValidationError> {
    let offsets = sample_offsets(shutter, samples)?;
    if context.render_quality() == crate::RenderQuality::Draft {
        return Ok(build(context));
    }
    let contexts = offsets
        .into_iter()
        .map(|offset| context.resampled(offset))
        .collect::<Result<Vec<_>, _>>()?;
    let nodes = contexts.iter().map(&mut build).collect::<Vec<_>>();
    Ok(Node::accumulate(nodes)?.shutter_sampled())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{Shape, Time};

    #[test]
    fn jittered_capture_replays_and_rejects_nested_scopes() {
        use crate::NodeModifiers;
        let cache = crate::SceneCache::default();
        let context = SceneContext::new(
            Duration::from_frame(5, crate::fps!(24)).after_origin(),
            crate::fps!(24),
            crate::Size::new(8.0, 8.0),
            &cache,
        );
        let policy = ShutterSampling {
            jitter_seed: Some(17),
            adaptive: true,
            curve: ShutterCurve::Triangle,
        };
        let build = |sample: &SceneContext<'_>| {
            Node::from(crate::Rect::new(0.0, 0.0, 2.0, 2.0).fill(crate::Color::WHITE))
                .translate(sample.seconds(), 0.0)
        };
        let first = policy
            .capture(
                MotionBlur::default(),
                &context,
                crate::PixelSize::new(8, 8),
                build,
            )
            .unwrap();
        let second = policy
            .capture(
                MotionBlur::default(),
                &context,
                crate::PixelSize::new(8, 8),
                build,
            )
            .unwrap();
        assert_eq!(
            first
                .compile(
                    context.size(),
                    crate::PixelSize::new(8, 8),
                    crate::ValidationLimits::default()
                )
                .unwrap(),
            second
                .compile(
                    context.size(),
                    crate::PixelSize::new(8, 8),
                    crate::ValidationLimits::default()
                )
                .unwrap()
        );
        assert!(
            policy
                .capture(
                    MotionBlur::default(),
                    &context,
                    crate::PixelSize::new(8, 8),
                    |_| Node::group([first.clone()])
                )
                .is_err()
        );
        assert!(
            ShutterSampling::default()
                .capture(
                    MotionBlur::default(),
                    &context,
                    crate::PixelSize::new(8, 8),
                    |_| Node::group([first.clone()])
                )
                .is_err(),
            "The default policy rejects nested captures too"
        );
    }

    #[test]
    fn draft_subtree_shutter_samples_once_but_still_validates() {
        let cache = crate::SceneCache::default();
        let context = SceneContext::new(
            Time::from_ticks(0),
            crate::fps!(30),
            crate::Size::new(8.0, 4.0),
            &cache,
        );
        for (quality, expected) in [
            (crate::RenderQuality::Draft, 1),
            (crate::RenderQuality::Final, 4),
        ] {
            let mut calls = 0;
            capture(
                &context.clone().with_quality(quality),
                Duration::from_ticks(100),
                4,
                |_| {
                    calls += 1;
                    Node::empty()
                },
            )
            .unwrap();
            assert_eq!(calls, expected);
        }
        assert!(
            capture(
                &context.clone().with_quality(crate::RenderQuality::Draft),
                Duration::from_ticks(100),
                0,
                |_| panic!("invalid shutter must not evaluate")
            )
            .is_err()
        );
    }
}
