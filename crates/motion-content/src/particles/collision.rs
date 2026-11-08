//! Swept geometric contacts and immutable, budgeted signed-distance grids.
//!
//! # Particle radius
//!
//! Every collider treats `Particle::size` as the particle's collision radius in
//! the same local design units as the collider geometry; the particle is a disc,
//! not a point. A contact occurs when the signed distance from the particle
//! centre to the collider surface is at most `size` (plus a `1e-6` tolerance):
//!
//! * circles and segments measure the distance to the circle/segment boundary;
//! * polygons use the distance to the nearest edge, negative inside, so a disc
//!   straddling an edge or fully inside a solid polygon is pushed out along the
//!   edge normal;
//! * SDF grids sample the stored signed distance (negative inside) and use its
//!   gradient as the contact normal.
//!
//! `size` is the birth size multiplied by any `SizeOverLifetime`, so a particle
//! that grows collides earlier. Set `size` to zero for point-like behaviour.
//! Because `size` is also the render scale, styles whose base radius differs from
//! one design unit should scale collider geometry accordingly.

use super::*;
use motion_core::{Point, Rect};

/// Largest coordinate magnitude for which [`swept_from`] may skip a particle using collider
/// bounds; probe rounding there stays far below [`BOUNDS_MARGIN`].
const BOUNDS_EXTENT: f64 = 1e7;

/// Absolute slack, in design units, added to the bounds test for probe rounding.
const BOUNDS_MARGIN: f64 = 1e-3;

/// Bounded normal restitution and tangential friction.
#[derive(Clone, Copy, Debug)]
pub struct CollisionResponse {
    /// Normal velocity restitution in 0..=1.
    pub restitution: f32,
    /// Fraction of tangential velocity removed in 0..=1.
    pub friction: f32,
    /// Remove the particle at its first contact.
    pub kill_on_hit: bool,
}

impl Default for CollisionResponse {
    fn default() -> Self {
        Self {
            restitution: 1.0,
            friction: 0.0,
            kill_on_hit: false,
        }
    }
}

impl CollisionResponse {
    pub(super) fn validate(self) -> Result<(), ValidationError> {
        if !self.restitution.is_finite()
            || !self.friction.is_finite()
            || !(0.0..=1.0).contains(&self.restitution)
            || !(0.0..=1.0).contains(&self.friction)
        {
            return Err(invalid(
                "collision",
                "Restitution/friction must be finite in 0..=1",
            ));
        }
        Ok(())
    }
}

fn nearest(point: Point, start: Point, end: Point) -> (f64, [f64; 2]) {
    let delta = [end.x - start.x, end.y - start.y];
    let square = delta[0] * delta[0] + delta[1] * delta[1];
    let progress = if square == 0.0 {
        0.0
    } else {
        ((point.x - start.x) * delta[0] + (point.y - start.y) * delta[1]) / square
    }
    .clamp(0.0, 1.0);
    let difference = [
        point.x - start.x - delta[0] * progress,
        point.y - start.y - delta[1] * progress,
    ];
    let distance = libm::hypot(difference[0], difference[1]);
    let normal = if distance > 1e-12 {
        difference.map(|value| value / distance)
    } else {
        let length = libm::hypot(delta[0], delta[1]).max(1e-12);
        [-delta[1] / length, delta[0] / length]
    };
    (distance, normal)
}

fn inside_polygon(point: Point, vertices: &[Point]) -> bool {
    let mut inside = false;
    for index in 0..vertices.len() {
        let first = vertices[index];
        let second = vertices[(index + 1) % vertices.len()];
        if (first.y > point.y) != (second.y > point.y)
            && point.x < (second.x - first.x) * (point.y - first.y) / (second.y - first.y) + first.x
        {
            inside = !inside;
        }
    }
    inside
}

fn swept(
    particle: &mut Particle,
    step: f32,
    response: CollisionResponse,
    bounds: Option<Rect>,
    distance: impl Fn(Point) -> (f64, [f64; 2]),
) -> Result<bool, ValidationError> {
    swept_from(particle, step, response, bounds, distance, 0.0)
}

/// `bounds` encloses all collider geometry, or is `None` to always run the probe loop.
fn swept_from(
    particle: &mut Particle,
    step: f32,
    response: CollisionResponse,
    bounds: Option<Rect>,
    distance: impl Fn(Point) -> (f64, [f64; 2]),
    mut progress: f64,
) -> Result<bool, ValidationError> {
    particle.validate()?;
    response.validate()?;
    if !step.is_finite() || step <= 0.0 {
        return Err(invalid(
            "collision",
            "Contact step must be positive and finite",
        ));
    }
    let start = particle.position.map(f64::from);
    let velocity = particle.velocity.map(f64::from);
    let travel = libm::hypot(velocity[0], velocity[1]) * f64::from(step);
    // `size` is the collision radius for all collider shapes.
    let radius = f64::from(particle.size);
    if let Some(bounds) = bounds {
        // A path farther than `radius + travel / 32` from the bounds never contacts, and each
        // probe below then advances by more than 1/32 of the travel, so the loop would end
        // without contact well inside its iteration allowance. Skipping it is exact.
        let end = [
            start[0] + velocity[0] * f64::from(step),
            start[1] + velocity[1] * f64::from(step),
        ];
        let margin = radius + travel / 32.0 + BOUNDS_MARGIN;
        if start
            .iter()
            .chain(&end)
            .all(|value| value.abs() <= BOUNDS_EXTENT)
            && (start[0].min(end[0]) - margin > bounds.right
                || start[0].max(end[0]) + margin < bounds.left
                || start[1].min(end[1]) - margin > bounds.bottom
                || start[1].max(end[1]) + margin < bounds.top)
        {
            return Ok(false);
        }
    }
    for _ in 0..64 {
        let point = Point::new(
            start[0] + velocity[0] * f64::from(step) * progress,
            start[1] + velocity[1] * f64::from(step) * progress,
        );
        let (signed, normal) = distance(point);
        if !signed.is_finite() || normal.iter().any(|value| !value.is_finite()) {
            return Err(invalid("collision", "Nonfinite contact geometry"));
        }
        if signed <= radius + 1e-6 {
            let contact = [
                point.x + normal[0] * (radius - signed + 1e-5),
                point.y + normal[1] * (radius - signed + 1e-5),
            ];
            let incoming = velocity[0] * normal[0] + velocity[1] * normal[1];
            let normal_speed = if incoming < 0.0 {
                -incoming * f64::from(response.restitution)
            } else {
                incoming
            };
            let tangent = [
                velocity[0] - incoming * normal[0],
                velocity[1] - incoming * normal[1],
            ];
            particle.velocity = std::array::from_fn(|axis| {
                (normal[axis] * normal_speed + tangent[axis] * (1.0 - f64::from(response.friction)))
                    as f32
            });
            particle.position = std::array::from_fn(|axis| {
                (contact[axis] - f64::from(particle.velocity[axis]) * f64::from(step) * progress)
                    as f32
            });
            if response.kill_on_hit {
                particle.lifetime = particle.age.max(f32::MIN_POSITIVE);
                particle.position = contact.map(|value| value as f32);
            }
            particle.validate()?;
            return Ok(true);
        }
        if progress >= 1.0 || travel == 0.0 {
            return Ok(false);
        }
        progress = (progress + ((signed - radius) / travel).max(1e-7)).min(1.0);
    }
    Err(limit("Swept collision iteration allowance exhausted"))
}

/// Solid circular obstacle with swept collision detection.
#[derive(Clone, Copy, Debug)]
pub struct CircleCollider {
    /// Obstacle center.
    pub center: Point,
    /// Nonnegative obstacle radius.
    pub radius: f64,
    /// Contact response.
    pub response: CollisionResponse,
}

impl CircleCollider {
    fn contact(&self, particle: &mut Particle, step: f32) -> Result<bool, ValidationError> {
        if !self.radius.is_finite()
            || self.radius < 0.0
            || !self.center.x.is_finite()
            || !self.center.y.is_finite()
        {
            return Err(invalid("circle", "Invalid collider circle"));
        }
        swept(particle, step, self.response, None, |point| {
            let delta = [point.x - self.center.x, point.y - self.center.y];
            let length = libm::hypot(delta[0], delta[1]);
            (
                length - self.radius,
                if length == 0.0 {
                    [1.0, 0.0]
                } else {
                    delta.map(|value| value / length)
                },
            )
        })
    }
}

/// Two-sided finite segment, including endpoint contacts.
#[derive(Clone, Copy, Debug)]
pub struct SegmentCollider {
    /// Segment start.
    pub start: Point,
    /// Segment end.
    pub end: Point,
    /// Contact response.
    pub response: CollisionResponse,
}

impl SegmentCollider {
    fn contact(&self, particle: &mut Particle, step: f32) -> Result<bool, ValidationError> {
        if [self.start.x, self.start.y, self.end.x, self.end.y]
            .iter()
            .any(|value| !value.is_finite())
            || self.start == self.end
        {
            return Err(invalid("segment", "Require finite distinct endpoints"));
        }
        particle.validate()?;
        self.response.validate()?;
        if !step.is_finite() || step <= 0.0 {
            return Err(invalid(
                "collision",
                "Contact step must be positive and finite",
            ));
        }
        let origin = Point::new(
            f64::from(particle.position[0]),
            f64::from(particle.position[1]),
        );
        let motion = particle
            .velocity
            .map(|value| f64::from(value) * f64::from(step));
        let radius = f64::from(particle.size);
        let mut hit = if nearest(origin, self.start, self.end).0 <= radius + 1e-6 {
            Some(0.0)
        } else {
            None
        };
        let delta = [self.end.x - self.start.x, self.end.y - self.start.y];
        let length = libm::hypot(delta[0], delta[1]);
        let axis = delta.map(|value| value / length);
        let normal = [-axis[1], axis[0]];
        let relative = [origin.x - self.start.x, origin.y - self.start.y];
        let side = relative[0] * normal[0] + relative[1] * normal[1];
        let crossing = motion[0] * normal[0] + motion[1] * normal[1];
        let mut admit = |progress: f64| {
            if (0.0..=1.0).contains(&progress) {
                hit = Some(hit.map_or(progress, |prior: f64| prior.min(progress)));
            }
        };
        if crossing != 0.0 {
            for edge in [-radius, radius] {
                let progress = (edge - side) / crossing;
                let along = (relative[0] + motion[0] * progress) * axis[0]
                    + (relative[1] + motion[1] * progress) * axis[1];
                if (0.0..=length).contains(&along) {
                    admit(progress);
                }
            }
        }
        let square = motion[0] * motion[0] + motion[1] * motion[1];
        if square > 0.0 {
            for end in [self.start, self.end] {
                let offset = [origin.x - end.x, origin.y - end.y];
                let projection = offset[0] * motion[0] + offset[1] * motion[1];
                let constant = offset[0] * offset[0] + offset[1] * offset[1] - radius * radius;
                let discriminant = projection * projection - square * constant;
                if discriminant >= 0.0 {
                    admit((-projection - libm::sqrt(discriminant)) / square);
                }
            }
        }
        let Some(progress) = hit else {
            return Ok(false);
        };
        swept_from(
            particle,
            step,
            self.response,
            None,
            |point| nearest(point, self.start, self.end),
            progress,
        )
    }
}

/// Bounded simple polygon obstacle, prepared once rather than per particle.
#[derive(Clone, Debug)]
pub struct PolygonCollider {
    vertices: Arc<[Point]>,
    response: CollisionResponse,
    /// Vertex bounds for the swept early-out, absent beyond [`BOUNDS_EXTENT`].
    bounds: Option<Rect>,
}

impl PolygonCollider {
    /// Validates 3..=256 finite vertices and rejects intersecting nonadjacent edges.
    pub fn new(
        vertices: Arc<[Point]>,
        response: CollisionResponse,
    ) -> Result<Self, ValidationError> {
        response.validate()?;
        if !(3..=256).contains(&vertices.len())
            || vertices
                .iter()
                .any(|point| !point.x.is_finite() || !point.y.is_finite())
        {
            return Err(invalid("polygon", "Require 3..=256 finite vertices"));
        }
        let cross = |first: Point, second: Point, third: Point| {
            (second.x - first.x) * (third.y - first.y) - (second.y - first.y) * (third.x - first.x)
        };
        let area = (0..vertices.len())
            .map(|index| {
                vertices[index].x * vertices[(index + 1) % vertices.len()].y
                    - vertices[(index + 1) % vertices.len()].x * vertices[index].y
            })
            .sum::<f64>();
        if !area.is_finite() || area.abs() < 1e-12 {
            return Err(invalid("polygon", "Degenerate polygon"));
        }
        for first in 0..vertices.len() {
            if vertices[first] == vertices[(first + 1) % vertices.len()] {
                return Err(invalid("polygon", "Duplicate adjacent vertices"));
            }
            for second in first + 2..vertices.len() {
                if first == 0 && second == vertices.len() - 1 {
                    continue;
                }
                let ends = [
                    vertices[first],
                    vertices[(first + 1) % vertices.len()],
                    vertices[second],
                    vertices[(second + 1) % vertices.len()],
                ];
                let overlap = ends[0].x.min(ends[1].x) <= ends[2].x.max(ends[3].x)
                    && ends[2].x.min(ends[3].x) <= ends[0].x.max(ends[1].x)
                    && ends[0].y.min(ends[1].y) <= ends[2].y.max(ends[3].y)
                    && ends[2].y.min(ends[3].y) <= ends[0].y.max(ends[1].y);
                if overlap
                    && cross(ends[0], ends[1], ends[2]) * cross(ends[0], ends[1], ends[3]) <= 0.0
                    && cross(ends[2], ends[3], ends[0]) * cross(ends[2], ends[3], ends[1]) <= 0.0
                {
                    return Err(invalid("polygon", "Polygon edges intersect"));
                }
            }
        }
        let bounds = vertices
            .iter()
            .all(|point| point.x.abs() <= BOUNDS_EXTENT && point.y.abs() <= BOUNDS_EXTENT)
            .then(|| {
                vertices.iter().skip(1).fold(
                    Rect::new(vertices[0].x, vertices[0].y, vertices[0].x, vertices[0].y),
                    |bounds, point| {
                        Rect::new(
                            bounds.left.min(point.x),
                            bounds.top.min(point.y),
                            bounds.right.max(point.x),
                            bounds.bottom.max(point.y),
                        )
                    },
                )
            });
        Ok(Self {
            vertices,
            response,
            bounds,
        })
    }

    fn contact(&self, particle: &mut Particle, step: f32) -> Result<bool, ValidationError> {
        swept(particle, step, self.response, self.bounds, |point| {
            let (distance, normal) = (0..self.vertices.len())
                .map(|index| {
                    nearest(
                        point,
                        self.vertices[index],
                        self.vertices[(index + 1) % self.vertices.len()],
                    )
                })
                .min_by(|first, second| first.0.total_cmp(&second.0))
                .unwrap_or((0.0, [1.0, 0.0]));
            if inside_polygon(point, &self.vertices) {
                (-distance, normal.map(|value| -value))
            } else {
                (distance, normal)
            }
        })
    }
}

macro_rules! collider {
    ($name:ident, $work:expr) => {
        impl ParticleModifier for $name {
            fn apply(
                &self,
                particle: &mut Particle,
                step: f32,
                _: Time,
                _: &mut Random,
            ) -> Result<(), ValidationError> {
                self.contact(particle, step).map(|_| ())
            }
            fn apply_event(
                &self,
                particle: &mut Particle,
                step: f32,
                _: Time,
                _: &mut Random,
            ) -> Result<bool, ValidationError> {
                self.contact(particle, step)
            }
            fn work(&self) -> u64 {
                $work(self)
            }
        }
    };
}
collider!(CircleCollider, |_: &CircleCollider| 64);
collider!(SegmentCollider, |_: &SegmentCollider| 64);
collider!(
    PolygonCollider,
    |collider: &PolygonCollider| collider.vertices.len() as u64 * 128
);

/// Precomputed signed samples plus exact cached boundary guards for thin features.
/// All data is immutable and can be shared through SceneContext::cache.
#[derive(Clone, Debug)]
pub struct SignedDistanceGrid {
    bounds: Rect,
    dimensions: [u32; 2],
    values: Arc<[f32]>,
    surface: Arc<crate::VectorSurface>,
    boundary: Arc<[(Point, Point)]>,
}

impl SignedDistanceGrid {
    /// Shares prepared immutable grids in the existing bounded SceneCache.
    /// The key includes geometry identity, resolution and preparation budgets.
    #[track_caller]
    pub fn cached<Key: Eq + std::hash::Hash + Send + Sync + 'static>(
        context: &motion_core::SceneContext<'_>,
        key: Key,
        surface: Arc<crate::VectorSurface>,
        bounds: Rect,
        dimensions: [u32; 2],
        limits: SimulationLimits,
    ) -> Result<Arc<Self>, ValidationError> {
        let identity = Arc::as_ptr(&surface) as usize;
        let geometry = [
            bounds.left.to_bits(),
            bounds.top.to_bits(),
            bounds.right.to_bits(),
            bounds.bottom.to_bits(),
        ];
        context
            .cache_with_cost(
                (
                    key,
                    identity,
                    geometry,
                    dimensions,
                    limits.max_bytes,
                    limits.max_work,
                ),
                || {
                    let result =
                        Self::new(surface.clone(), bounds, dimensions, limits).map(Arc::new);
                    (surface, result)
                },
                |(_, result)| result.as_ref().map_or(0, |grid| grid.bytes() as usize),
            )
            .1
            .clone()
    }

    /// Prepares an explicitly resolved grid under SimulationLimits; no downsampling fallback.
    pub fn new(
        surface: Arc<crate::VectorSurface>,
        bounds: Rect,
        dimensions: [u32; 2],
        limits: SimulationLimits,
    ) -> Result<Self, ValidationError> {
        motion_core::drawing::validate_rect(bounds, "particles.sdf", "bounds")?;
        let [width, height] = dimensions;
        let count = u64::from(width) * u64::from(height);
        if width < 2
            || height < 2
            || width > 2048
            || height > 2048
            || bounds.width() <= 0.0
            || bounds.height() <= 0.0
            || count * 4 + surface.triangles().len() as u64 * 512 > limits.max_bytes
        {
            return Err(limit("SDF resolution/byte allowance exhausted"));
        }
        let contains = |point: Point| {
            surface
                .triangles()
                .iter()
                .any(|triangle| inside_polygon(point, &triangle.vertices))
        };
        let mut boundary = Vec::new();
        let mut work = 0u64;
        for triangle in surface.triangles() {
            for index in 0..3 {
                let start = triangle.vertices[index];
                let end = triangle.vertices[(index + 1) % 3];
                let delta = [end.x - start.x, end.y - start.y];
                let length = libm::hypot(delta[0], delta[1]);
                if length == 0.0 {
                    continue;
                }
                let margin = surface.tolerance().min(length * 0.001).max(1e-9);
                let middle = Point::new((start.x + end.x) * 0.5, (start.y + end.y) * 0.5);
                work += surface.triangles().len() as u64 * 6;
                if work > limits.max_work {
                    return Err(limit("SDF boundary work exhausted"));
                }
                if contains(Point::new(
                    middle.x - delta[1] / length * margin,
                    middle.y + delta[0] / length * margin,
                )) != contains(Point::new(
                    middle.x + delta[1] / length * margin,
                    middle.y - delta[0] / length * margin,
                )) {
                    boundary.push((start, end));
                }
            }
        }
        if boundary.is_empty() {
            return Err(invalid("sdf", "Surface has no resolved boundary"));
        }
        let mut values = Vec::new();
        values
            .try_reserve_exact(count as usize)
            .map_err(|_| limit("SDF grid allocation failed"))?;
        for row in 0..height {
            for column in 0..width {
                work += surface.triangles().len() as u64 * 3 + boundary.len() as u64;
                if work > limits.max_work {
                    return Err(limit("SDF grid work exhausted"));
                }
                let point = Point::new(
                    bounds.left + f64::from(column) / f64::from(width - 1) * bounds.width(),
                    bounds.top + f64::from(row) / f64::from(height - 1) * bounds.height(),
                );
                let distance = boundary
                    .iter()
                    .map(|(start, end)| nearest(point, *start, *end).0)
                    .fold(f64::INFINITY, f64::min);
                values.push((distance * if contains(point) { -1.0 } else { 1.0 }) as f32);
            }
        }
        Ok(Self {
            bounds,
            dimensions,
            values: values.into(),
            surface,
            boundary: boundary.into(),
        })
    }

    /// Exact retained grid and boundary payload, suitable for SceneCache cost accounting.
    pub fn bytes(&self) -> u64 {
        self.values.len() as u64 * 4
            + self.boundary.len() as u64 * 32
            + self.surface.triangles().len() as u64
                * std::mem::size_of::<crate::SurfaceTriangle>() as u64
    }

    /// Bilinear grid sampling, with an exact immutable boundary guard within one cell diagonal.
    pub fn sample(&self, point: Point) -> (f64, [f64; 2]) {
        let cell = [
            self.bounds.width() / f64::from(self.dimensions[0] - 1),
            self.bounds.height() / f64::from(self.dimensions[1] - 1),
        ];
        let coordinate = [
            (point.x - self.bounds.left) / cell[0],
            (point.y - self.bounds.top) / cell[1],
        ];
        let outside = coordinate[0] < 0.0
            || coordinate[1] < 0.0
            || coordinate[0] > f64::from(self.dimensions[0] - 1)
            || coordinate[1] > f64::from(self.dimensions[1] - 1);
        let position = [
            coordinate[0].clamp(0.0, f64::from(self.dimensions[0] - 1)),
            coordinate[1].clamp(0.0, f64::from(self.dimensions[1] - 1)),
        ];
        let low = position.map(|value| libm::floor(value) as u32);
        let high = [
            (low[0] + 1).min(self.dimensions[0] - 1),
            (low[1] + 1).min(self.dimensions[1] - 1),
        ];
        let value =
            |column, row| f64::from(self.values[(row * self.dimensions[0] + column) as usize]);
        let values = [
            value(low[0], low[1]),
            value(high[0], low[1]),
            value(low[0], high[1]),
            value(high[0], high[1]),
        ];
        let fraction = [
            position[0] - f64::from(low[0]),
            position[1] - f64::from(low[1]),
        ];
        let distance = (values[0] * (1.0 - fraction[0]) + values[1] * fraction[0])
            * (1.0 - fraction[1])
            + (values[2] * (1.0 - fraction[0]) + values[3] * fraction[0]) * fraction[1];
        if outside || distance.abs() <= libm::hypot(cell[0], cell[1]) * 2.0 {
            let (distance, normal) = self
                .boundary
                .iter()
                .map(|(start, end)| nearest(point, *start, *end))
                .min_by(|first, second| first.0.total_cmp(&second.0))
                .unwrap_or((0.0, [1.0, 0.0]));
            if self
                .surface
                .triangles()
                .iter()
                .any(|triangle| inside_polygon(point, &triangle.vertices))
            {
                return (-distance, normal.map(|value| -value));
            }
            return (distance, normal);
        }
        let gradient = [
            ((values[1] - values[0]) * (1.0 - fraction[1]) + (values[3] - values[2]) * fraction[1])
                / cell[0],
            ((values[2] - values[0]) * (1.0 - fraction[0]) + (values[3] - values[1]) * fraction[0])
                / cell[1],
        ];
        let length = libm::hypot(gradient[0], gradient[1]);
        (
            distance,
            if length == 0.0 {
                [1.0, 0.0]
            } else {
                gradient.map(|value| value / length)
            },
        )
    }
}

/// Grid collision; velocity*step must remain below the smallest cell dimension.
#[derive(Clone, Debug)]
pub struct SdfCollider {
    /// Cached immutable collision grid.
    pub grid: Arc<SignedDistanceGrid>,
    /// Normal/tangential contact response.
    pub response: CollisionResponse,
}

impl SdfCollider {
    fn contact(&self, particle: &mut Particle, step: f32) -> Result<bool, ValidationError> {
        let cell = (self.grid.bounds.width() / f64::from(self.grid.dimensions[0] - 1))
            .min(self.grid.bounds.height() / f64::from(self.grid.dimensions[1] - 1));
        if f64::from(libm::hypotf(particle.velocity[0], particle.velocity[1]) * step) >= cell {
            return Err(invalid(
                "sdf",
                "Velocity times step must be smaller than a grid cell",
            ));
        }
        swept(particle, step, self.response, None, |point| {
            self.grid.sample(point)
        })
    }
}
collider!(
    SdfCollider,
    |collider: &SdfCollider| (collider.grid.boundary.len() as u64
        + collider.grid.surface.triangles().len() as u64 * 3
        + 8)
        * 64
);
