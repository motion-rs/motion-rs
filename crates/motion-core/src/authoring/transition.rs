//! Snapshot transitions; all captures use the shared linear fill pipeline.
//!
//! ```
//! use motion_core::prelude::*;
//! use motion_core::{
//!     DisplayList, RasterLimits, ResourceStore, ValidatedDisplayList, MAX_PATH_VERBS,
//!     MAX_STROKE_VERBS, MAX_SVG_PATH_BYTES, sample_offsets,
//! };
//! use motion_core::authoring::Scene as _;
//! let first = (|context: &SceneContext<'_>| Node::from(context.bounds().fill(Color::WHITE)))
//!     .with_duration(1.seconds());
//! let next = (|context: &SceneContext<'_>| Node::from(context.bounds().fill(Color::BLACK)))
//!     .with_duration(1.seconds());
//! let timeline = Timeline::new(Size::new(8.0, 4.0), 30).track(|track| {
//!     track
//!         .push(first)
//!         .transition(CrossFade, 500.milliseconds())
//!         .push(next)
//! }).build()?;
//! assert_eq!(timeline.frame_count(), 45);
//! let cache = SceneCache::default();
//! let context = SceneContext::new(Time::ZERO, timeline.frame_rate(), timeline.size(), &cache);
//! timeline.render(&context).compile(
//!     timeline.size(),
//!     PixelSize::new(8, 4),
//!     ValidationLimits::default(),
//! )?;
//! # Ok::<(), motion_core::Error>(())
//! ```

use crate::{Color, Node, NodeModifiers, Point, Rect, SceneContext, Shape};

/// A stateless composition of two already evaluated clip snapshots.
pub trait Transition: Send + Sync {
    /// Composes finite progress in 0..=1 inside the context design viewport.
    fn compose(
        &self,
        progress: f64,
        outgoing: Node,
        incoming: Node,
        context: &SceneContext<'_>,
    ) -> Node;

    /// continuous-time easing; finite overshoot is clamped by the scheduler.
    fn easing(&self) -> crate::Easing {
        crate::Easing::Linear
    }
}

impl Transition for std::sync::Arc<dyn Transition> {
    fn compose(
        &self,
        progress: f64,
        outgoing: Node,
        incoming: Node,
        context: &SceneContext<'_>,
    ) -> Node {
        self.as_ref().compose(progress, outgoing, incoming, context)
    }

    fn easing(&self) -> crate::Easing {
        self.as_ref().easing()
    }
}

struct Failure(crate::ValidationError);

impl crate::Drawable for Failure {
    fn bounds(&self, _: &crate::DrawContext) -> Result<Rect, crate::ValidationError> {
        Err(self.0.clone())
    }

    fn draw(
        &self,
        _: &crate::DrawContext,
        _: &mut crate::Canvas<'_>,
    ) -> Result<(), crate::ValidationError> {
        Err(self.0.clone())
    }
}

/// Retains a construction failure as a checked node instead of drawing fallback content.
pub fn diagnostic(error: crate::ValidationError) -> Node {
    Node::from(Failure(error))
}

pub(crate) fn failure(field: &str, message: &str) -> Node {
    diagnostic(crate::ValidationError::invalid(
        "transition",
        field,
        message,
    ))
}

fn sum(samples: impl IntoIterator<Item = (Node, f64)>) -> Node {
    Node::weighted_sum(samples).expect("bounded built-in transition weights")
}

fn endpoint(
    progress: f64,
    outgoing: &Node,
    incoming: &Node,
    context: &SceneContext<'_>,
) -> Option<Node> {
    if !progress.is_finite() || !(0.0..=1.0).contains(&progress) {
        return Some(failure(
            "progress",
            "Transition progress must be finite in 0..=1",
        ));
    }
    if progress == 0.0 {
        return Some(
            Node::group([
                outgoing.clone(),
                Node::group([incoming.clone()]).opacity(0.0),
            ])
            .clip_with(context.bounds(), crate::ClipMode::Geometric),
        );
    }
    if progress == 1.0 {
        return Some(
            Node::group([
                Node::group([outgoing.clone()]).opacity(0.0),
                incoming.clone(),
            ])
            .clip_with(context.bounds(), crate::ClipMode::Geometric),
        );
    }
    None
}

fn select(mask: Node, outgoing: Node, incoming: Node, context: &SceneContext<'_>) -> Node {
    sum([
        (
            outgoing.mask(mask.clone(), crate::MaskMode::InvertedAlpha),
            1.0,
        ),
        (incoming.mask(mask, crate::MaskMode::Alpha), 1.0),
    ])
    .clip_with(context.bounds(), crate::ClipMode::Geometric)
}

/// Exact linear-premultiplied interpolation, including transparent snapshots.
#[derive(Clone, Copy, Debug, Default)]
pub struct CrossFade;

impl Transition for CrossFade {
    fn compose(
        &self,
        progress: f64,
        outgoing: Node,
        incoming: Node,
        context: &SceneContext<'_>,
    ) -> Node {
        if let Some(node) = endpoint(progress, &outgoing, &incoming, context) {
            return node;
        }
        sum([(outgoing, 1.0 - progress), (incoming, progress)])
            .clip_with(context.bounds(), crate::ClipMode::Geometric)
    }
}

/// Incoming viewport moves in the selected direction over a stationary outgoing view.
#[derive(Clone, Copy, Debug)]
pub struct Slide {
    /// Incoming travel direction.
    pub direction: Direction,
}

/// Both viewports move in the selected direction with an adjoining boundary.
#[derive(Clone, Copy, Debug)]
pub struct Push {
    /// Viewport travel direction.
    pub direction: Direction,
}

fn displacement(direction: Direction, context: &SceneContext<'_>) -> (f64, f64) {
    match direction {
        Direction::Left => (-context.size().width, 0.0),
        Direction::Right => (context.size().width, 0.0),
        Direction::Up => (0.0, -context.size().height),
        Direction::Down => (0.0, context.size().height),
    }
}

impl Transition for Slide {
    fn compose(
        &self,
        progress: f64,
        outgoing: Node,
        incoming: Node,
        context: &SceneContext<'_>,
    ) -> Node {
        if let Some(node) = endpoint(progress, &outgoing, &incoming, context) {
            return node;
        }
        let (horizontal, vertical) = displacement(self.direction, context);
        let offset = (-horizontal * (1.0 - progress), -vertical * (1.0 - progress));
        let mask = Node::from(context.bounds().fill(Color::WHITE)).translate(offset.0, offset.1);
        let incoming = incoming
            .clip_with(context.bounds(), crate::ClipMode::Geometric)
            .translate(offset.0, offset.1);
        sum([
            (outgoing.mask(mask, crate::MaskMode::InvertedAlpha), 1.0),
            (incoming, 1.0),
        ])
        .clip_with(context.bounds(), crate::ClipMode::Geometric)
    }
}

impl Transition for Push {
    fn compose(
        &self,
        progress: f64,
        outgoing: Node,
        incoming: Node,
        context: &SceneContext<'_>,
    ) -> Node {
        if let Some(node) = endpoint(progress, &outgoing, &incoming, context) {
            return node;
        }
        let (horizontal, vertical) = displacement(self.direction, context);
        sum([
            (
                outgoing
                    .clip_with(context.bounds(), crate::ClipMode::Geometric)
                    .translate(horizontal * progress, vertical * progress),
                1.0,
            ),
            (
                incoming
                    .clip_with(context.bounds(), crate::ClipMode::Geometric)
                    .translate(-horizontal * (1.0 - progress), -vertical * (1.0 - progress)),
                1.0,
            ),
        ])
        .clip_with(context.bounds(), crate::ClipMode::Geometric)
    }
}

/// Projection wipe: radians from +X, with a nonnegative logical-pixel feather.
#[derive(Clone, Copy, Debug, Default)]
pub struct Wipe {
    /// Direction of the advancing edge's normal.
    pub angle: f64,
    /// Linear transition width in design pixels; zero is hard.
    pub feather: f64,
}

impl Transition for Wipe {
    fn compose(
        &self,
        progress: f64,
        outgoing: Node,
        incoming: Node,
        context: &SceneContext<'_>,
    ) -> Node {
        if !self.angle.is_finite() || !self.feather.is_finite() || self.feather < 0.0 {
            return failure("wipe", "Wipe requires finite angle and nonnegative feather");
        }
        if let Some(node) = endpoint(progress, &outgoing, &incoming, context) {
            return node;
        }
        let normal = Point::new(libm::cos(self.angle), libm::sin(self.angle));
        let projections = [
            0.0,
            context.size().width * normal.x,
            context.size().height * normal.y,
            context.size().width * normal.x + context.size().height * normal.y,
        ];
        let minimum = projections.iter().copied().fold(f64::INFINITY, f64::min);
        let maximum = projections
            .iter()
            .copied()
            .fold(f64::NEG_INFINITY, f64::max);
        let position = minimum - self.feather * 0.5 + (maximum - minimum + self.feather) * progress;
        let mask = if self.feather == 0.0 {
            let radius = libm::hypot(context.size().width, context.size().height) * 2.0;
            Node::from(Rect::new(-radius, -radius, position, radius).fill(Color::WHITE))
                .pivot(crate::Pivot::Point(Point::ORIGIN))
                .rotate(self.angle)
        } else {
            let first = Point::new(
                normal.x * (position - self.feather * 0.5),
                normal.y * (position - self.feather * 0.5),
            );
            let last = Point::new(
                normal.x * (position + self.feather * 0.5),
                normal.y * (position + self.feather * 0.5),
            );
            Node::from(
                context.bounds().fill(
                    crate::LinearGradient::new(first, last)
                        .stop(0.0, Color::WHITE)
                        .stop(1.0, Color::WHITE.with_alpha(0.0))
                        .units(crate::GradientUnits::UserSpace)
                        .space(crate::GradientSpace::LinearRgb),
                ),
            )
        };
        select(mask, outgoing, incoming, context)
    }
}

/// Built-in iris reveal geometry centered in the design viewport.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
#[cfg_attr(
    feature = "serde",
    derive(serde::Serialize, serde::Deserialize),
    serde(rename_all = "snake_case")
)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
pub enum IrisShape {
    /// Circle reaches every viewport corner at progress one.
    #[default]
    Circle,
    /// Rectangle grows proportionally in both dimensions.
    Rectangle,
}

/// Centered growing-shape selection of the incoming snapshot.
#[derive(Clone, Copy, Debug, Default)]
pub struct Iris {
    /// Reveal geometry.
    pub shape: IrisShape,
}

impl Transition for Iris {
    fn compose(
        &self,
        progress: f64,
        outgoing: Node,
        incoming: Node,
        context: &SceneContext<'_>,
    ) -> Node {
        if let Some(node) = endpoint(progress, &outgoing, &incoming, context) {
            return node;
        }
        let center = context.center();
        let mask = match self.shape {
            IrisShape::Circle => {
                match crate::Circle::new(center, libm::hypot(center.x, center.y) * progress) {
                    Ok(circle) => Node::from(circle.fill(Color::WHITE)),
                    Err(error) => return diagnostic(error),
                }
            }
            IrisShape::Rectangle => Node::from(
                Rect::new(
                    center.x * (1.0 - progress),
                    center.y * (1.0 - progress),
                    center.x * (1.0 + progress),
                    center.y * (1.0 + progress),
                )
                .fill(Color::WHITE),
            ),
        };
        select(mask, outgoing, incoming, context)
    }
}

/// Twelve spatial zoom samples per snapshot; no Gaussian substitute or time resampling.
#[derive(Clone, Copy, Debug, Default)]
pub struct ZoomBlur;

impl Transition for ZoomBlur {
    fn compose(
        &self,
        progress: f64,
        outgoing: Node,
        incoming: Node,
        context: &SceneContext<'_>,
    ) -> Node {
        if let Some(node) = endpoint(progress, &outgoing, &incoming, context) {
            return node;
        }
        let blur = |node: Node, base: f64| {
            let samples = (0..12).map(|index| {
                node.clone()
                    .clip_with(context.bounds(), crate::ClipMode::Geometric)
                    .pivot(crate::Pivot::Point(context.center()))
                    .scale(
                        base + libm::sin(std::f64::consts::PI * progress)
                            * 0.2
                            * (f64::from(index) + 0.5)
                            / 12.0,
                    )
            });
            Node::accumulate(samples).expect("fixed zoom samples")
        };
        CrossFade.compose(
            progress,
            blur(outgoing, 1.0 + progress * 0.25),
            blur(incoming, 0.75 + progress * 0.25),
            context,
        )
    }
}

/// A movement vector in layer-local coordinates, not the edge of entry.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[cfg_attr(
    feature = "serde",
    derive(serde::Serialize, serde::Deserialize),
    serde(rename_all = "snake_case")
)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
pub enum Direction {
    /// Move/reveal toward decreasing X; incoming push content enters from the right.
    Left,
    /// Move/reveal toward increasing X; incoming push content enters from the left.
    Right,
    /// Move/reveal toward decreasing Y; incoming push content enters from below.
    Up,
    /// Move/reveal toward increasing Y; incoming push content enters from above.
    Down,
}
