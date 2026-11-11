//! Paint sources: gradient tables, pattern sampling and row-by-row paint evaluation.

use super::*;
use motion_core::drawing::paint::{Geometry, Gradient, GradientExtend};
use motion_core::{Paint, Point, Rect, Transform, ValidationError};

#[cfg(test)]
mod tests;

pub(crate) const TABLE_STEPS: usize = 256;

pub(super) struct TableCache {
    entries: std::sync::Arc<motion_core::SceneCache>,
}

impl Default for TableCache {
    fn default() -> Self {
        Self::new(&motion_core::RenderCache::default())
    }
}

impl TableCache {
    pub(super) fn new(cache: &motion_core::RenderCache) -> Self {
        Self {
            entries: cache
                .partition(motion_core::CachePartition::Gradient)
                .clone(),
        }
    }

    pub(super) fn retained_bytes(&self) -> u64 {
        self.entries.retained_bytes() as u64
    }

    fn get_or_build(
        &mut self,
        gradient: &Gradient,
        build: impl FnOnce() -> Result<std::sync::Arc<[[f32; 4]]>, ValidationError>,
    ) -> Result<std::sync::Arc<[[f32; 4]]>, ValidationError> {
        let key = gradient
            .stops
            .iter()
            .map(|(position, color)| {
                let [red, green, blue, alpha] = color.to_rgba_float().map(f64::to_bits);
                [position.to_bits(), red, green, blue, alpha]
            })
            .collect::<Vec<_>>();
        let key = (key, gradient.space as u8);
        if let Some(table) = self.entries.lookup::<_, std::sync::Arc<[[f32; 4]]>>(&key) {
            return Ok(table.as_ref().clone());
        }
        let table = build()?;
        let bytes = key.0.len() * std::mem::size_of::<[u64; 5]>()
            + table.len() * 16
            + std::mem::size_of_val(&key)
            + std::mem::size_of_val(&table);
        Ok(self
            .entries
            .insert_with_cost(key, table, |_| bytes)
            .as_ref()
            .clone())
    }
}

#[derive(Clone, Debug, PartialEq)]
enum GradientGeometry {
    Linear(Point, motion_core::Vector2, f64),
    Radial(Point, f64),
    Focal(Point, motion_core::Vector2, f64),
    Sweep(Point, f64, f64),
}

impl GradientGeometry {
    fn new(geometry: &Geometry) -> Self {
        match *geometry {
            Geometry::Linear(start, end) => {
                let direction = end - start;
                let length = direction.length();
                Self::Linear(start, direction / length, length)
            }
            Geometry::Radial(center, radius) => Self::Radial(center, radius),
            Geometry::RadialFocal(center, radius, focus) => {
                let displacement = focus - center;
                Self::Focal(
                    focus,
                    displacement,
                    radius * radius - displacement.dot(displacement),
                )
            }
            Geometry::Sweep(center, start, end) => Self::Sweep(center, start, end - start),
        }
    }

    fn position(&self, point: Point) -> f64 {
        match *self {
            Self::Linear(start, direction, length) => (point - start).dot(direction) / length,
            Self::Radial(center, radius) => point.distance(center) / radius,
            Self::Focal(focus, displacement, radius_square) => {
                let offset = point - focus;
                let square = offset.dot(offset);
                if square == 0.0 {
                    0.0
                } else {
                    let projection = offset.dot(displacement);
                    square
                        / (-projection
                            + libm::sqrt(projection * projection + square * radius_square))
                }
            }
            Self::Sweep(center, start, range) => {
                (libm::atan2(point.y - center.y, point.x - center.x) - start)
                    .rem_euclid(std::f64::consts::TAU)
                    / range
            }
        }
    }
}

#[derive(Clone, Debug, PartialEq)]
pub(crate) struct Source {
    pub(crate) paint: Paint,
    pub(crate) inverse: Transform,
    pattern_inverse: Transform,
    geometry: Option<GradientGeometry>,
    invertible: bool,
    pub(crate) bounds: Rect,
    pub(crate) gradient_table: std::sync::Arc<[[f32; 4]]>,
    pub(crate) mip: (usize, usize, f32),
}

impl Source {
    pub(crate) fn work_per_pixel(&self) -> u64 {
        self.paint
            .gradient()
            .map_or(24, |gradient| gradient.stops.len() as u64 + 24)
    }

    fn new(
        paint: &Paint,
        bounds: Rect,
        transform: Transform,
        cache: Option<&mut TableCache>,
    ) -> Result<Self, ValidationError> {
        paint.validate()?;
        let inverted = transform.inverse();
        let inverse = inverted.unwrap_or(Transform::IDENTITY);
        let mut invertible = inverted.is_some();
        let mut table = std::sync::Arc::from([]);
        if let Some(gradient) = paint.gradient() {
            if gradient.units == motion_core::GradientUnits::ObjectBoundingBox
                && (bounds.width() == 0.0 || bounds.height() == 0.0)
            {
                invertible = false;
            } else {
                gradient.position(bounds.center(), bounds)?;
            }
            for stops in gradient.stops.windows(2) {
                if stops[0].0 != stops[1].0 && stops[0].0 as f32 == stops[1].0 as f32 {
                    return Err(ValidationError::invalid(
                        "paint",
                        "stops",
                        "Gradient stops must remain distinguishable in shared f32 storage",
                    ));
                }
            }
            let build = || {
                let mut table = Vec::new();
                for stops in gradient.stops.windows(2) {
                    let pair = Gradient {
                        stops: std::sync::Arc::from([(0.0, stops[0].1), (1.0, stops[1].1)]),
                        ..gradient.clone()
                    };
                    for sample in 0..=TABLE_STEPS {
                        let color = pair
                            .color(sample as f64 / TABLE_STEPS as f64)
                            .map(|value| value as f32);
                        if color.iter().any(|value| !value.is_finite()) {
                            return Err(ValidationError::invalid(
                                "paint",
                                "color",
                                "Gradient table must remain finite in working storage",
                            ));
                        }
                        table.push(color);
                    }
                }
                Ok(std::sync::Arc::from(table))
            };
            table = if let Some(cache) = cache {
                cache.get_or_build(gradient, build)?
            } else {
                build()?
            };
        }
        let pattern_inverse = if let Paint::Pattern(pattern) = paint {
            pattern.transform.inverse().expect("validated pattern")
        } else {
            Transform::IDENTITY
        };
        let mip = if let Paint::Pattern(pattern) = paint {
            let mapping = inverse.then(pattern_inverse);
            let [xx, yx, xy, yy, _, _] = mapping.coefficients();
            let scale = libm::hypot(xx, yx).max(libm::hypot(xy, yy));
            if !scale.is_finite() {
                return Err(ValidationError::invalid(
                    "paint",
                    "lod",
                    "Image sampling scale must remain finite",
                ));
            }
            let lod = if pattern.sampling == motion_core::Sampling::Trilinear {
                libm::log2(scale.max(1.0)).min((pattern.image.levels() - 1) as f64)
            } else {
                0.0
            };
            let first = libm::floor(lod) as usize;
            let second = (first + 1).min(pattern.image.levels() - 1);
            (first, second, (lod - first as f64) as f32)
        } else {
            (0, 0, 0.0)
        };
        Ok(Self {
            paint: paint.clone(),
            inverse,
            pattern_inverse,
            geometry: paint
                .gradient()
                .map(|gradient| GradientGeometry::new(&gradient.geometry)),
            invertible,
            bounds,
            gradient_table: table,
            mip,
        })
    }

    pub(crate) fn sample(&self, point: Point) -> Result<[f32; 4], ValidationError> {
        if !self.invertible {
            return Err(ValidationError::invalid(
                "paint",
                "transform",
                "Collapsed spatial paint has no inverse sample mapping",
            ));
        }
        let point = self.inverse.apply(point);
        if let Some(gradient) = self.paint.gradient() {
            let position = gradient.position(point, self.bounds)? as f32;
            return Ok(self.gradient_color(gradient, position));
        }
        self.sample_pattern(point)
    }

    fn gradient_color(&self, gradient: &Gradient, position: f32) -> [f32; 4] {
        let upper = gradient
            .stops
            .partition_point(|(offset, _)| *offset as f32 <= position);
        if upper == gradient.stops.len() {
            return *self
                .gradient_table
                .last()
                .expect("gradient table has entries");
        }
        let lower = upper.saturating_sub(1);
        let start = gradient.stops[lower].0 as f32;
        let end = gradient.stops[upper].0 as f32;
        let position = ((position - start) / (end - start)).clamp(0.0, 1.0) * TABLE_STEPS as f32;
        let index = (position as usize).min(TABLE_STEPS - 1);
        let amount = position - index as f32;
        let first = self.gradient_table[lower * (TABLE_STEPS + 1) + index];
        let second = self.gradient_table[lower * (TABLE_STEPS + 1) + index + 1];
        std::array::from_fn(|channel| first[channel] * (1.0 - amount) + second[channel] * amount)
    }

    fn sample_pattern(&self, point: Point) -> Result<[f32; 4], ValidationError> {
        let Paint::Pattern(pattern) = &self.paint else {
            unreachable!("spatial paint")
        };
        let point = self.pattern_inverse.apply(point);
        let first = sample_image(pattern, self.mip.0, point)?;
        if self.mip.2 == 0.0 {
            return Ok(first);
        }
        let second = sample_image(pattern, self.mip.1, point)?;
        Ok(std::array::from_fn(|channel| {
            first[channel] * (1.0 - self.mip.2) + second[channel] * self.mip.2
        }))
    }
}

pub(crate) struct PaintRow<'frame> {
    source: Option<&'frame Source>,
    solid: [f32; 4],
    coefficients: [f64; 6],
    vertical: [f64; 2],
    horizontal: [f64; 2],
    column: u32,
    row: u32,
    ready: bool,
    incremental: bool,
}

impl PaintRow<'_> {
    pub(crate) fn sample(&mut self, column: u32) -> Result<[f32; 4], ValidationError> {
        let Some(source) = self.source else {
            return Ok(self.solid);
        };
        if !self.ready {
            return source.sample(Point::new(
                f64::from(column) + 0.5,
                f64::from(self.row) + 0.5,
            ));
        }
        let [xx, yx, _, _, tx, ty] = self.coefficients;
        if self.incremental && column <= 1_048_576 {
            let delta = f64::from(column) - f64::from(self.column);
            self.horizontal[0] += xx * delta;
            self.horizontal[1] += yx * delta;
        } else {
            self.horizontal = [
                xx * (f64::from(column) + 0.5),
                yx * (f64::from(column) + 0.5),
            ];
        }
        self.column = column;
        let mut point = Point::new(
            self.horizontal[0] + self.vertical[0] + tx,
            self.horizontal[1] + self.vertical[1] + ty,
        );
        let Some(gradient) = source.paint.gradient() else {
            return source.sample_pattern(point);
        };
        if gradient.units == motion_core::GradientUnits::ObjectBoundingBox {
            point = Point::new(
                (point.x - source.bounds.left) / source.bounds.width(),
                (point.y - source.bounds.top) / source.bounds.height(),
            );
        }
        let position = source
            .geometry
            .as_ref()
            .expect("prepared gradient")
            .position(point);
        if !position.is_finite() {
            return Err(ValidationError::invalid(
                "paint.gradient",
                "parameters",
                "Gradient sample position must remain finite",
            ));
        }
        Ok(source.gradient_color(gradient, gradient.extend.apply(position) as f32))
    }
}

fn sample_image(
    pattern: &motion_core::ImagePattern,
    level: usize,
    point: Point,
) -> Result<[f32; 4], ValidationError> {
    if pattern.decal
        && (point.x < 0.0
            || point.y < 0.0
            || point.x >= f64::from(pattern.image.width())
            || point.y >= f64::from(pattern.image.height()))
    {
        return Ok([0.0; 4]);
    }
    let point = Point::new(
        point.x / f64::from(pattern.image.width()),
        point.y / f64::from(pattern.image.height()),
    );
    let (width, height) = pattern.image.size(level);
    let width = width as usize;
    let height = height as usize;
    if !point.x.is_finite() || !point.y.is_finite() {
        return Err(ValidationError::invalid(
            "paint",
            "coordinates",
            "Image sample must remain finite",
        ));
    }
    let horizontal = pattern.extend.apply(point.x) * width as f64 - 0.5;
    let vertical = pattern.extend.apply(point.y) * height as f64 - 0.5;
    let column = libm::floor(horizontal) as i64;
    let row = libm::floor(vertical) as i64;
    let horizontal = (horizontal - column as f64) as f32;
    let vertical = (vertical - row as f64) as f32;
    let index = |value: i64, size: usize| match pattern.extend {
        GradientExtend::Pad => value.clamp(0, size as i64 - 1) as usize,
        GradientExtend::Repeat => value.rem_euclid(size as i64) as usize,
        GradientExtend::Reflect => {
            let value = value.rem_euclid(size as i64 * 2);
            value.min(size as i64 * 2 - 1 - value) as usize
        }
    };
    if pattern.sampling == motion_core::Sampling::Nearest {
        return Ok(pattern.image.texel(
            level,
            index((horizontal + column as f32 + 0.5) as i64, width),
            index((vertical + row as f32 + 0.5) as i64, height),
        ));
    }
    let first = pattern
        .image
        .texel(level, index(column, width), index(row, height));
    let second = pattern
        .image
        .texel(level, index(column + 1, width), index(row, height));
    let third = pattern
        .image
        .texel(level, index(column, width), index(row + 1, height));
    let fourth = pattern
        .image
        .texel(level, index(column + 1, width), index(row + 1, height));
    Ok(std::array::from_fn(|channel| {
        (first[channel] * (1.0 - horizontal) + second[channel] * horizontal) * (1.0 - vertical)
            + (third[channel] * (1.0 - horizontal) + fourth[channel] * horizontal) * vertical
    }))
}

impl PreparedFrame {
    pub(crate) fn paint_row(&self, index: usize, column: u32, row: u32) -> PaintRow<'_> {
        let source = self.paint_sources[index].as_ref();
        let ready = source.is_none_or(|source| source.invertible);
        let coefficients = source
            .map_or(Transform::IDENTITY, |source| source.inverse)
            .coefficients();
        let [xx, yx, xy, yy, _, _] = coefficients;
        PaintRow {
            source,
            solid: self.paints[index].linear_premultiplied,
            coefficients,
            vertical: [xy * (f64::from(row) + 0.5), yy * (f64::from(row) + 0.5)],
            horizontal: [
                xx * (f64::from(column) + 0.5),
                yx * (f64::from(column) + 0.5),
            ],
            column,
            row,
            ready,
            incremental: column <= 1_048_576
                && [xx, yx].iter().all(|coefficient| {
                    let units = coefficient * 65_536.0;
                    units.abs() <= 1_073_741_824.0 && units.fract() == 0.0
                }),
        }
    }

    pub(super) fn add_paint(
        &mut self,
        paint: &Paint,
        bounds: Rect,
        transform: Transform,
        budget: &mut Budget,
        path: &str,
        cache: Option<&mut TableCache>,
    ) -> Result<usize, ValidationError> {
        let baked;
        let paint = if let Paint::Mesh(mesh) = paint {
            baked = Paint::Pattern(mesh.pattern(bounds, budget.limits)?);
            &baked
        } else {
            paint
        };
        let (record, source) = match paint {
            Paint::Solid(color) => (paint_record(*color, path)?, None),
            _ => {
                let entries = paint.gradient().map_or(0, |gradient| {
                    gradient.stops.len().saturating_sub(1) * (TABLE_STEPS + 1)
                });
                budget.bytes(entries as u64 * 16, path)?;
                budget.work(entries as u64, path)?;
                (
                    PaintRecord {
                        linear_premultiplied: [0.0; 4],
                    },
                    Some(Source::new(paint, bounds, transform, cache)?),
                )
            }
        };
        let index = self.paints.len();
        budget.push(&mut self.paints, record, path)?;
        budget.push(&mut self.paint_sources, source, path)?;
        Ok(index)
    }

    /// Samples a validated paint at a device-space point. Spatial paints
    /// share immutable linear-premultiplied tables/pixels with the GPU backend.
    pub fn paint_at(&self, index: usize, point: Point) -> Result<[f32; 4], ValidationError> {
        if !point.x.is_finite() || !point.y.is_finite() {
            return Err(ValidationError::invalid(
                "paint",
                "coordinates",
                "Paint sample must be finite",
            ));
        }
        match self.paint_sources.get(index) {
            Some(Some(source)) => source.sample(point),
            Some(None) => Ok(self.paints[index].linear_premultiplied),
            None => Err(ValidationError::invalid(
                "paint",
                "index",
                "Paint index is out of bounds",
            )),
        }
    }
}
