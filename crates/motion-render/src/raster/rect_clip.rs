//! Bounded geometric clip domains, with convex rectangle and general Shape paths.

use super::polygon::shape_clip::{self, Region};
use super::*;
use motion_core::{PathVerb, Point, Rect, RectClip, ValidationError};

struct Geometry {
    points: Vec<Point>,
    coverage: usize,
    region: Option<Region>,
    parent: Option<usize>,
}

pub(crate) struct Preparation {
    stack: [Option<usize>; 129],
    depth: usize,
    geometry: Vec<Geometry>,
}

impl Default for Preparation {
    fn default() -> Self {
        Self {
            stack: [None; 129],
            depth: 0,
            geometry: Vec::new(),
        }
    }
}

impl Preparation {
    pub(crate) fn push(
        &mut self,
        clip: Option<&RectClip>,
        shape: Option<&motion_core::ShapeClip>,
        frame: &mut PreparedScene,
        tolerance: f64,
        budget: &mut Budget,
        path: &str,
    ) -> Result<Option<usize>, ValidationError> {
        let mut current = self.stack[self.depth];
        if let Some(clip) = clip {
            current = Some(self.rectangle(clip, current, frame, tolerance, budget, path)?);
        }
        if let Some(shape) = shape {
            current = Some(match &shape.geometry {
                motion_core::drawing::ClipGeometry::Rectangle(bounds) => self.rectangle(
                    &RectClip {
                        bounds: *bounds,
                        transform: shape.transform,
                    },
                    current,
                    frame,
                    tolerance,
                    budget,
                    path,
                )?,
                motion_core::drawing::ClipGeometry::Path(_) => {
                    let region = Region::new(shape, tolerance, budget, path)?;
                    self.region(region, current, frame, tolerance, budget, path)?
                }
            });
        }
        self.depth += 1;
        self.stack[self.depth] = current;
        Ok(current.map(|index| self.geometry[index].coverage))
    }

    fn rectangle(
        &mut self,
        clip: &RectClip,
        parent: Option<usize>,
        frame: &mut PreparedScene,
        tolerance: f64,
        budget: &mut Budget,
        path: &str,
    ) -> Result<usize, ValidationError> {
        if parent.is_some_and(|index| self.geometry[index].region.is_some()) {
            let region = Region::rectangle(clip, tolerance, budget, path)?;
            return self.region(region, parent, frame, tolerance, budget, path);
        }
        let viewport = [
            Point::ORIGIN,
            Point::new(f64::from(frame.size.width), 0.0),
            Point::new(f64::from(frame.size.width), f64::from(frame.size.height)),
            Point::new(0.0, f64::from(frame.size.height)),
        ];
        let subject = parent.map_or(viewport.as_slice(), |index| {
            self.geometry[index].points.as_slice()
        });
        let points = intersection(subject, clip, budget, path)?;
        let strip_start = frame.strips.len();
        let span_start = frame.spans.len();
        let mut bounds = Rect::default();
        if let Some(first) = points.first() {
            bounds = Rect::new(first.x, first.y, first.x, first.y);
            for point in &points {
                budget.work(1, path)?;
                bounds.left = bounds.left.min(point.x);
                bounds.top = bounds.top.min(point.y);
                bounds.right = bounds.right.max(point.x);
                bounds.bottom = bounds.bottom.max(point.y);
            }
            let verbs = points
                .iter()
                .enumerate()
                .map(|(index, point)| {
                    if index == 0 {
                        PathVerb::MoveTo(*point)
                    } else {
                        PathVerb::LineTo(*point)
                    }
                })
                .chain(std::iter::once(PathVerb::Close));
            frame.polygon(
                verbs,
                motion_core::Transform::IDENTITY,
                motion_core::FillRule::NonZero,
                tolerance,
                budget,
                path,
            )?;
        }
        let coverage = frame.clips.len();
        budget.push(
            &mut frame.clips,
            ClipRecord {
                bounds: pixel_bounds(bounds, frame.size),
                strips: strip_start..frame.strips.len(),
                spans: span_start..frame.spans.len(),
            },
            path,
        )?;
        let index = self.geometry.len();
        budget.push(
            &mut self.geometry,
            Geometry {
                points,
                coverage,
                region: None,
                parent: None,
            },
            path,
        )?;
        Ok(index)
    }

    fn region(
        &mut self,
        region: Region,
        parent: Option<usize>,
        frame: &mut PreparedScene,
        tolerance: f64,
        budget: &mut Budget,
        path: &str,
    ) -> Result<usize, ValidationError> {
        let strip_start = frame.strips.len();
        let span_start = frame.spans.len();
        let bounds = {
            let viewport = [
                Point::ORIGIN,
                Point::new(f64::from(frame.size.width), 0.0),
                Point::new(f64::from(frame.size.width), f64::from(frame.size.height)),
                Point::new(0.0, f64::from(frame.size.height)),
            ];
            let mut base = viewport.as_slice();
            let mut regions = Vec::new();
            budget.push(&mut regions, &region, path)?;
            let mut ancestor = parent;
            while let Some(index) = ancestor {
                budget.work(1, path)?;
                let geometry = &self.geometry[index];
                if let Some(region) = &geometry.region {
                    budget.push(&mut regions, region, path)?;
                    ancestor = geometry.parent;
                } else {
                    base = &geometry.points;
                    break;
                }
            }
            let bounds = shape_clip::rasterize(frame, base, &regions, tolerance, budget, path)?;
            budget.release(regions.len() as u64 * size_of::<&Region>() as u64);
            bounds
        };
        let coverage = frame.clips.len();
        budget.push(
            &mut frame.clips,
            ClipRecord {
                bounds,
                strips: strip_start..frame.strips.len(),
                spans: span_start..frame.spans.len(),
            },
            path,
        )?;
        let index = self.geometry.len();
        budget.push(
            &mut self.geometry,
            Geometry {
                points: Vec::new(),
                coverage,
                region: Some(region),
                parent,
            },
            path,
        )?;
        Ok(index)
    }

    pub(crate) fn begin_mask(&mut self) {
        self.depth += 1;
        self.stack[self.depth] = None;
    }

    pub(crate) fn pop(&mut self) {
        self.depth -= 1;
    }

    pub(crate) fn release(self, budget: &mut Budget) -> Result<(), ValidationError> {
        budget.work(self.geometry.len() as u64, "prepared.rect_clips")?;
        budget.release(
            self.geometry.len() as u64 * size_of::<Geometry>() as u64
                + self
                    .geometry
                    .iter()
                    .map(|geometry| {
                        geometry.points.len() as u64 * size_of::<Point>() as u64
                            + geometry.region.as_ref().map_or(0, Region::bytes)
                    })
                    .sum::<u64>(),
        );
        Ok(())
    }
}

fn intersection(
    subject: &[Point],
    clip: &RectClip,
    budget: &mut Budget,
    path: &str,
) -> Result<Vec<Point>, ValidationError> {
    let bounds = clip.bounds;
    let corners = [
        Point::new(bounds.left, bounds.top),
        Point::new(bounds.right, bounds.top),
        Point::new(bounds.right, bounds.bottom),
        Point::new(bounds.left, bounds.bottom),
    ]
    .map(|point| clip.transform.apply(point));
    budget.work(4, path)?;
    let direction = |first: Point, second: Point| {
        let horizontal = second.x - first.x;
        let vertical = second.y - first.y;
        let scale = horizontal.abs().max(vertical.abs());
        if scale == 0.0 {
            (0.0, 0.0)
        } else {
            (horizontal / scale, vertical / scale)
        }
    };
    let first = direction(corners[0], corners[1]);
    let second = direction(corners[0], corners[3]);
    let orientation = first.0 * second.1 - first.1 * second.0;
    if orientation == 0.0 || subject.is_empty() {
        return Ok(Vec::new());
    }
    if !orientation.is_finite() {
        return Err(ValidationError::invalid(
            path,
            "rect_clip",
            "Rectangle orientation must remain finite",
        ));
    }
    let mut polygon = Vec::new();
    for point in subject {
        budget.work(1, path)?;
        budget.push(&mut polygon, *point, path)?;
    }
    for edge in 0..4 {
        if polygon.is_empty() {
            break;
        }
        let start = corners[edge];
        let normal = direction(start, corners[(edge + 1) % 4]);
        let distance = |point: Point| -> Result<f64, ValidationError> {
            let value = orientation.signum()
                * (normal.0 * (point.y - start.y) - normal.1 * (point.x - start.x));
            let uncertainty = (normal.0.abs() * (point.y.abs() + start.y.abs())
                + normal.1.abs() * (point.x.abs() + start.x.abs()))
                * (32.0 * f64::EPSILON);
            if uncertainty > 1.0e-7 && value.abs() <= uncertainty {
                return Err(ValidationError::invalid(
                    path,
                    "rect_clip",
                    "Visible rectangle cancellation exceeds the 1e-7 device-pixel precision guard",
                ));
            }
            if value.is_finite() && uncertainty.is_finite() {
                Ok(value)
            } else {
                Err(ValidationError::invalid(
                    path,
                    "rect_clip",
                    "Rectangle clipping exceeded finite geometric precision",
                ))
            }
        };
        let mut result = Vec::new();
        let mut previous = *polygon.last().expect("nonempty convex polygon");
        budget.work(1, path)?;
        let mut previous_distance = distance(previous)?;
        for current in &polygon {
            budget.work(1, path)?;
            let current_distance = distance(*current)?;
            if (previous_distance >= 0.0) != (current_distance >= 0.0) {
                let scale = previous_distance.abs().max(current_distance.abs());
                let ratio = (previous_distance / scale)
                    / (previous_distance / scale - current_distance / scale);
                let point = Point::new(
                    if normal.0 == 0.0 {
                        start.x
                    } else {
                        previous.x + (current.x - previous.x) * ratio
                    },
                    if normal.1 == 0.0 {
                        start.y
                    } else {
                        previous.y + (current.y - previous.y) * ratio
                    },
                );
                budget.push(&mut result, point, path)?;
            }
            if current_distance >= 0.0 {
                budget.push(&mut result, *current, path)?;
            }
            previous = *current;
            previous_distance = current_distance;
        }
        budget.release(polygon.len() as u64 * size_of::<Point>() as u64);
        polygon = result;
    }
    Ok(polygon)
}
