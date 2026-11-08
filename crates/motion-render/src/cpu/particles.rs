//! Ordered per-instance scatter into bounded CPU sampling tiles.

use super::*;
use motion_core::{Point, Transform, ValidationError};

const TILE: u32 = 16;

pub(super) fn tiled(
    batch: &dyn motion_core::instances::InstanceBatch,
    transform: Transform,
    target: &mut Surface,
    space: motion_core::BlendSpace,
    budget: &mut Budget,
    path: &str,
) -> Result<bool, ValidationError> {
    let Some(spatial) = batch.spatial() else {
        return Ok(false);
    };
    let Some(inverse) = transform.inverse() else {
        return Ok(false);
    };
    let forward = transform.coefficients();
    let backward = inverse.coefficients();
    let scale = forward[..4]
        .iter()
        .fold(0.0_f64, |scale, value| scale.max(value.abs()));
    let inverse_scale = backward[..4]
        .iter()
        .fold(0.0_f64, |scale, value| scale.max(value.abs()));
    if scale * inverse_scale > 1e6
        || forward
            .iter()
            .any(|value| !value.is_finite() || value.abs() > 1e10)
    {
        return Ok(false);
    }
    let antialias =
        libm::hypot(backward[0], backward[1]).max(libm::hypot(backward[2], backward[3]));
    let columns = (target.bounds.right - target.bounds.left).div_ceil(TILE);
    let rows = (target.bounds.bottom - target.bounds.top).div_ceil(TILE);
    let count = u64::from(columns) * u64::from(rows);
    let initial_bytes = budget.live_bytes();
    budget.bytes(
        count
            .checked_mul(size_of::<Vec<usize>>() as u64)
            .ok_or_else(|| {
                ValidationError::limit(path, "max_bytes", "Particle tile metadata overflow")
            })?,
        path,
    )?;
    budget.work(count + batch.len() as u64 * 8, path)?;
    let mut bins = Vec::<Vec<usize>>::new();
    let count = usize::try_from(count).map_err(|_| {
        ValidationError::limit(
            path,
            "max_bytes",
            "Particle tile count exceeds address space",
        )
    })?;
    bins.try_reserve_exact(count).map_err(|_| {
        ValidationError::limit(path, "max_bytes", "Could not reserve particle tiles")
    })?;
    bins.resize_with(count, Vec::new);
    for index in 0..batch.len() {
        let local = spatial.support(index, antialias);
        if local.is_empty() {
            continue;
        }
        if ![local.left, local.top, local.right, local.bottom]
            .into_iter()
            .all(f64::is_finite)
        {
            return Err(ValidationError::invalid(
                path,
                "instances.support",
                "Spatial instance support must be finite",
            ));
        }
        let corners = [
            (local.left, local.top),
            (local.right, local.top),
            (local.left, local.bottom),
            (local.right, local.bottom),
        ]
        .map(|(horizontal, vertical)| transform.apply(Point::new(horizontal, vertical)));
        if corners
            .iter()
            .any(|point| !point.x.is_finite() || !point.y.is_finite())
        {
            return Err(ValidationError::invalid(
                path,
                "instances.support",
                "Transformed instance support must be finite",
            ));
        }
        let magnitude = corners.iter().fold(1.0_f64, |value, point| {
            value.max(point.x.abs()).max(point.y.abs())
        });
        let guard = magnitude * (4096.0 * f64::EPSILON) + 1.0;
        let left = corners
            .iter()
            .map(|point| point.x)
            .fold(f64::INFINITY, f64::min)
            - guard;
        let top = corners
            .iter()
            .map(|point| point.y)
            .fold(f64::INFINITY, f64::min)
            - guard;
        let right = corners
            .iter()
            .map(|point| point.x)
            .fold(f64::NEG_INFINITY, f64::max)
            + guard;
        let bottom = corners
            .iter()
            .map(|point| point.y)
            .fold(f64::NEG_INFINITY, f64::max)
            + guard;
        let left = left.floor().clamp(
            f64::from(target.bounds.left),
            f64::from(target.bounds.right),
        ) as u32;
        let top = top.floor().clamp(
            f64::from(target.bounds.top),
            f64::from(target.bounds.bottom),
        ) as u32;
        let right = right.ceil().clamp(
            f64::from(target.bounds.left),
            f64::from(target.bounds.right),
        ) as u32;
        let bottom = bottom.ceil().clamp(
            f64::from(target.bounds.top),
            f64::from(target.bounds.bottom),
        ) as u32;
        if left >= right || top >= bottom {
            continue;
        }
        for row in (top - target.bounds.top) / TILE..=(bottom - 1 - target.bounds.top) / TILE {
            for column in
                (left - target.bounds.left) / TILE..=(right - 1 - target.bounds.left) / TILE
            {
                budget.work(1, path)?;
                budget.push(
                    &mut bins[row as usize * columns as usize + column as usize],
                    index,
                    path,
                )?;
            }
        }
    }
    for row in 0..rows {
        for column in 0..columns {
            let indices = &bins[row as usize * columns as usize + column as usize];
            if indices.is_empty() {
                continue;
            }
            let left = target.bounds.left + column * TILE;
            let top = target.bounds.top + row * TILE;
            let right = left.saturating_add(TILE).min(target.bounds.right);
            let bottom = top.saturating_add(TILE).min(target.bounds.bottom);
            budget.work(
                u64::from(right - left)
                    * u64::from(bottom - top)
                    * (indices.len() as u64
                        * batch.work().div_ceil(batch.len().max(1) as u64).max(64)
                        + 1),
                path,
            )?;
            for vertical in top..bottom {
                for horizontal in left..right {
                    let destination = target.index(horizontal, vertical);
                    let point = inverse.apply(Point::new(
                        f64::from(horizontal) + 0.5,
                        f64::from(vertical) + 0.5,
                    ));
                    target.pixels[destination] = spatial.sample_indices(
                        indices,
                        point,
                        antialias,
                        target.pixels[destination],
                        space,
                        particle_compositor(batch.blend()),
                    )?;
                }
            }
        }
    }
    budget.set_live_bytes(initial_bytes);
    Ok(true)
}

#[cfg(test)]
mod tests {
    use super::*;
    use motion_core::instances::{InstanceBatch, InstanceComposite, SpatialInstances};
    use motion_core::{BlendSpace, Rect};

    #[derive(Debug)]
    struct Fixture(bool);

    impl InstanceBatch for Fixture {
        fn len(&self) -> usize {
            2
        }

        fn bounds(&self) -> Rect {
            Rect::new(0.0, 0.0, 16.0, 16.0)
        }

        fn bytes(&self) -> u64 {
            0
        }

        fn work(&self) -> u64 {
            129
        }

        fn validate(&self) -> Result<(), ValidationError> {
            Ok(())
        }

        fn spatial(&self) -> Option<&dyn SpatialInstances> {
            if self.0 { Some(self) } else { None }
        }

        fn sample(
            &self,
            point: Point,
            antialias: f64,
            destination: [f32; 4],
            space: BlendSpace,
            composite: InstanceComposite,
        ) -> Result<[f32; 4], ValidationError> {
            self.sample_indices(&[0, 1], point, antialias, destination, space, composite)
        }

        fn as_any(&self) -> &dyn std::any::Any {
            self
        }

        fn content_eq(&self, other: &dyn InstanceBatch) -> bool {
            other
                .as_any()
                .downcast_ref::<Self>()
                .is_some_and(|other| self.0 == other.0)
        }
    }

    impl SpatialInstances for Fixture {
        fn support(&self, index: usize, _: f64) -> Rect {
            Rect::new(
                4.0 + index as f64 * 4.0,
                4.0,
                12.0 + index as f64 * 4.0,
                12.0,
            )
        }

        fn sample_indices(
            &self,
            indices: &[usize],
            point: Point,
            antialias: f64,
            mut destination: [f32; 4],
            space: BlendSpace,
            composite: InstanceComposite,
        ) -> Result<[f32; 4], ValidationError> {
            for &index in indices {
                let bounds = self.support(index, antialias);
                if point.x >= bounds.left
                    && point.x <= bounds.right
                    && point.y >= bounds.top
                    && point.y <= bounds.bottom
                {
                    composite(
                        &mut destination,
                        if index == 0 {
                            [0.25, 0.0, 0.0, 0.5]
                        } else {
                            [0.0, 0.25, 0.0, 0.5]
                        },
                        space,
                    )?;
                }
            }
            Ok(destination)
        }
    }

    fn setup() -> Result<(Surface, Budget), ValidationError> {
        let mut budget = Budget::new(PixelSize::new(16, 16), RasterLimits::default())?;
        let surface = Surface::new(
            PixelRect {
                left: 0,
                top: 0,
                right: 16,
                bottom: 16,
            },
            1.0,
            motion_core::BlendMode::SourceOver,
            [0.1, 0.2, 0.3, 1.0],
            &mut budget,
            "test",
        )?;
        Ok((surface, budget))
    }

    #[test]
    fn tiles_preserve_scalar_order_backdrop_and_transient_accounting() -> Result<(), ValidationError>
    {
        let (mut surface, mut budget) = setup()?;
        let initial = budget.live_bytes();
        let fixture = Fixture(true);
        let mut expected = surface.pixels.clone();
        for row in 0..16 {
            for column in 0..16 {
                let index = surface.index(column, row);
                expected[index] = fixture.sample(
                    Point::new(f64::from(column) + 0.5, f64::from(row) + 0.5),
                    1.0,
                    expected[index],
                    BlendSpace::LinearRgb,
                    crate::cpu::particle_compositor(
                        motion_core::instances::InstanceBlend::SourceOver,
                    ),
                )?;
            }
        }
        assert!(tiled(
            &fixture,
            Transform::IDENTITY,
            &mut surface,
            BlendSpace::LinearRgb,
            &mut budget,
            "test"
        )?);
        assert_eq!(surface.pixels, expected);
        assert_eq!(budget.live_bytes(), initial);
        assert!(budget.peak_bytes() > initial);
        Ok(())
    }

    #[test]
    fn failed_admission_and_fallback_leave_pixels_unchanged() -> Result<(), ValidationError> {
        for field in ["max_bytes", "max_work"] {
            let (mut surface, mut budget) = setup()?;
            let before = surface.pixels.clone();
            if field == "max_bytes" {
                budget.limits.max_bytes = budget.live_bytes();
            } else {
                budget.limits.max_work = budget.work_done() + 1;
            }
            let error = tiled(
                &Fixture(true),
                Transform::IDENTITY,
                &mut surface,
                BlendSpace::LinearRgb,
                &mut budget,
                "test",
            )
            .unwrap_err();
            assert_eq!(error.field, field);
            assert_eq!(surface.pixels, before);
        }
        for (fixture, transform) in [
            (Fixture(false), Transform::IDENTITY),
            (Fixture(true), Transform::scale_non_uniform(1e-8, 1e8)),
        ] {
            let (mut surface, mut budget) = setup()?;
            let before = surface.pixels.clone();
            let work = budget.work_done();
            assert!(!tiled(
                &fixture,
                transform,
                &mut surface,
                BlendSpace::LinearRgb,
                &mut budget,
                "test"
            )?);
            assert_eq!(surface.pixels, before);
            assert_eq!(budget.work_done(), work);
        }
        Ok(())
    }
}
