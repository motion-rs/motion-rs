//! Core-backed spring and path motion, expression maps and bounded animation clocks.

use super::Compiler;
use super::expr::parse;
use super::value::Value;
use crate::{ClockSpec, FollowPathSpec, MapSpec, PathComponent, SpringSpec};
use motion_core::{FollowPath, Signal, Spring, Time};
use std::sync::Arc;

/// Most numeric sources one component or sum motion may combine.
const MAX_NUMERIC_SOURCES: usize = 32;

pub(crate) fn numeric_animation(
    animation: &crate::Animated<f64>,
) -> motion_core::Result<Arc<dyn Signal<f64>>> {
    let parameters = std::collections::BTreeMap::new();
    let mut compiler = Compiler::new(None, &parameters);
    let property = compiler.animated(animation, "animation");
    if !compiler.errors.is_empty() {
        return Err(compiler.into_report().into());
    }
    match property {
        Some(super::value::Prop::Fixed(value)) => Ok(Arc::new(move |_: Time| value)),
        Some(super::value::Prop::Varying(signal)) => Ok(signal),
        None => Err(compiler.into_report().into()),
    }
}

impl Compiler<'_> {
    pub(super) fn numeric_sources(
        &mut self,
        values: &[crate::Animated<f64>],
        path: &str,
    ) -> Option<Vec<Arc<dyn Signal<f64>>>> {
        if values.is_empty() || values.len() > MAX_NUMERIC_SOURCES {
            self.limit(path, "sources", "Motion requires 1..=32 numeric sources");
            return None;
        }
        values
            .iter()
            .enumerate()
            .map(|(index, source)| {
                let property = self.animated(source, &format!("{path}[{index}]"))?;
                Some(match property {
                    super::value::Prop::Fixed(value) => {
                        Arc::new(move |_: Time| value) as Arc<dyn Signal<f64>>
                    }
                    super::value::Prop::Varying(signal) => signal,
                })
            })
            .collect()
    }

    pub(super) fn spring<T: Value>(
        &mut self,
        spec: &SpringSpec<T>,
        path: &str,
    ) -> Option<Arc<dyn Signal<T::Engine>>> {
        let from = spec
            .from
            .engine()
            .map_err(|message| self.error(&format!("{path}.from"), "value", message))
            .ok();
        let to = spec
            .to
            .engine()
            .map_err(|message| self.error(&format!("{path}.to"), "value", message))
            .ok();
        let mut builder = Spring::builder(from?, to?);
        if let Some(value) = spec.mass {
            builder = builder.mass(value);
        }
        if let Some(value) = spec.stiffness {
            builder = builder.stiffness(value);
        }
        if let Some(value) = spec.damping {
            builder = builder.damping(value);
        }
        if let Some(value) = spec.initial_velocity {
            builder = builder.initial_velocity(value);
        }
        if let Some(value) = spec.settling_threshold {
            builder = builder.settling_threshold(value);
        }
        let spring = builder
            .build()
            .map_err(|error| self.error(path, &error.field, error.message))
            .ok()?;
        self.seconds(spring.settle_duration().as_seconds(), path, "duration")?;
        Some(Arc::new(spring))
    }

    pub(super) fn clock<T: Value>(
        &mut self,
        spec: &ClockSpec<T>,
        path: &str,
    ) -> Option<Arc<dyn Signal<T::Engine>>> {
        let source = self.animated(&spec.source, &format!("{path}.source"));
        let scale = self.bounded(
            spec.scale,
            -super::MAX_SECONDS,
            super::MAX_SECONDS,
            path,
            "scale",
        );
        let offset = self.bounded(
            spec.offset,
            -super::MAX_SECONDS,
            super::MAX_SECONDS,
            path,
            "offset",
        );
        let (source, scale, offset) = (source?, scale?, offset?);
        Some(Arc::new(move |time: Time| {
            let seconds =
                (time.as_seconds() * scale + offset).clamp(-super::MAX_SECONDS, super::MAX_SECONDS);
            source.at(Time::from_seconds(seconds))
        }))
    }

    pub(super) fn map_animation(
        &mut self,
        spec: &MapSpec,
        path: &str,
    ) -> Option<Arc<dyn Signal<f64>>> {
        let source = self.animated(&spec.source, &format!("{path}.source"));
        let parsed = parse(&spec.expr, self.params)
            .map_err(|message| self.error(&format!("{path}.expr"), "expr", message))
            .ok()?;
        if !parsed.references.is_empty() {
            self.error(
                path,
                "expr",
                "Mapping expressions cannot reference elements",
            );
            return None;
        }
        let source = source?;
        Some(Arc::new(move |time: Time| {
            parsed.expr.eval(source.at(time), &[])
        }))
    }

    pub(super) fn follow_path(
        &mut self,
        spec: &FollowPathSpec,
        path: &str,
    ) -> Option<Arc<dyn Signal<f64>>> {
        let geometry = self.svg_path(&spec.data, &format!("{path}.data"));
        let duration = self.seconds(spec.duration, path, "duration");
        let easing = self.easing(spec.easing, path);
        let motion = FollowPath::new(geometry?, duration?).easing(easing?);
        let component = spec.component;
        Some(Arc::new(move |time: Time| {
            let sample = motion.sample(time);
            match component {
                PathComponent::X => sample.position.x,
                PathComponent::Y => sample.position.y,
                PathComponent::Angle => sample.angle.to_degrees(),
            }
        }))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{Animated, EasingSpec};

    #[test]
    fn spring_clock_and_mapping_match_core_samples() {
        for damping in [9.0, 22.0, 38.0] {
            let mut spec = SpringSpec::new(0.0, 1.0);
            spec.stiffness = Some(121.0);
            spec.damping = Some(damping);
            let animation = Animated::from(spec).clock(2.0, -0.2).map_expr("t * t");
            let restored: Animated<f64> =
                serde_json::from_str(&serde_json::to_string(&animation).unwrap()).unwrap();
            assert_eq!(animation, restored);
            let actual = restored.compile().unwrap();
            let spring = Spring::builder(0.0, 1.0)
                .stiffness(121.0)
                .damping(damping)
                .build()
                .unwrap();
            for seconds in [0.0, 0.1, 0.6, 2.0, -1.0, 0.6] {
                let time = Time::from_seconds(seconds);
                let value = spring.sample(Time::from_seconds(time.as_seconds() * 2.0 - 0.2));
                assert_eq!(actual.sample(time), value * value);
            }
        }
    }

    #[test]
    fn path_components_match_core_and_use_degrees() {
        let data = "M 0 0 C 0 20 30 20 30 0";
        let motion = FollowPath::new(
            motion_core::Path2D::parse_svg(data).unwrap(),
            motion_core::Duration::from_seconds(2.0),
        )
        .easing(motion_core::Easing::SineInOut);
        for component in [PathComponent::X, PathComponent::Y, PathComponent::Angle] {
            let mut spec = FollowPathSpec::new(data, 2.0, component);
            spec.easing = EasingSpec::SineInOut;
            let actual = Animated::from(spec).compile().unwrap();
            for seconds in [-1.0, 0.0, 0.4, 2.0, 3.0] {
                let time = Time::from_seconds(seconds);
                let sample = motion.sample(time);
                let expected = match component {
                    PathComponent::X => sample.position.x,
                    PathComponent::Y => sample.position.y,
                    PathComponent::Angle => sample.angle.to_degrees(),
                };
                assert_eq!(actual.sample(time), expected);
            }
        }
    }

    #[test]
    fn malformed_and_deep_compositions_report_errors() {
        let mut spring = SpringSpec::new(0.0, 1.0);
        spring.damping = Some(0.0);
        assert!(Animated::from(spring).compile().is_err());
        let mut slow = SpringSpec::new(0.0, 1.0);
        slow.stiffness = Some(1e-12);
        slow.damping = Some(2e-6);
        assert!(Animated::from(slow).compile().is_err());
        assert!(Animated::from(1.0).clock(f64::NAN, 0.0).compile().is_err());
        assert!(
            Animated::from(1.0)
                .map_expr("@missing.rotate")
                .compile()
                .is_err()
        );
        let mut animation = Animated::from(1.0);
        for _ in 0..33 {
            animation = animation.clock(1.0, 0.0);
        }
        assert!(
            animation
                .compile()
                .err()
                .unwrap()
                .to_string()
                .contains("32 levels")
        );
    }
}
