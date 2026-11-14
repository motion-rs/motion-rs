//! Validated immutable vector and inline image particle materials.

use super::Compiler;
use crate::particles::{ParticleImageSpec, ParticlePointsSpec, ParticleSurfaceSpec};
use motion_content::{ImageAsset, VectorGroup, VectorItem, VectorPath, VectorTree};
use motion_core::{Point, Rect, Time};
use std::sync::Arc;

/// Most painted paths retained in one particle surface.
const MAX_SURFACE_PATHS: usize = 256;

impl Compiler<'_> {
    pub(super) fn particle_points(
        &mut self,
        spec: &ParticlePointsSpec,
        path: &str,
    ) -> Option<Vec<motion_core::Point>> {
        if matches!(spec, ParticlePointsSpec::Fixed(points) if points.len() > 65_536) {
            self.limit(
                path,
                "points",
                "Particle target sets take at most 65536 points",
            );
            return None;
        }
        let source = match spec {
            ParticlePointsSpec::Fixed(points) => {
                crate::content::PointsSpec::new(points.iter().map(|[x, y]| Point::new(*x, *y)))
            }
            ParticlePointsSpec::Sampled(source) => source.clone(),
        };
        if source.animated.is_some() {
            self.error(
                path,
                "animated",
                "Particle target points must be fixed at compilation",
            );
            return None;
        }
        let points = self.point_source(&source, path)?;
        if points.is_empty() {
            self.error(path, "points", "Particle targets cannot be empty");
            return None;
        }
        Some(points)
    }

    pub(super) fn particle_image(
        &mut self,
        spec: &ParticleImageSpec,
        path: &str,
    ) -> Option<Arc<ImageAsset>> {
        self.inline_image(spec, path)
    }

    pub(super) fn particle_surface(
        &mut self,
        spec: &ParticleSurfaceSpec,
        path: &str,
    ) -> Option<Arc<VectorTree>> {
        if spec.paths.is_empty() || spec.paths.len() > MAX_SURFACE_PATHS {
            self.limit(
                path,
                "paths",
                "Particle surfaces take 1 to 256 painted paths",
            );
            return None;
        }
        let [left, top, right, bottom] = spec.bounds;
        let bounds = Rect::new(left, top, right, bottom);
        motion_core::drawing::validate_rect(bounds, path, "bounds")
            .map_err(|error| self.error(path, "bounds", error.to_string()))
            .ok()?;
        let mut children = Vec::with_capacity(spec.paths.len());
        for (index, spec) in spec.paths.iter().enumerate() {
            let path = format!("{path}.paths[{index}]");
            let geometry = self.svg_path(&spec.data, &format!("{path}.data"))?;
            let paint = self.paint(&spec.fill, &format!("{path}.fill"))?;
            if paint.varying() {
                self.error(&path, "fill", "Particle surface paints must be fixed");
                return None;
            }
            let paint = paint
                .at(Time::ZERO)
                .map_err(|error| self.error(&path, "fill", error.message))
                .ok()?;
            children.push(VectorItem::Path(VectorPath::filled(geometry, paint)));
        }
        VectorTree::new(
            bounds,
            VectorGroup {
                children,
                ..Default::default()
            },
        )
        .map(Arc::new)
        .map_err(|error| self.error(path, "surface", error.to_string()))
        .ok()
    }
}
