//! Compilation of bounded population, collision and field operators.

use super::Compiler;
use super::particle_events::GENERATION_SLOT;
use crate::particles::{ParticleFalloffSpec, ParticleOperatorSpec};
use motion_content::particles::{
    Boids, CircleCollider, CollisionResponse, ColorField, ColorOverLifetime, FlowField, FlowSource,
    ForceFalloff, Particle, ParticleCollisions, ParticleModifier, ParticleSimulationBuilder,
    PolygonCollider, RadialForce, SdfCollider, SegmentCollider, SignedDistanceGrid,
    SimulationLimits, Spring,
};
use motion_core::{Point, Random, Rect, Time, ValidationError};
use std::sync::Arc;

/// Maximum colors or obstacle vertices admitted by one operator.
const MAX_OPERATOR_ITEMS: usize = 256;

impl Compiler<'_> {
    pub(super) fn particle_response(
        &mut self,
        spec: CollisionResponse,
        path: &str,
    ) -> Option<CollisionResponse> {
        self.bounded(
            f64::from(spec.restitution),
            0.0,
            1.0,
            path,
            "response.restitution",
        )?;
        self.bounded(
            f64::from(spec.friction),
            0.0,
            1.0,
            path,
            "response.friction",
        )?;
        Some(spec)
    }

    pub(super) fn particle_operator(
        &mut self,
        simulation: ParticleSimulationBuilder,
        spec: &ParticleOperatorSpec,
        path: &str,
    ) -> Option<ParticleSimulationBuilder> {
        Some(match spec {
            ParticleOperatorSpec::SeekTargets { targets, strength } => {
                simulation.modifier(motion_content::particles::Seek {
                    targets: self.particle_points(targets, path)?.into(),
                    strength: *strength,
                })
            }
            ParticleOperatorSpec::RadialForce {
                center,
                strength,
                radius,
                falloff,
            } => {
                let falloff = match falloff {
                    ParticleFalloffSpec::Constant => ForceFalloff::Constant,
                    ParticleFalloffSpec::Linear => ForceFalloff::Linear,
                    ParticleFalloffSpec::InverseSquare => ForceFalloff::InverseSquare,
                    ParticleFalloffSpec::Curve(curve) => {
                        ForceFalloff::Curve(self.particle_curve(curve, path)?)
                    }
                };
                simulation.modifier(RadialForce {
                    center: Point::new(center[0], center[1]),
                    strength: *strength,
                    radius: *radius,
                    falloff,
                })
            }
            ParticleOperatorSpec::ImageFlow {
                image,
                bounds,
                strength,
                phase_speed,
            } => {
                let [left, top, right, bottom] = *bounds;
                simulation.modifier(FlowField {
                    source: FlowSource::Image {
                        image: self.particle_image(image, path)?,
                        bounds: Rect::new(left, top, right, bottom),
                    },
                    strength: *strength,
                    phase_speed: *phase_speed,
                })
            }
            ParticleOperatorSpec::CurlFlow {
                seed,
                frequency,
                strength,
            } => simulation.modifier(FlowField::curl(
                self.noise_seed(*seed, path, "curl_flow.seed")?,
                *frequency,
                *strength,
            )),
            ParticleOperatorSpec::Boids {
                radius,
                separation,
                alignment,
                cohesion,
            } => simulation.group_modifier(Boids {
                radius: *radius,
                separation: *separation,
                alignment: *alignment,
                cohesion: *cohesion,
                ..Default::default()
            }),
            ParticleOperatorSpec::Spring {
                rest_length,
                stiffness,
                damping,
                closed,
            } => simulation.group_modifier(Spring {
                rest_length: *rest_length,
                stiffness: *stiffness,
                damping: *damping,
                closed: *closed,
            }),
            ParticleOperatorSpec::ParticleCollisions(response) => {
                simulation.group_modifier(ParticleCollisions {
                    response: self.particle_response(*response, path)?,
                })
            }
            ParticleOperatorSpec::CircleCollider {
                center,
                radius,
                response,
            } => simulation.modifier(CircleCollider {
                center: Point::new(center[0], center[1]),
                radius: *radius,
                response: self.particle_response(*response, path)?,
            }),
            ParticleOperatorSpec::SegmentCollider {
                start,
                end,
                response,
            } => simulation.modifier(SegmentCollider {
                start: Point::new(start[0], start[1]),
                end: Point::new(end[0], end[1]),
                response: self.particle_response(*response, path)?,
            }),
            ParticleOperatorSpec::PolygonCollider { vertices, response } => {
                if !(3..=MAX_OPERATOR_ITEMS).contains(&vertices.len()) {
                    self.limit(path, "vertices", "Polygon colliders take 3 to 256 vertices");
                    return None;
                }
                let response = self.particle_response(*response, path)?;
                let vertices = vertices.iter().map(|[x, y]| Point::new(*x, *y)).collect();
                let collider = PolygonCollider::new(vertices, response)
                    .map_err(|error| self.error(path, "polygon_collider", error.to_string()))
                    .ok()?;
                simulation.modifier(collider)
            }
            ParticleOperatorSpec::SdfCollider {
                surface,
                resolution,
                response,
            } => {
                if resolution.iter().any(|side| !(2..=512).contains(side)) {
                    self.limit(path, "resolution", "SDF grid sides must be in 2..=512");
                    return None;
                }
                let tree = self.particle_surface(surface, path)?;
                let shape = tree
                    .surface()
                    .map_err(|error| self.error(path, "surface", error.to_string()))
                    .ok()?;
                let [left, top, right, bottom] = surface.bounds;
                let grid = SignedDistanceGrid::new(
                    shape,
                    Rect::new(left, top, right, bottom),
                    *resolution,
                    SimulationLimits::default(),
                )
                .map_err(|error| self.error(path, "sdf_collider", error.to_string()))
                .ok()?;
                simulation.modifier(SdfCollider {
                    grid: Arc::new(grid),
                    response: self.particle_response(*response, path)?,
                })
            }
            ParticleOperatorSpec::ColorCurve { colors, easing } => {
                self.particle_colors(colors, path)?;
                let easing = self.easing(*easing, path)?;
                simulation.modifier(ColorOverLifetime::new(Arc::from(colors.clone())).curve(easing))
            }
            ParticleOperatorSpec::ColorField {
                coordinate,
                range,
                colors,
            } => {
                self.particle_colors(colors, path)?;
                simulation.modifier(ColorField {
                    coordinate: *coordinate,
                    range: *range,
                    colors: ColorOverLifetime::new(Arc::from(colors.clone()))
                        .curve(motion_core::Easing::Linear),
                })
            }
            ParticleOperatorSpec::CustomSlot {
                slot,
                modulus,
                scale,
            } => {
                if usize::from(*slot) >= GENERATION_SLOT
                    || *modulus == Some(0)
                    || !scale.is_finite()
                {
                    self.error(
                        path,
                        "custom_slot",
                        "Require slot 0..=2 (slot 3 is reserved), positive optional modulus and finite scale",
                    );
                    return None;
                }
                simulation.modifier(CustomSlot {
                    slot: usize::from(*slot),
                    modulus: *modulus,
                    scale: *scale,
                })
            }
        })
    }

    fn particle_colors(&mut self, colors: &[[f32; 4]], path: &str) -> Option<()> {
        if colors.is_empty()
            || colors.len() > MAX_OPERATOR_ITEMS
            || colors.iter().any(|color| {
                color.iter().any(|value| !value.is_finite() || *value < 0.0) || color[3] > 1.0
            })
        {
            self.limit(
                path,
                "colors",
                "Require 1 to 256 finite nonnegative premultiplied colors with alpha in 0..=1",
            );
            return None;
        }
        Some(())
    }
}

struct CustomSlot {
    slot: usize,
    modulus: Option<u32>,
    scale: f32,
}

impl ParticleModifier for CustomSlot {
    fn apply(
        &self,
        particle: &mut Particle,
        _: f32,
        _: Time,
        _: &mut Random,
    ) -> Result<(), ValidationError> {
        let value = self.modulus.map_or(particle.index, |modulus| {
            particle.index % u64::from(modulus)
        }) as f32
            * self.scale;
        if !value.is_finite() {
            return Err(ValidationError::invalid(
                "particles",
                "custom_slot",
                "Custom slot exceeds finite particle support",
            ));
        }
        particle.custom[self.slot] = value;
        Ok(())
    }
}
