//! Validated particle instances and spatial indexing.

use super::*;
use motion_core::{Drawable, Point, Rect, Transform};

#[derive(Clone, Debug, PartialEq)]
pub(super) struct Instance {
    pub(super) particle: Particle,
    pub(super) previous: [f32; 2],
    pub(super) line: bool,
    pub(super) line_bounds: [f32; 4],
    pub(super) line_guard: f64,
}

impl Instance {
    pub(super) fn new(particle: Particle, previous: [f32; 2], line: bool) -> Self {
        let line_bounds = if line {
            [
                previous[0].min(particle.position[0]),
                previous[1].min(particle.position[1]),
                previous[0].max(particle.position[0]),
                previous[1].max(particle.position[1]),
            ]
        } else {
            [0.0; 4]
        };
        let line_guard = if line {
            previous
                .map(f64::from)
                .into_iter()
                .chain(particle.position.map(f64::from))
                .fold(1.0_f64, |scale, coordinate| scale.max(coordinate.abs()))
                * (128.0 * f64::EPSILON)
        } else {
            0.0
        };
        Self {
            particle,
            previous,
            line,
            line_bounds,
            line_guard,
        }
    }
}

#[derive(Clone, Debug)]
pub(super) struct SpatialNode {
    pub(super) bounds: Rect,
    pub(super) guard: f64,
    pub(super) end: usize,
    pub(super) next: usize,
    pub(super) leaf: bool,
}

impl SpatialNode {
    pub(super) fn outside(&self, point: Point, margin: f64, point_guard: f64) -> bool {
        let padding = margin + self.guard + point_guard;
        point.x < self.bounds.left - padding
            || point.x > self.bounds.right + padding
            || point.y < self.bounds.top - padding
            || point.y > self.bounds.bottom + padding
    }
}

/// Validated opaque instance payload, consumed directly by CPU/GPU paths.
#[derive(Clone, Debug)]
pub struct ParticleBatch {
    pub(super) instances: Arc<[Instance]>,
    pub(super) kind: u32,
    pub(super) radius: f32,
    pub(super) width: f32,
    pub(super) align: bool,
    pub(super) sprite: Option<Arc<crate::ImageAsset>>,
    pub(super) bounds: Rect,
    pub(super) index: Arc<[SpatialNode]>,
}

impl PartialEq for ParticleBatch {
    fn eq(&self, other: &Self) -> bool {
        self.instances == other.instances
            && self.kind == other.kind
            && self.radius == other.radius
            && self.width == other.width
            && self.align == other.align
            && self.sprite.as_ref().map(Arc::as_ptr) == other.sprite.as_ref().map(Arc::as_ptr)
            && self.bounds == other.bounds
    }
}

impl ParticleBatch {
    /// Number of dot/sprite/segment instances, including Plexus links.
    pub fn len(&self) -> usize {
        self.instances.len()
    }

    /// Whether this payload contains no instances.
    pub fn is_empty(&self) -> bool {
        self.instances.is_empty()
    }

    /// Conservative local geometric extent (before output antialias padding).
    pub fn bounds(&self) -> Rect {
        self.bounds
    }

    pub(crate) fn bytes(&self) -> u64 {
        self.instances.len() as u64 * std::mem::size_of::<Instance>() as u64
            + self.index.len() as u64 * std::mem::size_of::<SpatialNode>() as u64
            + std::mem::size_of::<Arc<[SpatialNode]>>() as u64
            + self.sprite.as_ref().map_or(0, |sprite| {
                u64::from(sprite.width()) * u64::from(sprite.height()) * 16
            })
    }

    pub(crate) fn work(&self) -> u64 {
        self.instances.len() as u64 * 64 + self.index.len() as u64 * 16 + 1
    }

    pub(super) fn build_spatial_index(&self) -> Arc<[SpatialNode]> {
        if self.kind == 2 || self.instances.len() < 32 || self.instances.len() > MAX_INSTANCE_COUNT
        {
            return Arc::from([]);
        }

        fn append(batch: &ParticleBatch, nodes: &mut Vec<SpatialNode>, first: usize, end: usize) {
            let mut bounds = Rect::new(
                f64::INFINITY,
                f64::INFINITY,
                f64::NEG_INFINITY,
                f64::NEG_INFINITY,
            );
            for instance in &batch.instances[first..end] {
                let particle = instance.particle;
                let radius = f64::from(batch.radius * particle.size);
                let extent = if instance.line {
                    let reach = f64::from(batch.width) * 0.5;
                    Rect::new(
                        f64::from(instance.line_bounds[0]) - reach,
                        f64::from(instance.line_bounds[1]) - reach,
                        f64::from(instance.line_bounds[2]) + reach,
                        f64::from(instance.line_bounds[3]) + reach,
                    )
                } else {
                    let reach = radius * if batch.kind == 1 { 3.0 } else { 1.0 };
                    Rect::new(
                        f64::from(particle.position[0]) - reach,
                        f64::from(particle.position[1]) - reach,
                        f64::from(particle.position[0]) + reach,
                        f64::from(particle.position[1]) + reach,
                    )
                };
                bounds.left = bounds.left.min(extent.left);
                bounds.top = bounds.top.min(extent.top);
                bounds.right = bounds.right.max(extent.right);
                bounds.bottom = bounds.bottom.max(extent.bottom);
            }
            let guard = [bounds.left, bounds.top, bounds.right, bounds.bottom]
                .into_iter()
                .fold(1.0_f64, |scale, coordinate| scale.max(coordinate.abs()))
                * (256.0 * f64::EPSILON);
            let current = nodes.len();
            let leaf = end - first <= 8;
            nodes.push(SpatialNode {
                bounds,
                guard,
                end,
                next: 0,
                leaf,
            });
            if !leaf {
                let middle = first + (end - first) / 2;
                append(batch, nodes, first, middle);
                append(batch, nodes, middle, end);
            }
            nodes[current].next = nodes.len();
        }
        let mut nodes = Vec::with_capacity(self.instances.len().div_ceil(8) * 4);
        append(self, &mut nodes, 0, self.instances.len());
        nodes.into()
    }

    pub(crate) fn validate(&self) -> Result<(), ValidationError> {
        if self.instances.len() > MAX_INSTANCE_COUNT
            || !self.radius.is_finite()
            || self.radius < 0.0
            || self.radius > 4096.0
            || !self.width.is_finite()
            || self.width < 0.0
            || self.width > 4096.0
        {
            return Err(invalid("style", "Invalid particle style size/count"));
        }
        for instance in self.instances.iter() {
            instance.particle.validate()?;
        }
        motion_core::drawing::validate_rect(self.bounds, "particles", "bounds")
    }

    pub(crate) fn sample(
        &self,
        point: Point,
        antialias: f64,
        destination: [f32; 4],
        space: motion_core::drawing::BlendSpace,
        composite: motion_core::instances::InstanceComposite,
    ) -> Result<[f32; 4], ValidationError> {
        self.sample_with_zero_policy::<false>(point, antialias, destination, space, composite)
    }

    pub(super) fn sample_with_zero_policy<const ZERO_IS_IDENTITY: bool>(
        &self,
        point: Point,
        antialias: f64,
        destination: [f32; 4],
        space: motion_core::drawing::BlendSpace,
        composite: motion_core::instances::InstanceComposite,
    ) -> Result<[f32; 4], ValidationError> {
        self.sample_selected::<ZERO_IS_IDENTITY>(
            point,
            antialias,
            destination,
            space,
            composite,
            None,
        )
    }

    fn sample_selected<const ZERO_IS_IDENTITY: bool>(
        &self,
        point: Point,
        antialias: f64,
        mut destination: [f32; 4],
        space: motion_core::drawing::BlendSpace,
        composite: motion_core::instances::InstanceComposite,
        selection: Option<&[usize]>,
    ) -> Result<[f32; 4], ValidationError> {
        let bounded = self.kind != 1
            && self.kind != 2
            && antialias.is_finite()
            && antialias > 0.0
            && point.x.is_finite()
            && point.y.is_finite();
        let point_guard = if bounded {
            point.x.abs().max(point.y.abs()).max(1.0) * (128.0 * f64::EPSILON)
        } else {
            0.0
        };
        let indexed = selection.is_none()
            && ZERO_IS_IDENTITY
            && !self.index.is_empty()
            && antialias.is_finite()
            && antialias > 0.0
            && point.x.is_finite()
            && point.y.is_finite();
        let index_guard = if indexed {
            point.x.abs().max(point.y.abs()).max(1.0) * (256.0 * f64::EPSILON)
        } else {
            0.0
        };
        let mut cursor = 0;
        let mut position = 0;
        let count = selection.map_or(self.instances.len(), <[usize]>::len);
        let mut leaf_end = if indexed { 0 } else { count };
        while position < count {
            if indexed && position == leaf_end {
                loop {
                    let node = &self.index[cursor];
                    if node.outside(point, antialias * 0.5, index_guard) {
                        position = node.end;
                        cursor = node.next;
                        if position == self.instances.len() {
                            break;
                        }
                    } else {
                        cursor += 1;
                        if node.leaf {
                            leaf_end = node.end;
                            break;
                        }
                    }
                }
                if position == self.instances.len() {
                    break;
                }
            }
            let instance = &self.instances[selection.map_or(position, |indices| indices[position])];
            position += 1;
            let particle = &instance.particle;
            let radius = f64::from(self.radius * particle.size);
            if radius == 0.0 || particle.color[3] == 0.0 {
                continue;
            }
            let relative = [
                point.x - f64::from(particle.position[0]),
                point.y - f64::from(particle.position[1]),
            ];
            let mut color = particle.color;
            let coverage = if instance.line {
                let guard = instance.line_guard + point_guard;
                let padding = f64::from(self.width) * 0.5 + antialias * 0.5 + guard;
                if bounded
                    && (point.x < f64::from(instance.line_bounds[0]) - padding
                        || point.x > f64::from(instance.line_bounds[2]) + padding
                        || point.y < f64::from(instance.line_bounds[1]) - padding
                        || point.y > f64::from(instance.line_bounds[3]) + padding)
                {
                    0.0
                } else {
                    let start = instance.previous.map(f64::from);
                    let end = particle.position.map(f64::from);
                    let delta = [end[0] - start[0], end[1] - start[1]];
                    let square = delta[0] * delta[0] + delta[1] * delta[1];
                    let progress = if square == 0.0 {
                        0.0
                    } else {
                        ((point.x - start[0]) * delta[0] + (point.y - start[1]) * delta[1]) / square
                    }
                    .clamp(0.0, 1.0);
                    let distance = libm::hypot(
                        point.x - start[0] - delta[0] * progress,
                        point.y - start[1] - delta[1] * progress,
                    );
                    ((f64::from(self.width) * 0.5 - distance) / antialias + 0.5).clamp(0.0, 1.0)
                }
            } else if self.kind == 1 {
                let square =
                    (relative[0] * relative[0] + relative[1] * relative[1]) / (radius * radius);
                if square > 9.0 {
                    0.0
                } else {
                    libm::exp(-2.0 * square)
                }
            } else if self.kind == 2 {
                let angle = f64::from(particle.rotation)
                    + if self.align && particle.velocity != [0.0; 2] {
                        libm::atan2(
                            f64::from(particle.velocity[1]),
                            f64::from(particle.velocity[0]),
                        )
                    } else {
                        0.0
                    };
                let cosine = libm::cos(angle);
                let sine = libm::sin(angle);
                let local = [
                    relative[0] * cosine + relative[1] * sine,
                    -relative[0] * sine + relative[1] * cosine,
                ];
                if local[0].abs() >= radius || local[1].abs() >= radius {
                    0.0
                } else {
                    let sprite = self.sprite.as_ref().expect("sprite material");
                    let sample = sprite_sample(
                        sprite,
                        local[0] / (2.0 * radius) + 0.5,
                        local[1] / (2.0 * radius) + 0.5,
                    );
                    color = std::array::from_fn(|channel| color[channel] * sample[channel]);
                    1.0
                }
            } else {
                let reach = radius + antialias * 0.5;
                let guard = reach.abs().max(1.0) * (128.0 * f64::EPSILON) + point_guard;
                if bounded && relative[0].abs().max(relative[1].abs()) > reach + guard {
                    0.0
                } else {
                    ((radius - libm::hypot(relative[0], relative[1])) / antialias + 0.5)
                        .clamp(0.0, 1.0)
                }
            } as f32;
            let source = color.map(|channel| channel * coverage);
            if ZERO_IS_IDENTITY && source == [0.0; 4] {
                continue;
            }
            composite(&mut destination, source, space)?;
        }
        Ok(destination)
    }

    #[cfg(feature = "gpu")]
    pub(crate) fn upload_bytes(&self) -> u64 {
        (4 + self.instances.len() as u64 * 12
            + self.sprite.as_ref().map_or(0, |sprite| {
                u64::from(sprite.width()) * u64::from(sprite.height()) * 4
            }))
            * 4
    }

    /// Glow dots add light; every other kind composites source-over.
    pub(super) fn blend(&self) -> motion_core::instances::InstanceBlend {
        if self.kind == 1 {
            motion_core::instances::InstanceBlend::Plus
        } else {
            motion_core::instances::InstanceBlend::SourceOver
        }
    }

    #[cfg(feature = "gpu")]
    pub(crate) fn upload(&self) -> Result<motion_core::instances::InstanceUpload, ValidationError> {
        use motion_core::instances::{InstanceKind, InstanceRecord, InstanceUpload};

        let sprite_size = self
            .sprite
            .as_ref()
            .map_or([0, 0], |sprite| [sprite.width(), sprite.height()]);
        let records = self.instances.iter().map(|instance| {
            let particle = instance.particle;
            let rotation = f64::from(particle.rotation)
                + if self.align && particle.velocity != [0.0; 2] {
                    libm::atan2(
                        f64::from(particle.velocity[1]),
                        f64::from(particle.velocity[0]),
                    )
                } else {
                    0.0
                };
            InstanceRecord {
                position: particle.position,
                start: instance.previous,
                color: particle.color,
                radius: self.radius * particle.size,
                rotation: rotation as f32,
                width: self.width,
                kind: if instance.line {
                    InstanceKind::Segment
                } else {
                    match self.kind {
                        1 => InstanceKind::Glow,
                        2 => InstanceKind::Sprite,
                        _ => InstanceKind::Disc,
                    }
                },
            }
        });
        let texels = self.sprite.iter().flat_map(|sprite| {
            (0..sprite.height()).flat_map(move |row| {
                (0..sprite.width()).map(move |column| {
                    sprite.levels()[0]
                        .pixel(column, row)
                        .expect("source texture bounds")
                })
            })
        });
        InstanceUpload::new_blended(records, sprite_size, texels, self.blend())
    }
}

impl motion_core::instances::InstanceBatch for ParticleBatch {
    fn blend(&self) -> motion_core::instances::InstanceBlend {
        ParticleBatch::blend(self)
    }

    fn len(&self) -> usize {
        self.len()
    }

    fn bounds(&self) -> Rect {
        self.bounds()
    }

    fn bytes(&self) -> u64 {
        self.bytes()
    }

    fn work(&self) -> u64 {
        self.work()
    }

    fn spatial(&self) -> Option<&dyn motion_core::instances::SpatialInstances> {
        Some(self)
    }

    fn validate(&self) -> Result<(), ValidationError> {
        self.validate()
    }

    fn sample(
        &self,
        point: Point,
        antialias: f64,
        destination: [f32; 4],
        space: motion_core::BlendSpace,
        composite: motion_core::instances::InstanceComposite,
    ) -> Result<[f32; 4], ValidationError> {
        self.sample(point, antialias, destination, space, composite)
    }

    fn sample_with_zero_identity(
        &self,
        point: Point,
        antialias: f64,
        destination: [f32; 4],
        space: motion_core::BlendSpace,
        composite: motion_core::instances::InstanceComposite,
    ) -> Result<[f32; 4], ValidationError> {
        self.sample_with_zero_policy::<true>(point, antialias, destination, space, composite)
    }

    #[cfg(feature = "gpu")]
    fn upload_bytes(&self) -> u64 {
        self.upload_bytes()
    }

    #[cfg(feature = "gpu")]
    fn upload(&self) -> Result<Option<motion_core::instances::InstanceUpload>, ValidationError> {
        self.upload().map(Some)
    }

    fn as_any(&self) -> &dyn std::any::Any {
        self
    }

    fn content_eq(&self, other: &dyn motion_core::instances::InstanceBatch) -> bool {
        other
            .as_any()
            .downcast_ref::<Self>()
            .is_some_and(|other| self == other)
    }
}

impl motion_core::instances::SpatialInstances for ParticleBatch {
    fn support(&self, index: usize, antialias: f64) -> Rect {
        let instance = &self.instances[index];
        let particle = instance.particle;
        let radius = f64::from(self.radius * particle.size);
        if radius == 0.0 || particle.color[3] == 0.0 {
            return Rect::new(0.0, 0.0, 0.0, 0.0);
        }
        let padding = antialias * 0.5;
        let (left, top, right, bottom) = if instance.line {
            let reach = f64::from(self.width) * 0.5 + padding + instance.line_guard;
            (
                f64::from(instance.line_bounds[0]) - reach,
                f64::from(instance.line_bounds[1]) - reach,
                f64::from(instance.line_bounds[2]) + reach,
                f64::from(instance.line_bounds[3]) + reach,
            )
        } else {
            let reach = radius
                * match self.kind {
                    1 => 3.0,
                    2 => 2.0,
                    _ => 1.0,
                }
                + padding;
            (
                f64::from(particle.position[0]) - reach,
                f64::from(particle.position[1]) - reach,
                f64::from(particle.position[0]) + reach,
                f64::from(particle.position[1]) + reach,
            )
        };
        let guard = [left, top, right, bottom]
            .into_iter()
            .fold(1.0_f64, |scale, value| scale.max(value.abs()))
            * (4096.0 * f64::EPSILON);
        Rect::new(left - guard, top - guard, right + guard, bottom + guard)
    }

    fn sample_indices(
        &self,
        indices: &[usize],
        point: Point,
        antialias: f64,
        destination: [f32; 4],
        space: motion_core::BlendSpace,
        composite: motion_core::instances::InstanceComposite,
    ) -> Result<[f32; 4], ValidationError> {
        self.sample_selected::<true>(
            point,
            antialias,
            destination,
            space,
            composite,
            Some(indices),
        )
    }
}

fn sprite_sample(sprite: &crate::ImageAsset, horizontal: f64, vertical: f64) -> [f32; 4] {
    let width = sprite.width() as usize;
    let height = sprite.height() as usize;
    let horizontal = horizontal * width as f64 - 0.5;
    let vertical = vertical * height as f64 - 0.5;
    let left = libm::floor(horizontal) as i64;
    let top = libm::floor(vertical) as i64;
    let fraction = [
        (horizontal - left as f64) as f32,
        (vertical - top as f64) as f32,
    ];
    let texel = |column: i64, row: i64| {
        sprite.levels()[0]
            .pixel(
                column.clamp(0, width as i64 - 1) as u32,
                row.clamp(0, height as i64 - 1) as u32,
            )
            .expect("source texture bounds")
    };
    let top_left = texel(left, top);
    let top_right = texel(left + 1, top);
    let bottom_left = texel(left, top + 1);
    let bottom_right = texel(left + 1, top + 1);
    std::array::from_fn(|channel| {
        (top_left[channel] * (1.0 - fraction[0]) + top_right[channel] * fraction[0])
            * (1.0 - fraction[1])
            + (bottom_left[channel] * (1.0 - fraction[0]) + bottom_right[channel] * fraction[0])
                * fraction[1]
    })
}

impl Drawable for ParticleBatch {
    fn bounds(&self, context: &motion_core::DrawContext) -> Rect {
        let margin = 1.0 / context.device_scale();
        Rect::new(
            self.bounds.left - margin,
            self.bounds.top - margin,
            self.bounds.right + margin,
            self.bounds.bottom + margin,
        )
    }

    fn draw(
        &self,
        _: &motion_core::DrawContext,
        canvas: &mut motion_core::Canvas<'_>,
    ) -> Result<(), ValidationError> {
        canvas.instances(Arc::new(self.clone()), Transform::IDENTITY)
    }
}
