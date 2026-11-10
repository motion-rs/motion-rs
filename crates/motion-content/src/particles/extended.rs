//! Versioned particle geometry batches with conservative support and ordered CPU/GPU sampling.

use super::{MAX_INSTANCE_COUNT, ValidationError, invalid, limit};
use motion_core::instances::{
    ExtendedInstanceRecord, InstanceBatch, InstanceBlend, InstanceComposite, InstanceUpload,
    SpatialInstances,
};
use motion_core::{BlendSpace, Drawable, Point, Rect, Transform};
use std::sync::Arc;

mod build;
mod sampling;

pub(super) use build::render;

use sampling::{rotation, sample_record, support};

#[derive(Clone, Debug)]
struct ExtendedBatch {
    records: Arc<[ExtendedInstanceRecord]>,
    sprite: Option<Arc<crate::ImageAsset>>,
    bounds: Rect,
    blend: InstanceBlend,
    /// Per-record `[cos, sin]` of the rotation, derived from `records` once per batch.
    rotations: Arc<[[f64; 2]]>,
}

impl ExtendedBatch {
    fn new(
        records: Arc<[ExtendedInstanceRecord]>,
        sprite: Option<Arc<crate::ImageAsset>>,
        bounds: Rect,
        blend: InstanceBlend,
    ) -> Self {
        let rotations = records.iter().map(rotation).collect();
        Self {
            records,
            sprite,
            bounds,
            blend,
            rotations,
        }
    }
}

impl PartialEq for ExtendedBatch {
    fn eq(&self, other: &Self) -> bool {
        self.records == other.records
            && self.bounds == other.bounds
            && self.blend == other.blend
            && match (&self.sprite, &other.sprite) {
                (None, None) => true,
                (Some(first), Some(second)) => Arc::ptr_eq(first, second),
                _ => false,
            }
    }
}

impl InstanceBatch for ExtendedBatch {
    fn len(&self) -> usize {
        self.records.len()
    }

    fn bounds(&self) -> Rect {
        self.bounds
    }

    fn blend(&self) -> InstanceBlend {
        self.blend
    }

    fn bytes(&self) -> u64 {
        std::mem::size_of::<Self>() as u64
            + self.records.len() as u64
                * (std::mem::size_of::<ExtendedInstanceRecord>() + std::mem::size_of::<[f64; 2]>())
                    as u64
            + self.sprite.as_ref().map_or(0, |image| {
                u64::from(image.width()) * u64::from(image.height()) * 16
            })
    }

    fn work(&self) -> u64 {
        self.records.len() as u64 * 128 + 1
    }

    fn spatial(&self) -> Option<&dyn SpatialInstances> {
        Some(self)
    }

    fn validate(&self) -> Result<(), ValidationError> {
        if self.len() > MAX_INSTANCE_COUNT {
            return Err(limit("Extended instance count exhausted"));
        }
        for record in self.records.iter() {
            record.validate()?;
            let base = record.base;
            if base
                .position
                .iter()
                .chain(&base.start)
                .chain(&base.color)
                .chain(&record.extra)
                .chain([&base.radius, &base.rotation, &base.width])
                .any(|value| !value.is_finite())
                || base.radius < 0.0
                || base.radius > 262144.0
                || base.width < 0.0
                || base.color[3] < 0.0
                || base.color[3] > 1.0
            {
                return Err(invalid("instances", "Invalid extended particle record"));
            }
        }
        motion_core::drawing::validate_rect(self.bounds, "particles", "bounds")
    }

    fn sample(
        &self,
        point: Point,
        antialias: f64,
        mut destination: [f32; 4],
        space: BlendSpace,
        composite: InstanceComposite,
    ) -> Result<[f32; 4], ValidationError> {
        for (record, rotation) in self.records.iter().zip(self.rotations.iter()) {
            composite(
                &mut destination,
                sample_record(record, *rotation, self.sprite.as_deref(), point, antialias)?,
                space,
            )?;
        }
        Ok(destination)
    }

    fn upload_bytes(&self) -> u64 {
        (4 + self.records.len() as u64 * 16
            + self.sprite.as_ref().map_or(0, |image| {
                u64::from(image.width()) * u64::from(image.height()) * 4
            }))
            * 4
    }

    fn upload(&self) -> Result<Option<InstanceUpload>, ValidationError> {
        let size = self
            .sprite
            .as_ref()
            .map_or([0, 0], |image| [image.width(), image.height()]);
        let mut texels = Vec::new();
        texels
            .try_reserve_exact(size[0] as usize * size[1] as usize)
            .map_err(|_| limit("Sprite upload allocation failed"))?;
        if let Some(image) = &self.sprite {
            for row in 0..image.height() {
                for column in 0..image.width() {
                    texels.push(
                        image.levels()[0]
                            .pixel(column, row)
                            .ok_or_else(|| invalid("sprite", "Missing validated sprite texel"))?,
                    );
                }
            }
        }
        InstanceUpload::new_extended(self.records.iter().copied(), size, texels, self.blend)
            .map(Some)
    }

    fn as_any(&self) -> &dyn std::any::Any {
        self
    }

    fn content_eq(&self, other: &dyn InstanceBatch) -> bool {
        other
            .as_any()
            .downcast_ref::<Self>()
            .is_some_and(|other| self == other)
    }
}

impl SpatialInstances for ExtendedBatch {
    fn support(&self, index: usize, antialias: f64) -> Rect {
        support(&self.records[index], antialias)
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
        for index in indices {
            let source = sample_record(
                &self.records[*index],
                self.rotations[*index],
                self.sprite.as_deref(),
                point,
                antialias,
            )?;
            if source != [0.0; 4] {
                composite(&mut destination, source, space)?;
            }
        }
        Ok(destination)
    }
}

impl Drawable for ExtendedBatch {
    fn bounds(&self, context: &motion_core::DrawContext) -> Result<Rect, ValidationError> {
        let margin = 1.0 / context.device_scale();
        Ok(Rect::new(
            self.bounds.left - margin,
            self.bounds.top - margin,
            self.bounds.right + margin,
            self.bounds.bottom + margin,
        ))
    }

    fn draw(
        &self,
        _: &motion_core::DrawContext,
        canvas: &mut motion_core::Canvas<'_>,
    ) -> Result<(), ValidationError> {
        canvas.instances(Arc::new(self.clone()), Transform::IDENTITY)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use motion_core::instances::{InstanceKind, InstanceRecord};

    fn over(
        destination: &mut [f32; 4],
        source: [f32; 4],
        _: BlendSpace,
    ) -> Result<(), ValidationError> {
        for channel in 0..4 {
            destination[channel] = source[channel] + destination[channel] * (1.0 - source[3]);
        }
        Ok(())
    }

    #[test]
    fn cached_rotations_follow_their_records_in_both_sampling_paths() -> Result<(), ValidationError>
    {
        let records: Vec<_> = [0.0_f32, 0.7, -2.3, 3.1]
            .into_iter()
            .enumerate()
            .map(|(index, rotation)| ExtendedInstanceRecord {
                base: InstanceRecord {
                    position: [8.0 + index as f32 * 3.0, 8.0],
                    start: [0.0; 2],
                    color: [0.2, 0.4, 0.6, 0.8],
                    radius: 4.0,
                    rotation,
                    width: 0.0,
                    kind: InstanceKind::Quad,
                },
                extra: [0.0, 0.0, 2.5, 0.0],
            })
            .collect();
        let batch = ExtendedBatch::new(
            records.clone().into(),
            None,
            Rect::new(0.0, 0.0, 32.0, 16.0),
            InstanceBlend::SourceOver,
        );
        let indices: Vec<_> = (0..records.len()).rev().collect();
        for row in 0..16 {
            for column in 0..32 {
                let point = Point::new(f64::from(column) + 0.5, f64::from(row) + 0.5);
                let mut expected = [0.0; 4];
                for record in &records {
                    over(
                        &mut expected,
                        sample_record(record, rotation(record), None, point, 1.0)?,
                        BlendSpace::LinearRgb,
                    )?;
                }
                let actual = batch.sample(point, 1.0, [0.0; 4], BlendSpace::LinearRgb, over)?;
                assert_eq!(actual.map(f32::to_bits), expected.map(f32::to_bits));
                let mut reversed = [0.0; 4];
                for index in &indices {
                    let source = sample_record(
                        &records[*index],
                        rotation(&records[*index]),
                        None,
                        point,
                        1.0,
                    )?;
                    if source != [0.0; 4] {
                        over(&mut reversed, source, BlendSpace::LinearRgb)?;
                    }
                }
                let actual = batch.sample_indices(
                    &indices,
                    point,
                    1.0,
                    [0.0; 4],
                    BlendSpace::LinearRgb,
                    over,
                )?;
                assert_eq!(actual.map(f32::to_bits), reversed.map(f32::to_bits));
            }
        }
        Ok(())
    }
}
