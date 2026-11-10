//! Deterministic label placement and bounded Cartesian line charts.

use crate::Scale;

use motion_core::{Node, Paint, Point, Rect, Size, StrokeStyle, ValidationError};

/// A measured label anchored to a local point, with stable priority ordering.
#[derive(Clone, Copy, Debug)]
pub struct Label {
    anchor: Point,
    size: Size,
    priority: i32,
}

impl Label {
    /// Uses measured dimensions without narrowing paragraph glyph bounds.
    pub fn new(anchor: Point, size: Size) -> Self {
        Self {
            anchor,
            size,
            priority: 0,
        }
    }

    /// Higher priorities are placed first; equal priorities preserve input order.
    pub fn priority(mut self, priority: i32) -> Self {
        self.priority = priority;
        self
    }
}

/// Eight candidate positions per label; unplaceable labels are explicitly omitted.
/// Placement is greedy: accepted labels are never moved to make room for later ones.
/// Stable ordering does not guarantee a globally optimal or temporally smooth layout.
///
/// ```
/// use motion_content::prelude::*;
/// use motion_core::prelude::*;
/// use motion_core::{
///     DisplayList, RasterLimits, ResourcePool, ValidatedDisplayList, MAX_PATH_VERBS,
///     MAX_STROKE_VERBS, MAX_SVG_PATH_BYTES, sample_offsets,
/// };
/// use motion_render::{Composition, RenderPrepare, RenderOptions, Renderer, render_still};
/// use motion_content::diagram::*;
/// let labels = [Label::new(Point::new(50.0, 50.0), Size::new(40.0, 30.0))];
/// let positions = LabelLayout::new(Rect::new(0.0, 0.0, 100.0, 100.0))
///     .gap(2.0)
///     .place(&labels)?;
/// assert_eq!(positions, [Some(Rect::new(52.0, 18.0, 92.0, 48.0))]);
/// # Ok::<(), motion_core::ValidationError>(())
/// ```
///
#[derive(Clone, Debug)]
pub struct LabelLayout {
    bounds: Rect,
    gap: f64,
    obstacles: Vec<Rect>,
}

impl LabelLayout {
    /// Creates a placement viewport with a four-pixel separation.
    pub fn new(bounds: Rect) -> Self {
        Self {
            bounds,
            gap: 4.0,
            obstacles: Vec::new(),
        }
    }

    /// Replaces finite spacing in `0..=1024` pixels.
    pub fn gap(mut self, pixels: f64) -> Self {
        self.gap = pixels;
        self
    }

    /// Adds fixed artwork to avoid; at most 256 obstacles are accepted by `place`.
    pub fn obstacle(mut self, bounds: Rect) -> Self {
        self.obstacles.push(bounds);
        self
    }

    /// Returns positions in input order. `None` means no collision-free candidate fits.
    /// Placement is stateless and bounded to 256 labels and 256 obstacles.
    /// Invalid geometry, nonpositive label/viewport dimensions, invalid spacing or
    /// exceeded counts return [`std::io::ErrorKind::InvalidInput`], not omitted labels.
    pub fn place(&self, labels: &[Label]) -> Result<Vec<Option<Rect>>, ValidationError> {
        if !valid_label_rect(self.bounds)
            || self.bounds.is_empty()
            || !self.gap.is_finite()
            || !(0.0..=1024.0).contains(&self.gap)
            || labels.len() > 256
            || self.obstacles.len() > 256
            || self.obstacles.iter().any(|rect| !valid_label_rect(*rect))
            || labels.iter().any(|label| {
                ![
                    label.anchor.x,
                    label.anchor.y,
                    label.size.width,
                    label.size.height,
                ]
                .iter()
                .all(|value| value.is_finite())
                    || label.size.width <= 0.0
                    || label.size.height <= 0.0
            })
        {
            return Err(invalid("Invalid label geometry, gap or placement budget"));
        }
        let mut occupied = self.obstacles.clone();
        let mut positions = vec![None; labels.len()];
        let mut order: Vec<_> = (0..labels.len()).collect();
        order.sort_by_key(|index| (std::cmp::Reverse(labels[*index].priority), *index));
        for index in order {
            let label = labels[index];
            let (left, top) = (label.anchor.x, label.anchor.y);
            let (width, height) = (label.size.width, label.size.height);
            let gap = self.gap;
            let candidates = [
                (left + gap, top - gap - height),
                (left - gap - width, top - gap - height),
                (left + gap, top + gap),
                (left - gap - width, top + gap),
                (left - width * 0.5, top - gap - height),
                (left - width * 0.5, top + gap),
                (left + gap, top - height * 0.5),
                (left - gap - width, top - height * 0.5),
            ];
            for (left, top) in candidates {
                let rect = Rect::from_origin_size(Point::new(left, top), label.size);
                let padded = rect.inset(-gap);
                if !valid_label_rect(padded) || rect.is_empty() {
                    return Err(invalid("Label candidate arithmetic is not representable"));
                }
                if rect.left >= self.bounds.left
                    && rect.top >= self.bounds.top
                    && rect.right <= self.bounds.right
                    && rect.bottom <= self.bounds.bottom
                    && occupied
                        .iter()
                        .all(|other| padded.intersect(*other).is_empty())
                {
                    positions[index] = Some(rect);
                    occupied.push(rect);
                    break;
                }
            }
        }
        Ok(positions)
    }
}

fn valid_label_rect(rect: Rect) -> bool {
    rect.is_finite()
        && rect.width().is_finite()
        && rect.height().is_finite()
        && rect.width() >= 0.0
        && rect.height() >= 0.0
}

fn inside(rect: Rect, bounds: Rect) -> bool {
    rect.left >= bounds.left
        && rect.top >= bounds.top
        && rect.right <= bounds.right
        && rect.bottom <= bounds.bottom
}

fn overlaps(first: Rect, second: Rect) -> bool {
    !first.intersect(second).is_empty()
}

fn invalid(message: &str) -> ValidationError {
    ValidationError::invalid("chart", "input", message)
}

fn tick_label_bounds(bounds: Rect, coordinate: f64, size: Size, vertical: bool) -> Rect {
    let (left, top) = if vertical {
        (
            bounds.left - 10.0 - size.width,
            coordinate - size.height * 0.5,
        )
    } else {
        (coordinate - size.width * 0.5, bounds.bottom + 10.0)
    };
    Rect::from_origin_size(Point::new(left, top), size)
}

#[derive(Clone, Debug)]
struct Series {
    points: Vec<Option<(f64, f64)>>,
    paint: Paint,
    stroke: StrokeStyle,
}

/// Multi-series Cartesian curves using caller-supplied linear/log scales.
/// The plot rectangle excludes tick labels. Their placement viewport extends
/// 96 pixels to either side, 40 above and 72 below the plot; reserve surrounding space.
/// Curves/grid are clipped to the plot, while labels remain outside that clip.
#[derive(Clone, Debug)]
pub struct LineChart {
    bounds: Rect,
    horizontal: Scale,
    vertical: Scale,
    series: Vec<Series>,
    ticks: usize,
}

impl LineChart {
    /// Builds curves/grid and project-font measured tick labels without
    /// system-font discovery. Curves use paints and stroke styles.
    pub fn node(
        &self,
        context: &motion_core::SceneContext<'_>,
    ) -> motion_core::Result<motion_core::Node> {
        use motion_core::{NodeModifiers, Shape};
        let fonts = context
            .assets()
            .fonts()
            .and_then(|environment| environment.downcast_ref::<crate::FontLibrary>())
            .ok_or_else(|| invalid("charts require explicit project fonts"))?;
        let bounds = self.bounds;
        let mut plot = Vec::new();
        let mut labels = Vec::new();
        for (vertical, scale) in [(false, self.horizontal), (true, self.vertical)] {
            for value in scale.nice_ticks(self.ticks)? {
                let coordinate = scale.map(value)?;
                let mut path = motion_core::Path2D::new();
                if vertical {
                    path.try_move_to(motion_core::Point::new(bounds.left, coordinate))?;
                    path.try_line_to(motion_core::Point::new(bounds.right, coordinate))?;
                } else {
                    path.try_move_to(motion_core::Point::new(coordinate, bounds.top))?;
                    path.try_line_to(motion_core::Point::new(coordinate, bounds.bottom))?;
                }
                plot.push(motion_core::Node::from(
                    path.stroke(1.0, motion_core::Color::rgb(215, 222, 222)),
                ));
                let value = if value != 0.0 && (value.abs() >= 100000.0 || value.abs() < 0.001) {
                    format!("{value:.1e}")
                } else {
                    format!("{value:.3}")
                        .trim_end_matches('0')
                        .trim_end_matches('.')
                        .to_owned()
                };
                let text = crate::Text::new(value)
                    .size(14.0)
                    .fill(motion_core::Color::rgb(35, 52, 48));
                let measured = text.layout(fonts)?.bounds();
                let size = measured.size();
                labels.push((
                    tick_label_bounds(self.bounds, coordinate, size, vertical),
                    text,
                ));
            }
        }
        for series in &self.series {
            let stroke = series.stroke;
            let mut path = motion_core::Path2D::new();
            let mut drawing = false;
            for point in &series.points {
                if let Some((horizontal, vertical)) = point {
                    let point = motion_core::Point::new(
                        self.horizontal.map(*horizontal)?,
                        self.vertical.map(*vertical)?,
                    );
                    if drawing {
                        path.try_line_to(point)?;
                    } else {
                        path.try_move_to(point)?;
                    }
                    drawing = true;
                } else {
                    drawing = false;
                }
            }
            plot.push(Node::from(
                path.stroke(stroke.width, series.paint.clone())
                    .stroke_style(stroke),
            ));
        }
        let mut axis = motion_core::Path2D::new();
        axis.try_move_to(motion_core::Point::new(bounds.left, bounds.top))?;
        axis.try_line_to(motion_core::Point::new(bounds.left, bounds.bottom))?;
        axis.try_line_to(motion_core::Point::new(bounds.right, bounds.bottom))?;
        plot.push(motion_core::Node::from(
            axis.stroke(1.0, motion_core::Color::BLACK),
        ));
        let viewport = Rect::new(
            bounds.left - 96.0,
            bounds.top - 40.0,
            bounds.right + 96.0,
            bounds.bottom + 72.0,
        );
        let mut occupied = Vec::new();
        let mut nodes = vec![
            motion_core::Node::group(plot).clip_with(bounds, motion_core::ClipMode::Geometric),
        ];
        for (position, text) in labels {
            let padded = position.inset(-2.0);
            if inside(padded, viewport) && occupied.iter().all(|other| !overlaps(padded, *other)) {
                occupied.push(padded);
                nodes.push(motion_core::Node::from(text).translate(position.left, position.top));
            }
        }
        Ok(motion_core::Node::group(nodes))
    }

    /// Requires each scale range to match its plot edges, in either direction.
    pub fn new(bounds: Rect, horizontal: Scale, vertical: Scale) -> Result<Self, ValidationError> {
        let horizontal_range = horizontal.range();
        let vertical_range = vertical.range();
        if !valid_label_rect(bounds)
            || bounds.is_empty()
            || bounds.width() > 65536.0
            || bounds.height() > 65536.0
            || horizontal_range.0.min(horizontal_range.1) != bounds.left
            || horizontal_range.0.max(horizontal_range.1) != bounds.right
            || vertical_range.0.min(vertical_range.1) != bounds.top
            || vertical_range.0.max(vertical_range.1) != bounds.bottom
        {
            return Err(invalid(
                "Chart scales must span the finite positive plot bounds",
            ));
        }
        Ok(Self {
            bounds,
            horizontal,
            vertical,
            series: Vec::new(),
            ticks: 6,
        })
    }

    /// Replaces the maximum number of nice ticks per axis, in `2..=32`.
    pub fn ticks(mut self, count: usize) -> Result<Self, ValidationError> {
        if !(2..=32).contains(&count) {
            return Err(invalid("Chart ticks require 2..=32"));
        }
        self.ticks = count;
        Ok(self)
    }

    /// Adds a curve; all points must be finite and valid in both scales.
    pub fn series(
        self,
        points: impl IntoIterator<Item = (f64, f64)>,
        paint: impl Into<Paint>,
        stroke: StrokeStyle,
    ) -> Result<Self, ValidationError> {
        self.gapped_series(points.into_iter().map(Some), paint, stroke)
    }

    /// Adds a curve where `None` breaks the path rather than connecting missing data.
    /// At most 32 series and 65536 total entries (including gaps) are retained.
    pub fn gapped_series(
        mut self,
        points: impl IntoIterator<Item = Option<(f64, f64)>>,
        paint: impl Into<Paint>,
        stroke: StrokeStyle,
    ) -> Result<Self, ValidationError> {
        if self.series.len() == 32 {
            return Err(invalid("Chart exceeds 32 series"));
        }
        stroke
            .validate("chart.series.stroke")
            .map_err(|error| invalid(&error.to_string()))?;
        let paint = paint.into();
        paint
            .validate()
            .map_err(|error| invalid(&error.to_string()))?;
        let remaining = 65536
            - self
                .series
                .iter()
                .map(|series| series.points.len())
                .sum::<usize>();
        let points: Vec<_> = points.into_iter().take(remaining + 1).collect();
        if points.len() > remaining {
            return Err(invalid("Chart exceeds 65536 data entries"));
        }
        for point in points.iter().flatten() {
            self.horizontal.map(point.0)?;
            self.vertical.map(point.1)?;
        }
        self.series.push(Series {
            points,
            paint,
            stroke,
        });
        Ok(self)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::FontLibrary;
    use motion_core::{Assets, PixelSize, SceneContext, Time, Timeline};
    use motion_core::{Color, LineCap, LineJoin};
    use motion_core::{Scene, TimeLiteral};
    use motion_render::{RenderOptions, render_still};

    fn stroke() -> StrokeStyle {
        StrokeStyle {
            width: 2.0,
            cap: LineCap::Round,
            join: LineJoin::Round,
        }
    }

    #[test]
    fn chart_tick_labels_keep_axis_alignment() {
        let plot = Rect::new(100.0, 40.0, 500.0, 240.0);
        let size = Size::new(17.0, 18.0);
        for coordinate in [100.0, 300.0, 500.0] {
            let label = tick_label_bounds(plot, coordinate, size, false);
            assert_eq!(label.left + label.width() * 0.5, coordinate);
            assert_eq!(label.top, 250.0);
            assert!(!overlaps(label, plot));
        }
        for coordinate in [40.0, 140.0, 240.0] {
            let label = tick_label_bounds(plot, coordinate, size, true);
            assert_eq!(label.top + label.height() * 0.5, coordinate);
            assert_eq!(label.right, 90.0);
            assert!(!overlaps(label, plot));
        }
        let size = Size::new(9.0, 20.0);
        let horizontal = tick_label_bounds(plot, plot.left, size, false);
        let vertical = tick_label_bounds(plot, plot.bottom, size, true);
        let padded = |rect: Rect| rect.inset(-2.0);
        assert!(!overlaps(padded(horizontal), padded(vertical)));
    }

    #[test]
    fn labels_are_stable_bounded_nonoverlapping_and_priority_ordered() -> Result<(), ValidationError>
    {
        let layout = LabelLayout::new(Rect::new(0.0, 0.0, 100.0, 100.0)).gap(2.0);
        let labels = vec![Label::new(Point::new(50.0, 50.0), Size::new(40.0, 30.0)); 12];
        let positions = layout.place(&labels)?;
        assert_eq!(positions, layout.place(&labels)?);
        assert!(positions.iter().any(Option::is_none));
        let placed: Vec<_> = positions.into_iter().flatten().collect();
        for (index, rect) in placed.iter().enumerate() {
            assert!(
                rect.left >= 0.0 && rect.top >= 0.0 && rect.right <= 100.0 && rect.bottom <= 100.0
            );
            assert!(
                placed[index + 1..]
                    .iter()
                    .all(|other| rect.intersect(*other).is_empty())
            );
        }
        let labels = [
            Label::new(Point::new(50.0, 50.0), Size::new(40.0, 30.0)),
            Label::new(Point::new(50.0, 50.0), Size::new(40.0, 30.0)).priority(10),
        ];
        let positions = layout.place(&labels)?;
        assert_eq!(positions[1], Some(Rect::new(52.0, 18.0, 92.0, 48.0)));
        assert!(
            LabelLayout::new(layout.bounds)
                .gap(f64::NAN)
                .place(&labels)
                .is_err()
        );
        Ok(())
    }

    #[test]
    fn labels_preserve_sub_f32_coordinates_and_reject_invalid_geometry()
    -> Result<(), ValidationError> {
        let layout =
            LabelLayout::new(Rect::new(100_000_000.0, 0.0, 100_000_100.0, 100.0)).gap(0.25);
        let labels = [Label::new(
            Point::new(100_000_000.125, 50.0),
            Size::new(1.0, 2.0),
        )];
        let expected = Some(Rect::new(100_000_000.375, 47.75, 100_000_001.375, 49.75));
        assert_eq!(layout.place(&labels)?, [expected]);
        assert_eq!(layout.place(&labels)?, [expected]);
        assert!(
            LabelLayout::new(Rect::new(1.0, 0.0, 0.0, 1.0))
                .place(&labels)
                .is_err()
        );
        assert!(
            layout
                .clone()
                .obstacle(Rect::new(0.0, 0.0, f64::INFINITY, 1.0))
                .place(&labels)
                .is_err()
        );
        assert!(
            layout
                .place(&[Label::new(Point::ORIGIN, Size::new(f64::NAN, 1.0))])
                .is_err()
        );
        assert!(
            layout
                .place(&[Label::new(Point::new(f64::MAX, 1.0), Size::new(1.0, 1.0))])
                .is_err()
        );
        assert_eq!(
            layout.place(&[Label::new(Point::new(1.0, 1.0), Size::new(f64::MAX, 1.0))])?,
            [None]
        );
        Ok(())
    }

    #[test]
    fn chart_rejects_bad_scales_values_and_unbounded_input() -> Result<(), ValidationError> {
        let bounds = Rect::new(100.0, 40.0, 500.0, 240.0);
        let chart = LineChart::new(
            bounds,
            Scale::log10((1.0, 100.0), (100.0, 500.0))?,
            Scale::linear((0.0, 10.0), (240.0, 40.0))?,
        )?;
        let stroke = stroke();
        let paint = Color::from_hex(0x118677);
        assert!(chart.clone().series([(0.0, 1.0)], paint, stroke).is_err());
        assert!(
            chart
                .clone()
                .series([(1.0, f64::NAN)], paint, stroke)
                .is_err()
        );
        assert!(
            chart
                .clone()
                .series(std::iter::repeat((1.0, 1.0)), paint, stroke)
                .is_err()
        );
        assert!(chart.clone().ticks(33).is_err());
        assert!(
            chart
                .gapped_series([Some((1.0, 0.0)), None, Some((100.0, 10.0))], paint, stroke)
                .is_ok()
        );
        Ok(())
    }

    #[test]
    #[ignore = "requires explicit MOTION_TEST_FONT; no system discovery"]
    fn chart_builds_a_valid_scene_with_gaps_and_logarithmic_axes() -> motion_core::Result {
        let chart = LineChart::new(
            Rect::new(100.0, 50.0, 500.0, 250.0),
            Scale::log10((1.0, 100.0), (100.0, 500.0))?,
            Scale::linear((0.0, 10.0), (250.0, 50.0))?,
        )?
        .gapped_series(
            [
                Some((1.0, 1.0)),
                Some((10.0, 5.0)),
                None,
                Some((100.0, 9.0)),
            ],
            Color::from_hex(0x118677),
            stroke(),
        )?;
        let font =
            std::env::var_os("MOTION_TEST_FONT").ok_or_else(|| invalid("Set MOTION_TEST_FONT"))?;
        let mut fonts = FontLibrary::new();
        fonts.load_file(font)?;
        let resources = Assets::new(".")?.with_fonts(fonts);
        let scene = (move |context: &SceneContext<'_>| chart.node(context).expect("valid chart"))
            .with_duration(1.seconds());
        let timeline = Timeline::try_new(Size::new(640.0, 360.0), 30)?
            .try_track(|track| track.try_push(scene))?
            .build()?;
        let options = RenderOptions {
            size: Some(PixelSize::new(640, 360)),
            resources: Some(resources),
            ..RenderOptions::default()
        };
        let first = render_still(&timeline, Time::ZERO, &options)?;
        let replay = render_still(&timeline, Time::ZERO, &options)?;
        assert_eq!(first.bytes(), replay.bytes());
        assert!(
            first
                .bytes()
                .as_chunks::<4>()
                .0
                .iter()
                .filter(|pixel| pixel[3] != 0)
                .count()
                > 100
        );
        Ok(())
    }
}
