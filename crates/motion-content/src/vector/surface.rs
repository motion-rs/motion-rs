//! Bounded triangulation of filled vector geometry into sampleable surfaces.

use super::*;
use motion_core::{Point, geometry::flatten::Edge};

const MAX_EDGES: usize = 4096;

const MAX_EVENTS: usize = 65536;

const MAX_TRIANGLES: usize = 65536;

const MAX_WORK: u64 = 32_000_000;

/// A nondegenerate tree-space triangle in a filled geometric union.
#[derive(Clone, Copy, Debug)]
pub struct SurfaceTriangle {
    /// Original f64 vertices, never raster coordinates.
    pub vertices: [Point; 3],
    /// Positive area in squared tree units.
    pub area: f64,
    /// Index of the ordered fill stack retained by the owning surface.
    pub paint_index: usize,
}

/// Immutable prepared fill silhouette for deterministic area-uniform sampling.
/// Images/strokes are not filled path regions. Paint alpha is not a density;
/// zero-opacity groups are omitted, clips/holes and overlapping fills are honored.
#[derive(Clone, Debug)]
pub struct VectorSurface {
    triangles: Vec<SurfaceTriangle>,
    paints: Vec<SurfacePaint>,
    paint_stacks: Vec<Arc<[usize]>>,
    ends: Vec<f64>,
    area: f64,
    tolerance: f64,
}

#[derive(Clone, Debug)]
struct SurfacePaint {
    paint: Paint,
    transform: Transform,
    bounds: Rect,
    opacity: f64,
}

struct Region {
    rule: FillRule,
}

struct Domain {
    regions: Vec<usize>,
}

fn work(spent: &mut u64, amount: usize) -> Result<(), ValidationError> {
    *spent = spent
        .checked_add(amount as u64)
        .ok_or_else(|| limited("Surface work overflow"))?;
    if *spent > MAX_WORK {
        return Err(limited("Surface exceeds 32 million work units"));
    }
    Ok(())
}

fn add_region(
    path: &Path2D,
    transform: Transform,
    rule: FillRule,
    tolerance: f64,
    edges: &mut Vec<(Edge, usize)>,
    regions: &mut Vec<Region>,
    spent: &mut u64,
) -> Result<usize, ValidationError> {
    if regions.len() >= 8192 {
        return Err(limited("Surface exceeds 8192 fill/clip regions"));
    }
    let index = regions.len();
    let (source, used) = motion_core::geometry::flatten::surface_edges(
        path,
        transform,
        tolerance,
        MAX_EDGES.saturating_sub(edges.len()),
        MAX_WORK.saturating_sub(*spent),
    )?;
    *spent += used;
    if source.len() > MAX_EDGES.saturating_sub(edges.len()) {
        return Err(limited("Surface exceeds 4096 directed edges"));
    }
    for edge in source {
        if !edge.top.x.is_finite()
            || !edge.top.y.is_finite()
            || !edge.bottom.x.is_finite()
            || !edge.bottom.y.is_finite()
            || [edge.top.x, edge.top.y, edge.bottom.x, edge.bottom.y]
                .iter()
                .any(|value| value.abs() * 128.0 * f64::EPSILON > tolerance * 0.25)
        {
            return Err(invalid(
                "precision",
                "Surface coordinates cannot resolve the requested tolerance",
            ));
        }
        edges.push((edge, index));
    }
    regions.push(Region { rule });
    Ok(index)
}

impl VectorTree {
    /// Shares a single concurrent preparation at 0.0025 tree-unit Wang tolerance. The chord error bound is
    /// met with equality, so the inscribed polygon can lose up to about 2/3 of it times the perimeter in area.
    pub fn surface(&self) -> Result<Arc<VectorSurface>, ValidationError> {
        self.data
            .surface
            .get_or_init(|| self.surface_with_tolerance(0.0025).map(Arc::new))
            .clone()
    }

    /// Prepares a separate continuous geometric triangulation with an explicit
    /// positive finite curve tolerance. No rasterization or random rejection loop.
    pub fn surface_with_tolerance(&self, tolerance: f64) -> Result<VectorSurface, ValidationError> {
        fn masked(group: &VectorGroup) -> bool {
            group.mask.is_some()
                || group
                    .children
                    .iter()
                    .any(|item| matches!(item,VectorItem::Group(child) if masked(child)))
        }
        if masked(self.root()) {
            return Err(ValidationError::unsupported(
                "vector_tree.surface",
                "mask",
                "Alpha/luminance-masked artwork is not a binary filled surface",
            ));
        }
        if !tolerance.is_finite() || tolerance <= 0.0 {
            return Err(invalid(
                "tolerance",
                "Surface tolerance must be finite and positive",
            ));
        }
        let mut edges = Vec::new();
        let mut regions = Vec::new();
        let mut domains = Vec::new();
        let mut paints = Vec::new();
        let mut spent = 0;
        for leaf in self.paint_leaves() {
            if leaf.opacity == 0.0 {
                continue;
            }
            let VectorItem::Path(path) = leaf.item else {
                continue;
            };
            let Some((paint, rule)) = &path.fill else {
                continue;
            };
            let mut ids = vec![add_region(
                &path.path,
                leaf.transform,
                *rule,
                tolerance,
                &mut edges,
                &mut regions,
                &mut spent,
            )?];
            for clip in &leaf.clips {
                ids.push(add_region(
                    &clip.path,
                    clip.transform,
                    clip.rule,
                    tolerance,
                    &mut edges,
                    &mut regions,
                    &mut spent,
                )?);
            }
            domains.push(Domain { regions: ids });
            paints.push(SurfacePaint {
                paint: paint.clone(),
                transform: leaf.transform,
                bounds: path.path.bounding_box(),
                opacity: leaf.opacity,
            });
        }
        let mut cuts = Vec::new();
        for (edge, _) in &edges {
            cuts.extend([edge.top.y, edge.bottom.y]);
        }
        for (index, (first, _)) in edges.iter().enumerate() {
            for (second, _) in &edges[index + 1..] {
                work(&mut spent, 1)?;
                let top = first.top.y.max(second.top.y);
                let bottom = first.bottom.y.min(second.bottom.y);
                if top >= bottom {
                    continue;
                }
                let top_difference = first.at(top) - second.at(top);
                let bottom_difference = first.at(bottom) - second.at(bottom);
                if top_difference != 0.0
                    && bottom_difference != 0.0
                    && top_difference.is_sign_positive() != bottom_difference.is_sign_positive()
                {
                    let cut = top
                        + (bottom - top) * (top_difference / (top_difference - bottom_difference));
                    if !cut.is_finite() {
                        return Err(invalid("precision", "Surface intersection overflow"));
                    }
                    if cut > top && cut < bottom {
                        cuts.push(cut);
                    }
                    if cuts.len() > MAX_EVENTS {
                        return Err(limited("Surface intersection allowance exhausted"));
                    }
                }
            }
        }
        work(&mut spent, cuts.len() * cuts.len().max(1).ilog2() as usize)?;
        cuts.sort_by(f64::total_cmp);
        cuts.dedup();
        let mut dependencies = vec![Vec::new(); regions.len()];
        for (index, domain) in domains.iter().enumerate() {
            work(&mut spent, domain.regions.len())?;
            for region in &domain.regions {
                dependencies[*region].push(index);
            }
        }
        let mut triangles = Vec::new();
        let mut paint_stacks = Vec::new();
        let mut paint_references = 0usize;
        let mut ends = Vec::new();
        let mut area = 0.0;
        for band in cuts.windows(2) {
            let top = band[0];
            let bottom = band[1];
            work(&mut spent, edges.len() + regions.len() * 2 + domains.len())?;
            let mut active = edges
                .iter()
                .enumerate()
                .filter(|(_, (edge, _))| edge.top.y <= top && edge.bottom.y >= bottom)
                .map(|(index, (edge, _))| (index, edge.at(top) * 0.5 + edge.at(bottom) * 0.5))
                .collect::<Vec<_>>();
            work(
                &mut spent,
                active.len() * active.len().max(1).ilog2() as usize,
            )?;
            active.sort_by(|left, right| left.1.total_cmp(&right.1).then(left.0.cmp(&right.0)));
            let mut winding = vec![0i32; regions.len()];
            let mut filled = vec![false; regions.len()];
            let mut inside = vec![false; domains.len()];
            let mut count = 0usize;
            for pair in active.windows(2) {
                let (edge, owner) = edges[pair[0].0];
                winding[owner] += edge.winding;
                let now = match regions[owner].rule {
                    FillRule::NonZero => winding[owner] != 0,
                    FillRule::EvenOdd => winding[owner] % 2 != 0,
                };
                if now != filled[owner] {
                    filled[owner] = now;
                    for domain in &dependencies[owner] {
                        work(&mut spent, domains[*domain].regions.len())?;
                        let next = domains[*domain].regions.iter().all(|index| filled[*index]);
                        if next != inside[*domain] {
                            if next {
                                count += 1
                            } else {
                                count -= 1
                            }
                            inside[*domain] = next;
                        }
                    }
                }
                if count == 0 || pair[0].1 == pair[1].1 {
                    continue;
                }
                let left = edge;
                let right = edges[pair[1].0].0;
                let left_top = Point::new(left.at(top), top);
                let right_top = Point::new(right.at(top), top);
                let right_bottom = Point::new(right.at(bottom), bottom);
                let left_bottom = Point::new(left.at(bottom), bottom);
                work(&mut spent, inside.len())?;
                let stack = inside
                    .iter()
                    .enumerate()
                    .filter_map(|(index, active)| active.then_some(index))
                    .collect::<Vec<_>>();
                paint_references += stack.len();
                if paint_stacks.len() >= MAX_TRIANGLES || paint_references > 1_048_576 {
                    return Err(limited("Surface paint stack allowance exhausted"));
                }
                let paint_index = paint_stacks.len();
                paint_stacks.push(Arc::<[usize]>::from(stack));
                for vertices in [
                    [left_top, right_top, right_bottom],
                    [left_top, right_bottom, left_bottom],
                ] {
                    let [first, second, third] = vertices;
                    let amount = ((second.x - first.x) * (third.y - first.y)
                        - (second.y - first.y) * (third.x - first.x))
                        .abs()
                        * 0.5;
                    if !amount.is_finite() {
                        return Err(invalid("area", "Surface area overflow"));
                    }
                    if amount == 0.0 {
                        continue;
                    }
                    if triangles.len() == MAX_TRIANGLES {
                        return Err(limited("Surface exceeds 65536 triangles"));
                    }
                    triangles.push(SurfaceTriangle {
                        vertices,
                        area: amount,
                        paint_index,
                    });
                }
            }
        }
        work(
            &mut spent,
            triangles.len() * triangles.len().max(1).ilog2() as usize,
        )?;
        triangles.sort_by(|left, right| left.area.total_cmp(&right.area));
        for triangle in &triangles {
            let next = area + triangle.area;
            if !next.is_finite() {
                return Err(invalid("area", "Surface area overflow"));
            }
            if next == area {
                return Err(invalid(
                    "precision",
                    "Surface area weights cannot represent a nonzero triangle",
                ));
            }
            area = next;
            ends.push(area);
        }
        Ok(VectorSurface {
            triangles,
            paints,
            paint_stacks,
            ends,
            area,
            tolerance,
        })
    }

    /// Samples the filled geometric union by triangle area and seeded Halton
    /// points. At most one million points; clones share cached preparation.
    pub fn sample_points(&self, count: usize, seed: u64) -> Result<Vec<Point>, ValidationError> {
        if count > 1_000_000 {
            return Err(limited(
                "Surface sampling admits at most one million points",
            ));
        }
        if count == 0 {
            return Ok(Vec::new());
        }
        self.surface()?.sample_points(count, seed)
    }

    /// Area-uniform points with ordered fill color; paint alpha is never a sampling density.
    pub fn sample_colored_points(
        &self,
        count: usize,
        seed: u64,
    ) -> Result<Vec<(Point, [f32; 4])>, ValidationError> {
        self.surface()?.sample_colored_points(count, seed)
    }
}

fn halton(mut index: u64, base: u64) -> f64 {
    let mut factor = 1.0;
    let mut value = 0.0;
    while index > 0 {
        factor /= base as f64;
        value += factor * (index % base) as f64;
        index /= base;
    }
    value
}

fn shift(mut seed: u64) -> f64 {
    seed = (seed ^ (seed >> 30)).wrapping_mul(0xbf58476d1ce4e5b9);
    seed = (seed ^ (seed >> 27)).wrapping_mul(0x94d049bb133111eb);
    ((seed ^ (seed >> 31)) >> 11) as f64 * (1.0 / (1u64 << 53) as f64)
}

impl VectorSurface {
    /// Positive area of the union (overlap counted once).
    pub fn area(&self) -> f64 {
        self.area
    }

    /// Read-only continuous triangles, including clipped/holed decomposition.
    pub fn triangles(&self) -> &[SurfaceTriangle] {
        &self.triangles
    }

    /// Requested maximum Wang curve-deviation tolerance in tree coordinates.
    pub fn tolerance(&self) -> f64 {
        self.tolerance
    }

    /// Pure deterministic area selection and barycentric Halton sampling.
    pub fn sample_points(&self, count: usize, seed: u64) -> Result<Vec<Point>, ValidationError> {
        if count > 1_000_000 {
            return Err(limited(
                "Surface sampling admits at most one million points",
            ));
        }
        if count > 0 && self.area == 0.0 {
            return Err(invalid(
                "area",
                "Cannot sample a surface without filled area",
            ));
        }
        let offsets = [
            shift(seed),
            shift(seed.wrapping_add(0x9e3779b97f4a7c15)),
            shift(seed.wrapping_add(0x3c6ef372fe94f82a)),
        ];
        let unit = |index, base, offset| {
            let value = halton(index, base) + offset;
            value - libm::floor(value)
        };
        let mut points = Vec::with_capacity(count);
        for index in 1..=count as u64 {
            let choice = unit(index, 2, offsets[0]) * self.area;
            let triangle = &self.triangles[self
                .ends
                .partition_point(|end| *end <= choice)
                .min(self.triangles.len() - 1)];
            let root = libm::sqrt(unit(index, 3, offsets[1]));
            let other = unit(index, 5, offsets[2]);
            let weights = [1.0 - root, root * (1.0 - other), root * other];
            let [first, second, third] = triangle.vertices;
            points.push(Point::new(
                first.x * weights[0] + second.x * weights[1] + third.x * weights[2],
                first.y * weights[0] + second.y * weights[1] + third.y * weights[2],
            ));
        }
        Ok(points)
    }

    /// Samples the same geometric points as sample_points and composites their retained fill stacks.
    pub fn sample_colored_points(
        &self,
        count: usize,
        seed: u64,
    ) -> Result<Vec<(Point, [f32; 4])>, ValidationError> {
        let points = self.sample_points(count, seed)?;
        let mut output = Vec::new();
        output
            .try_reserve_exact(points.len())
            .map_err(|_| limited("Colored point allocation failed"))?;
        let mut spent = 0;
        for (index, point) in points.into_iter().enumerate() {
            let choice = (halton(index as u64 + 1, 2) + shift(seed)).fract() * self.area;
            let triangle = &self.triangles[self
                .ends
                .partition_point(|end| *end <= choice)
                .min(self.triangles.len() - 1)];
            let mut color = [0.0_f64; 4];
            for paint in self.paint_stacks[triangle.paint_index].iter() {
                work(&mut spent, 16)?;
                let paint = &self.paints[*paint];
                let inverse = paint
                    .transform
                    .inverse()
                    .ok_or_else(|| invalid("paint", "Singular surface paint transform"))?;
                let position = inverse.apply(point);
                let sample = match &paint.paint {
                    Paint::Solid(color) => color.to_linear_premultiplied(),
                    Paint::Linear(gradient) => gradient.sample(position, paint.bounds)?,
                    Paint::Radial(gradient) => gradient.sample(position, paint.bounds)?,
                    Paint::Sweep(gradient) => gradient.sample(position, paint.bounds)?,
                    _ => {
                        return Err(ValidationError::unsupported(
                            "vector_tree.surface",
                            "paint",
                            "Colored surface emission requires a solid or gradient fill",
                        ));
                    }
                }
                .map(|value| value * paint.opacity);
                color = std::array::from_fn(|channel| {
                    sample[channel] + color[channel] * (1.0 - sample[3])
                });
            }
            output.push((point, color.map(|value| value as f32)));
        }
        Ok(output)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use motion_core::Color;

    fn tree(path: Path2D, rule: FillRule) -> Result<VectorTree, ValidationError> {
        let mut path = VectorPath::filled(path, Color::WHITE);
        path.fill.as_mut().unwrap().1 = rule;
        VectorTree::new(
            Rect::new(0.0, 0.0, 30.0, 10.0),
            VectorGroup {
                children: vec![VectorItem::Path(path)],
                ..Default::default()
            },
        )
    }

    #[test]
    fn surfaces_respect_holes_clips_overlap_and_area_weighted_deterministic_points()
    -> motion_core::Result<()> {
        let hole = tree(
            Path2D::parse_svg("M0 0h10v10h-10z M3 3h4v4h-4z")?,
            FillRule::EvenOdd,
        )?;
        assert_eq!(hole.surface()?.area(), 84.0);
        let points = hole.sample_points(4096, 42)?;
        assert!(points.iter().all(|p| p.x >= 0.0
            && p.x <= 10.0
            && p.y >= 0.0
            && p.y <= 10.0
            && !(p.x > 3.0 && p.x < 7.0 && p.y > 3.0 && p.y < 7.0)));
        assert_eq!(points, hole.sample_points(4096, 42)?);
        assert_ne!(points, hole.sample_points(4096, 43)?);
        let mut group = hole.root().clone();
        group.transform =
            Transform::scale_non_uniform(2.0, 1.0).then(Transform::translate(10.0, -2.0));
        group.clip = Some(Path2D::from_rect(Rect::new(1.0, 1.0, 9.0, 9.0))?.into());
        let clipped = VectorTree::new(hole.view_box(), group)?;
        assert_eq!(clipped.surface()?.area(), 96.0);
        assert!(
            clipped
                .sample_points(100, 0)?
                .iter()
                .all(|p| p.x >= 12.0 && p.x <= 28.0 && p.y >= -1.0 && p.y <= 7.0)
        );
        let mut group = VectorGroup::default();
        for left in [0.0, 5.0] {
            group.children.push(VectorItem::Path(VectorPath::filled(
                Path2D::from_rect(Rect::new(left, 0.0, left + 10.0, 10.0))?,
                Color::WHITE,
            )));
        }
        let union = VectorTree::new(hole.view_box(), group)?;
        assert_eq!(union.surface()?.area(), 150.0);
        let bow = tree(
            Path2D::parse_svg("M0 0L10 10L0 10L10 0Z")?,
            FillRule::EvenOdd,
        )?;
        assert_eq!(bow.surface()?.area(), 50.0);
        let nonzero = tree(
            Path2D::parse_svg("M0 0h10v10h-10z M3 3v4h4v-4z")?,
            FillRule::NonZero,
        )?;
        assert_eq!(nonzero.surface()?.area(), 84.0);
        let mut group = VectorGroup::default();
        for (left, width) in [(0.0, 1.0), (10.0, 3.0)] {
            group.children.push(VectorItem::Path(VectorPath::filled(
                Path2D::from_rect(Rect::new(left, 0.0, left + width, 1.0))?,
                Color::WHITE,
            )));
        }
        let weighted = VectorTree::new(hole.view_box(), group)?;
        let points = weighted.sample_points(8192, 31)?;
        let first = points.iter().filter(|point| point.x < 2.0).count();
        assert!((first as f64 / 8192.0 - 0.25).abs() < 0.01);
        for _ in 0..3 {
            let copy = hole.clone();
            let reference = hole.surface()?;
            std::thread::spawn(move || {
                assert!(Arc::ptr_eq(&reference, &copy.surface().unwrap()));
                assert_eq!(copy.sample_points(4096, 42).unwrap(), points_for(&copy));
            })
            .join()
            .unwrap();
        }
        assert!(hole.sample_points(1_000_001, 0).is_err());
        assert!(hole.surface_with_tolerance(f64::NAN).is_err());
        Ok(())
    }

    fn points_for(tree: &VectorTree) -> Vec<Point> {
        tree.sample_points(4096, 42).unwrap()
    }

    #[test]
    fn curved_and_straight_shapes_are_sampled_in_proportion_to_their_area()
    -> motion_core::Result<()> {
        use motion_core::Shape;
        let mut group = VectorGroup::default();
        group.children.push(VectorItem::Path(VectorPath::filled(
            motion_core::Circle::new(Point::new(5.0, 5.0), 3.0)?.path()?,
            Color::WHITE,
        )));
        group.children.push(VectorItem::Path(VectorPath::filled(
            Path2D::from_rect(Rect::new(10.0, 0.0, 16.0, 6.0))?,
            Color::WHITE,
        )));
        let tree = VectorTree::new(Rect::new(0.0, 0.0, 30.0, 10.0), group)?;
        let circle = std::f64::consts::PI * 9.0;
        let expected = circle / (circle + 36.0);
        let count = 40_000;
        let points = tree.sample_points(count, 7)?;
        let inside = points.iter().filter(|point| point.x < 8.5).count() as f64 / count as f64;
        // Five standard deviations of the binomial estimate, far above the tolerance bias.
        assert!(
            (inside - expected).abs() < 5.0 * (expected * (1.0 - expected) / count as f64).sqrt(),
            "{inside} against {expected}"
        );
        let quadrants = |center: f64| {
            points
                .iter()
                .filter(|point| point.x < 8.5 && point.x < center)
                .count() as f64
                / points.iter().filter(|point| point.x < 8.5).count() as f64
        };
        assert!((quadrants(5.0) - 0.5).abs() < 0.02);
        Ok(())
    }

    #[test]
    fn curves_and_resource_refusals_do_not_turn_into_raster_or_unbounded_work()
    -> motion_core::Result<()> {
        use motion_core::Shape;
        let circle = tree(
            motion_core::Circle::new(Point::new(5.0, 5.0), 3.0)?.path()?,
            FillRule::NonZero,
        )?;
        assert!((circle.surface()?.area() - std::f64::consts::PI * 9.0).abs() < 0.03);
        assert!(
            circle
                .sample_points(1000, 9)?
                .iter()
                .all(|p| (p.x - 5.0) * (p.x - 5.0) + (p.y - 5.0) * (p.y - 5.0) < 9.01)
        );
        assert!(circle.surface_with_tolerance(1e-16).is_err());
        assert!(circle.surface()?.area() > 28.0);
        let next = f64::from_bits(1.0f64.to_bits() + 1);
        let thin = tree(
            Path2D::from_rect(Rect::new(0.0, 1.0, 2.0, next))?,
            FillRule::NonZero,
        )?;
        assert_eq!(thin.surface()?.area(), 2.0 * (next - 1.0));
        let empty = VectorTree::new(circle.view_box(), VectorGroup::default())?;
        assert!(empty.sample_points(1, 0).is_err());
        assert!(empty.sample_points(0, 0)?.is_empty());
        let mut deep = VectorGroup::default();
        for _ in 0..50 {
            deep = VectorGroup {
                children: vec![VectorItem::Group(deep)],
                ..Default::default()
            };
        }
        assert!(VectorTree::new(circle.view_box(), deep).is_err());
        Ok(())
    }
}
