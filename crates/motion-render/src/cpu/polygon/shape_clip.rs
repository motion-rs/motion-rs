//! Intersection coverage with independent winding for each flattened Shape.

use motion_core::{Rect, ValidationError};

use super::*;

pub(crate) struct Region {
    edges: Vec<Edge>,
    rule: FillRule,
}

impl Region {
    pub(crate) fn new(
        clip: &motion_core::ShapeClip,
        tolerance: f64,
        budget: &mut Budget,
        path: &str,
    ) -> Result<Self, ValidationError> {
        match &clip.geometry {
            motion_core::drawing::ClipGeometry::Rectangle(bounds) => Self::rectangle(
                &motion_core::RectClip {
                    bounds: *bounds,
                    transform: clip.transform,
                },
                tolerance,
                budget,
                path,
            ),
            motion_core::drawing::ClipGeometry::Path(geometry) => Ok(Self {
                edges: path_edges(
                    geometry.verbs().iter().copied(),
                    clip.transform,
                    tolerance,
                    budget,
                    path,
                )?,
                rule: clip.rule,
            }),
        }
    }

    pub(crate) fn rectangle(
        clip: &motion_core::RectClip,
        tolerance: f64,
        budget: &mut Budget,
        path: &str,
    ) -> Result<Self, ValidationError> {
        let bounds = clip.bounds;
        let points = [
            Point::new(bounds.left, bounds.top),
            Point::new(bounds.right, bounds.top),
            Point::new(bounds.right, bounds.bottom),
            Point::new(bounds.left, bounds.bottom),
        ];
        Self::points(&points, clip.transform, tolerance, budget, path)
    }

    fn points(
        points: &[Point],
        transform: Transform,
        tolerance: f64,
        budget: &mut Budget,
        path: &str,
    ) -> Result<Self, ValidationError> {
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
        Ok(Self {
            edges: path_edges(verbs, transform, tolerance, budget, path)?,
            rule: FillRule::NonZero,
        })
    }

    pub(crate) fn bytes(&self) -> u64 {
        self.edges.len() as u64 * size_of::<Edge>() as u64
    }
}

#[derive(Clone, Copy)]
struct Boundary {
    edge: Edge,
    group: usize,
    horizontal: f64,
}

pub(crate) fn rasterize(
    frame: &mut PreparedScene,
    base: &[Point],
    regions: &[&Region],
    tolerance: f64,
    budget: &mut Budget,
    path: &str,
) -> Result<PixelRect, ValidationError> {
    let base = Region::points(base, Transform::IDENTITY, tolerance, budget, path)?;
    let mut edges = Vec::new();
    let mut groups = Vec::new();
    let mut rules = Vec::new();
    let mut bounds = Rect::new(
        0.0,
        0.0,
        f64::from(frame.size.width),
        f64::from(frame.size.height),
    );
    for (group, region) in regions
        .iter()
        .copied()
        .chain(std::iter::once(&base))
        .enumerate()
    {
        budget.work(1, path)?;
        budget.push(&mut rules, region.rule, path)?;
        let mut hull = Rect::new(
            f64::INFINITY,
            f64::INFINITY,
            f64::NEG_INFINITY,
            f64::NEG_INFINITY,
        );
        let admitted = budget.work(region.edges.len() as u64, path).is_ok();
        for edge in &region.edges {
            if !admitted {
                budget.work(1, path)?;
            }
            hull.left = hull.left.min(edge.top.x).min(edge.bottom.x);
            hull.right = hull.right.max(edge.top.x).max(edge.bottom.x);
            hull.top = hull.top.min(edge.top.y);
            hull.bottom = hull.bottom.max(edge.bottom.y);
            budget.push(&mut edges, *edge, path)?;
            budget.push(&mut groups, group, path)?;
        }
        if region.edges.is_empty() {
            bounds = Rect::default();
        } else {
            bounds.left = bounds.left.max(hull.left);
            bounds.top = bounds.top.max(hull.top);
            bounds.right = bounds.right.min(hull.right).max(bounds.left);
            bounds.bottom = bounds.bottom.min(hull.bottom).max(bounds.top);
        }
    }
    let pixels = pixel_bounds(bounds, frame.size);
    let left = pixels.left / 4 * 4;
    let right = pixels.right;
    let mut row = pixels.top / 4 * 4;
    if !pixels.is_empty() {
        while row < pixels.bottom {
            budget.work(1, path)?;
            let height = (frame.size.height - row).min(4);
            let count = u64::from(right - left) * u64::from(height);
            budget.work(count, path)?;
            let mut areas = scratch::<f64>(count, budget, path)?;
            areas.resize(count as usize, 0.0);
            for vertical in row..row + height {
                let offset = (vertical - row) as usize * (right - left) as usize;
                integrate(
                    &edges,
                    &groups,
                    &rules,
                    Band {
                        row: vertical,
                        left,
                        right,
                    },
                    &mut areas[offset..offset + (right - left) as usize],
                    &mut RasterScope::new(budget, path),
                )?;
            }
            frame.polygon_band(
                &mut areas,
                Band { row, left, right },
                height,
                &mut RasterScope::new(budget, path),
            )?;
            budget.release(count * size_of::<f64>() as u64);
            row += height;
        }
    }
    budget.release(
        base.bytes()
            + edges.len() as u64 * size_of::<Edge>() as u64
            + groups.len() as u64 * size_of::<usize>() as u64
            + rules.len() as u64 * size_of::<FillRule>() as u64,
    );
    Ok(pixels)
}

fn inside(winding: i64, rule: FillRule) -> bool {
    match rule {
        FillRule::NonZero => winding != 0,
        FillRule::EvenOdd => winding % 2 != 0,
    }
}

fn integrate(
    edges: &[Edge],
    groups: &[usize],
    rules: &[FillRule],
    band: Band,
    areas: &mut [f64],
    scope: &mut RasterScope<'_>,
) -> Result<(), ValidationError> {
    let (row, right) = (band.row, band.right);
    let (budget, path) = (&mut *scope.budget, scope.path);
    let mut cuts = intersections::row_cuts(edges, row, right, budget, path)?;
    let mut columns = ColumnScratch::default();
    sort_work(cuts.len(), budget, path)?;
    cuts.sort_unstable_by(f64::total_cmp);
    let mut crossings = scratch::<Boundary>(edges.len() as u64, budget, path)?;
    let mut windings = scratch::<i64>(rules.len() as u64, budget, path)?;
    budget.work(rules.len() as u64, path)?;
    windings.resize(rules.len(), 0);
    let top = f64::from(row);
    for interval in cuts.windows(2) {
        budget.work(1, path)?;
        let [start, end] = [interval[0], interval[1]];
        if start == end {
            continue;
        }
        crossings.clear();
        budget.work(edges.len() as u64, path)?;
        for (index, edge) in edges.iter().enumerate() {
            if edge.top.y <= start && edge.bottom.y >= end {
                crossings.push(Boundary {
                    edge: *edge,
                    group: groups[index],
                    horizontal: edge.at(start) * 0.5 + edge.at(end) * 0.5,
                });
            }
        }
        sort_work(crossings.len(), budget, path)?;
        crossings.sort_unstable_by(|first, second| first.horizontal.total_cmp(&second.horizontal));
        budget.work(rules.len() as u64, path)?;
        windings.fill(0);
        let mut active = 0usize;
        let mut previous = None;
        for crossing in &crossings {
            budget.work(1, path)?;
            if active == rules.len() {
                integrate_trapezoid(
                    previous.expect("active clip interval"),
                    crossing.edge,
                    start - top,
                    end - top,
                    band,
                    areas,
                    &mut columns,
                    &mut RasterScope::new(budget, path),
                )?;
            }
            let winding = &mut windings[crossing.group];
            let was_inside = inside(*winding, rules[crossing.group]);
            *winding += i64::from(crossing.edge.winding);
            let is_inside = inside(*winding, rules[crossing.group]);
            if !was_inside && is_inside {
                active += 1;
            }
            if was_inside && !is_inside {
                active -= 1;
            }
            previous = Some(crossing.edge);
        }
        budget.work(rules.len() as u64, path)?;
        if windings.iter().any(|winding| *winding != 0) {
            return Err(ValidationError::invalid(
                path,
                "shape_clip",
                "Unbalanced geometric clip winding",
            ));
        }
    }
    budget.release(
        edges.len() as u64 * size_of::<Boundary>() as u64
            + rules.len() as u64 * size_of::<i64>() as u64
            + cuts.len() as u64 * size_of::<f64>() as u64,
    );
    Ok(())
}
