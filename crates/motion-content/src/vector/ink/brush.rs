//! Brush strokes and ink fills rendered with ink materials.

use super::Ink;
use crate::ImageAsset;
use motion_core::{
    Canvas, Color, DrawContext, Drawable, FillRule, ImagePattern, Paint, Path2D, PathVerb, Point,
    Rect, Transform, ValidationError, Vector2,
};
use std::sync::{Arc, OnceLock};

mod advanced;

pub use advanced::{
    BrushCap, BrushPreset, BrushTiming, InputSample, NibMode, Paper, PigmentMix, PigmentPainting,
};

#[derive(Clone, Debug)]
struct Prepared {
    path: Arc<Path2D>,
    paint: Paint,
    bounds: Rect,
}

/// Pressure-sensitive pen stroke, recorded as nonzero path fills. This is the single brush
/// implementation: `new` captures the defaults of a fixed-angle elliptical nib and the builder
/// methods in `brush::advanced` (pressure curves, caps, bristles, paper, timing) refine it.
/// Curves use bounded arc-length sampling (at most 256 intervals by default); nib caps use
/// 24-sided outlines.
/// Optional Ink reuses the deterministic material kernel in local coordinates,
/// with at most 262144 texture texels and, by default, 32 million distance evaluations.
/// Every render quality evaluates the full material.
/// Explicit workloads may raise the latter through [`Self::material_work_limit`].
/// This is geometric pen/material rendering, not fluid simulation. Paper here is
/// stroke-local; use untransformed design coordinates for canvas-fixed paper.
#[derive(Clone, Debug)]
pub struct BrushStroke {
    path: Path2D,
    width: f64,
    color: Color,
    aspect: f64,
    angle: f64,
    pressure: [f64; 3],
    load: [f64; 2],
    progress: f64,
    ink: Option<Ink>,
    material_work_limit: u64,
    advanced: advanced::Options,
    full_material: Arc<OnceLock<Result<Vec<Prepared>, ValidationError>>>,
}

fn invalid(field: &str, message: &str) -> ValidationError {
    ValidationError::invalid("brush_stroke", field, message)
}

impl BrushStroke {
    /// Captures finite geometry; validation is deferred to recording.
    pub fn new(path: Path2D, width: f64, color: Color) -> Self {
        Self {
            path,
            width,
            color,
            aspect: 1.0,
            angle: 0.0,
            pressure: [1.0; 3],
            load: [1.0; 2],
            progress: 1.0,
            ink: None,
            material_work_limit: 32_000_000,
            advanced: advanced::Options::default(),
            full_material: Arc::new(OnceLock::new()),
        }
    }

    /// Sets the minor/major ratio in (0,1] and fixed clockwise radians.
    pub fn nib(mut self, aspect: f64, angle: f64) -> Self {
        self.aspect = aspect;
        self.angle = angle;
        self.advanced.set_fixed_angle(angle);
        self.reset();
        self
    }

    /// Sets start/middle/end nonnegative pressure, each at most four.
    pub fn pressure(mut self, start: f64, middle: f64, end: f64) -> Self {
        self.pressure = [start, middle, end];
        self.advanced.reset_pressure();
        self.reset();
        self
    }

    /// Replaces deterministic local-coordinate ink material.
    pub fn ink(mut self, ink: Ink) -> Self {
        self.ink = Some(ink);
        self.reset();
        self
    }

    /// Replaces the positive material distance-evaluation allowance for this stroke.
    /// Defaults to 32 million. Raising it admits more CPU preparation work, not
    /// a higher-quality sampler or a larger texture. Frame raster budgets remain independent.
    /// Zero is rejected during validation, including hidden or unrevealed strokes.
    pub fn material_work_limit(mut self, work: u64) -> Self {
        self.material_work_limit = work;
        self.reset();
        self
    }

    /// Replaces start/end ink load, each in `[0,1]`.
    pub fn ink_load(mut self, start: f64, end: f64) -> Self {
        self.load = [start, end];
        self.reset();
        self
    }

    /// Reveals `[0,progress]` by the original full path's arc length.
    pub fn write(mut self, progress: f64) -> Self {
        self.progress = progress;
        self
    }

    fn reset(&mut self) {
        self.full_material = Arc::new(OnceLock::new());
    }
}

impl Drawable for BrushStroke {
    fn bounds(&self, context: &DrawContext) -> Rect {
        self.try_bounds(context)
            .unwrap_or(Rect::new(f64::NAN, 0.0, 0.0, 0.0))
    }

    fn try_bounds(&self, _: &DrawContext) -> Result<Rect, ValidationError> {
        advanced::bounds(self)
    }

    fn draw(&self, _: &DrawContext, canvas: &mut Canvas<'_>) -> Result<(), ValidationError> {
        advanced::draw(self, canvas)
    }
}

struct FillSegment {
    start: Point,
    end: Point,
    delta: Vector2,
    square: f64,
}

impl FillSegment {
    fn new(start: Point, end: Point) -> Self {
        let delta = end - start;
        Self {
            start,
            end,
            delta,
            square: delta.x * delta.x + delta.y * delta.y,
        }
    }

    fn squared_distance(&self, point: Point) -> f64 {
        let offset = point - self.start;
        let amount = if self.square == 0.0 {
            0.0
        } else {
            ((offset.x * self.delta.x + offset.y * self.delta.y) / self.square).clamp(0.0, 1.0)
        };
        let nearest = point - (self.start + self.delta * amount);
        nearest.x * nearest.x + nearest.y * nearest.y
    }

    fn crossing(&self, vertical: f64) -> Option<(f64, i32)> {
        if (self.start.y <= vertical && vertical < self.end.y)
            || (self.end.y <= vertical && vertical < self.start.y)
        {
            Some((
                self.start.x
                    + (vertical - self.start.y) * (self.end.x - self.start.x)
                        / (self.end.y - self.start.y),
                if self.end.y > self.start.y { 1 } else { -1 },
            ))
        } else {
            None
        }
    }
}

fn fill_distance(segments: &[FillSegment], point: Point) -> f64 {
    libm::sqrt(segments.iter().fold(f64::INFINITY, |distance, segment| {
        distance.min(segment.squared_distance(point))
    }))
}

/// bounded ink fill. Kurbo flattens the original contours; signed boundary
/// distance and the existing Ink material kernel produce a RGBA texture.
/// The texture includes feathering and is emitted as one rectangle fill.
/// At most 4096 edges, 262144 texels and 32 million edge/texel evaluations are
/// accepted. Local paper coordinates and no fluid simulation, as for BrushStroke.
#[derive(Clone, Debug)]
pub struct InkFill {
    path: Path2D,
    color: Color,
    ink: Ink,
    prepared: Arc<OnceLock<Result<Prepared, ValidationError>>>,
}

impl InkFill {
    /// Captures an immutable contour and deterministic material.
    pub fn new(path: Path2D, color: Color, ink: Ink) -> Self {
        Self {
            path,
            color,
            ink,
            prepared: Arc::new(OnceLock::new()),
        }
    }

    fn prepared(&self) -> Result<&Prepared, ValidationError> {
        self.prepared
            .get_or_init(|| self.prepare())
            .as_ref()
            .map_err(Clone::clone)
    }

    fn prepare(&self) -> Result<Prepared, ValidationError> {
        if self.path.verbs().len() > 512 {
            return Err(invalid("fill", "Ink fill admits at most 512 input verbs"));
        }
        let padding = f64::from(
            self.ink
                .padding()
                .map_err(|error| invalid("ink", &error.to_string()))?,
        ) + 1.0;
        let bounds = self.path.bounding_box().inset(-padding);
        let width = bounds.width().ceil().max(1.0) as u32;
        let height = bounds.height().ceil().max(1.0) as u32;
        if width > 2048 || height > 2048 || u64::from(width) * u64::from(height) > 262144 {
            return Err(invalid("fill", "Ink fill texture exceeds allowance"));
        }
        let mut curve = kurbo::BezPath::new();
        for verb in self.path.verbs() {
            let point = |point: &Point| (point.x, point.y);
            match verb {
                PathVerb::MoveTo(end) => curve.move_to(point(end)),
                PathVerb::LineTo(end) => curve.line_to(point(end)),
                PathVerb::QuadraticTo(control, end) => curve.quad_to(point(control), point(end)),
                PathVerb::CubicTo(first, second, end) => {
                    curve.curve_to(point(first), point(second), point(end))
                }
                PathVerb::Close => curve.close_path(),
            }
        }
        let mut edges = Vec::new();
        let mut exceeded = false;
        let mut append = |first, second| {
            if edges.len() < 4096 {
                edges.push((first, second));
            } else {
                exceeded = true;
            }
        };
        let mut position = Point::ORIGIN;
        let mut start = position;
        let mut open = false;
        kurbo::flatten(
            curve.elements().iter().copied(),
            0.05,
            |element| match element {
                kurbo::PathEl::MoveTo(end) => {
                    if open && position != start {
                        append(position, start);
                    }
                    position = Point::new(end.x, end.y);
                    start = position;
                    open = true;
                }
                kurbo::PathEl::LineTo(end) => {
                    let end = Point::new(end.x, end.y);
                    append(position, end);
                    position = end;
                }
                kurbo::PathEl::ClosePath => {
                    append(position, start);
                    position = start;
                    open = false;
                }
                _ => unreachable!("flatten returns lines"),
            },
        );
        if open && position != start {
            append(position, start);
        }
        if exceeded || edges.len() as u64 * u64::from(width) * u64::from(height) > 32_000_000 {
            return Err(invalid(
                "fill",
                "Ink fill edge/texel work exceeds allowance",
            ));
        }
        let base = self.color.to_rgba_float();
        let segments = edges
            .into_iter()
            .map(|(start, end)| FillSegment::new(start, end))
            .collect::<Vec<_>>();
        let mut crossings = Vec::with_capacity(segments.len());
        let mut pixels = Vec::with_capacity(width as usize * height as usize);
        for row in 0..height {
            let vertical = bounds.top + f64::from(row) + 0.5;
            crossings.clear();
            crossings.extend(
                segments
                    .iter()
                    .filter_map(|segment| segment.crossing(vertical)),
            );
            for column in 0..width {
                let point = Point::new(bounds.left + f64::from(column) + 0.5, vertical);
                let distance = fill_distance(&segments, point);
                let mut winding = 0;
                for &(crossing, direction) in &crossings {
                    if crossing > point.x {
                        winding += direction;
                    }
                }
                let signed = if winding == 0 { distance } else { -distance };
                let material = self.ink.sample((point.x, point.y), (point.x, point.y));
                let alpha = material.fill_edge_coverage(signed, 1.0).clamp(0.0, 1.0);
                pixels.push([base[0], base[1], base[2], base[3] * alpha]);
            }
        }
        let image = ImageAsset::from_rgba_float(width, height, pixels)
            .map_err(|error| invalid("fill", &error.to_string()))?;
        let paint = ImagePattern::from_asset(Arc::new(image))
            .transform(Transform::translate(bounds.left, bounds.top))
            .into();
        Ok(Prepared {
            path: Arc::new(Path2D::from_rect(bounds)?),
            paint,
            bounds,
        })
    }
}

impl Drawable for InkFill {
    fn bounds(&self, context: &DrawContext) -> Rect {
        self.try_bounds(context)
            .unwrap_or(Rect::new(f64::NAN, 0.0, 0.0, 0.0))
    }

    fn try_bounds(&self, _: &DrawContext) -> Result<Rect, ValidationError> {
        Ok(self.prepared()?.bounds)
    }

    fn draw(&self, _: &DrawContext, canvas: &mut Canvas<'_>) -> Result<(), ValidationError> {
        let prepared = self.prepared()?;
        canvas.fill_path(
            prepared.path.clone(),
            Transform::IDENTITY,
            prepared.paint.clone(),
            FillRule::NonZero,
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use motion_render::RenderPrepare;

    fn fill_reference(edges: &[(Point, Point)], point: Point) -> (f64, i32) {
        let mut distance = f64::INFINITY;
        let mut winding = 0;
        for &(first, second) in edges {
            let delta = second - first;
            let offset = point - first;
            let square = delta.x * delta.x + delta.y * delta.y;
            let amount = if square == 0.0 {
                0.0
            } else {
                ((offset.x * delta.x + offset.y * delta.y) / square).clamp(0.0, 1.0)
            };
            let nearest = point - (first + delta * amount);
            distance = distance.min(libm::sqrt(nearest.x * nearest.x + nearest.y * nearest.y));
            if (first.y <= point.y && point.y < second.y)
                || (second.y <= point.y && point.y < first.y)
            {
                let crossing =
                    first.x + (point.y - first.y) * (second.x - first.x) / (second.y - first.y);
                if crossing > point.x {
                    winding += if second.y > first.y { 1 } else { -1 };
                }
            }
        }
        (distance, winding)
    }

    #[test]
    fn fill_queries_match_original_distance_and_winding_bits() {
        let contours = [
            vec![],
            vec![(0.0, 0.0), (0.0, 0.0)],
            vec![
                (0.0, 0.0),
                (20.0, 0.0),
                (20.0, 20.0),
                (0.0, 20.0),
                (0.0, 0.0),
            ],
            vec![
                (0.0, 0.0),
                (20.0, 20.0),
                (0.0, 20.0),
                (20.0, 0.0),
                (0.0, 0.0),
            ],
            vec![(0.0, 0.0), (20.0, 0.0), (0.0, 0.0), (0.0, 20.0), (0.0, 0.0)],
            (0..=96)
                .map(|index| {
                    let angle = index as f64 * std::f64::consts::TAU / 96.0;
                    (libm::cos(angle) * 40.0, libm::sin(angle) * 20.0)
                })
                .collect(),
        ];
        for (scale, translation) in [(1.0, 0.0), (1e-100, 0.0), (1e-300, 0.0), (1.0, 1e12)] {
            for contour in &contours {
                let edges = contour
                    .windows(2)
                    .map(|pair| {
                        let point = |coordinates: (f64, f64)| {
                            Point::new(
                                coordinates.0 * scale + translation,
                                coordinates.1 * scale - translation,
                            )
                        };
                        (point(pair[0]), point(pair[1]))
                    })
                    .collect::<Vec<_>>();
                let segments = edges
                    .iter()
                    .map(|&(start, end)| FillSegment::new(start, end))
                    .collect::<Vec<_>>();
                for row in -2..=22 {
                    let vertical = row as f64 * scale - translation;
                    let crossings = segments
                        .iter()
                        .filter_map(|segment| segment.crossing(vertical))
                        .collect::<Vec<_>>();
                    for column in -2..=22 {
                        let point = Point::new(column as f64 * scale + translation, vertical);
                        let expected = fill_reference(&edges, point);
                        let winding: i32 = crossings
                            .iter()
                            .filter(|(crossing, _)| *crossing > point.x)
                            .map(|(_, direction)| *direction)
                            .sum();
                        assert_eq!(
                            fill_distance(&segments, point).to_bits(),
                            expected.0.to_bits()
                        );
                        assert_eq!(winding, expected.1);
                    }
                }
            }
        }
    }

    #[test]
    fn material_work_override_preserves_defaults_validation_and_retry()
    -> Result<(), ValidationError> {
        let stroke = BrushStroke::new(Path2D::parse_svg("M0 0L16 0")?, 2.0, Color::WHITE)
            .ink(Ink::new(7).dryness(0.2));
        let context = DrawContext::default();
        let limits = motion_core::ValidationLimits::default();
        let expected = context.record(&stroke, limits)?;
        assert_eq!(stroke.material_work_limit, 32_000_000);
        assert_eq!(
            context
                .record(&stroke.clone().material_work_limit(1), limits)
                .unwrap_err()
                .field,
            "material"
        );
        assert_eq!(
            context.record(&stroke.clone().material_work_limit(100_000_000), limits)?,
            expected
        );
        assert!(
            context
                .record(&stroke.clone().material_work_limit(0).write(0.0), limits)
                .is_err()
        );
        assert_eq!(context.record(&stroke, limits)?, expected);
        Ok(())
    }

    #[cfg(feature = "gpu")]
    #[test]
    #[ignore = "requires native GPU; no fonts/window/encoder"]
    fn ink_fill_material_matches_cpu_gpu() -> motion_core::Result {
        let fill = InkFill::new(
            Path2D::parse_svg("M4 4H28V28H4Z M10 10V22H22V10Z")?,
            Color::rgb(20, 80, 65),
            Ink::new(4)
                .dryness(0.2)
                .granulation(0.3)
                .bleed(1.0)
                .paper(7, 0.2, 1.0),
        );
        let limits = motion_core::RasterLimits::default();
        let prepared = motion_core::Node::from(fill)
            .compile(
                motion_core::Size::new(32.0, 32.0),
                motion_core::PixelSize::new(64, 64),
                motion_core::ValidationLimits::default(),
            )?
            .prepare(motion_core::ValidationLimits::default(), limits)?;
        let expected = prepared
            .render_linear(limits)?
            .to_srgb_rgba8(false, limits)?;
        let mut gpu = motion_render::gpu::Gpu::new()?;
        let actual = gpu
            .render(&prepared, motion_render::gpu::Plan::new(&prepared, limits)?)?
            .to_srgb_rgba8(false, limits)?;
        let replay = gpu
            .render(&prepared, motion_render::gpu::Plan::new(&prepared, limits)?)?
            .to_srgb_rgba8(false, limits)?;
        assert_eq!(actual.bytes(), replay.bytes());
        assert!(
            expected
                .bytes()
                .iter()
                .zip(actual.bytes())
                .all(|(expected, actual)| expected.abs_diff(*actual) <= 2)
        );
        Ok(())
    }

    #[test]
    fn ink_preserves_holes_and_rejects_excessive_material() -> Result<(), ValidationError> {
        let fill = InkFill::new(
            Path2D::parse_svg("M4 4H28V28H4Z M10 10V22H22V10Z")?,
            Color::BLACK,
            Ink::new(4).granulation(0.2),
        );
        let frame = motion_core::Node::from(fill)
            .compile(
                motion_core::Size::new(32.0, 32.0),
                motion_core::PixelSize::new(32, 32),
                motion_core::ValidationLimits::default(),
            )?
            .prepare(
                motion_core::ValidationLimits::default(),
                motion_core::RasterLimits::default(),
            )?
            .render_linear(motion_core::RasterLimits::default())?;
        assert_eq!(frame.pixel(16, 16).unwrap()[3], 0.0);
        assert!(frame.pixel(6, 6).unwrap()[3] > 0.5);
        assert_eq!(frame.pixel(0, 0).unwrap()[3], 0.0);
        let oversized = InkFill::new(
            Path2D::from_rect(Rect::new(0.0, 0.0, 2000.0, 2000.0))?,
            Color::BLACK,
            Ink::new(1),
        );
        assert!(
            DrawContext::default()
                .record(&oversized, motion_core::ValidationLimits::default())
                .is_err()
        );
        Ok(())
    }

    #[test]
    fn brush_is_single_fill_bounded_and_material_replays() -> Result<(), ValidationError> {
        let path = Path2D::parse_svg("M4 16Q16 2 28 16")?;
        let brush = BrushStroke::new(path.clone(), 6.0, Color::BLACK)
            .pressure(0.1, 1.0, 0.1)
            .nib(0.4, 0.5)
            .ink(Ink::new(7).dryness(0.3).granulation(0.2));
        let context = DrawContext::default();
        let commands = context.record(&brush, motion_core::ValidationLimits::default())?;
        assert_eq!(commands.len(), 1);
        assert_eq!(
            commands,
            context.record(&brush, motion_core::ValidationLimits::default())?
        );
        assert!(
            context
                .record(
                    &BrushStroke::new(path, 6.0, Color::BLACK).nib(0.0, 0.0),
                    motion_core::ValidationLimits::default()
                )
                .is_err()
        );
        Ok(())
    }

    #[test]
    fn ink_fill_uses_one_bounded_command_and_replays() -> Result<(), ValidationError> {
        let fill = InkFill::new(
            Path2D::parse_svg("M4 4H28V28H4Z M10 10V22H22V10Z")?,
            Color::BLACK,
            Ink::new(4).granulation(0.2).bleed(1.0),
        );
        let context = DrawContext::default();
        let first = context.record(&fill, motion_core::ValidationLimits::default())?;
        assert_eq!(first.len(), 1);
        assert_eq!(
            first,
            context.record(&fill, motion_core::ValidationLimits::default())?
        );
        Ok(())
    }
}
