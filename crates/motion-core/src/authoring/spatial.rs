//! Logical placement and double-precision bounds.

use crate::{Pivot, Point, Rect, Transform, ValidationError, Vector2};

/// An edge in the downward-positive parent coordinate system.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Side {
    /// The left edge.
    Left,
    /// The right edge.
    Right,
    /// The upper edge.
    Top,
    /// The lower edge.
    Bottom,
}

impl Side {
    pub(crate) fn adjacent(self, subject: Rect, reference: Rect, gap: f64) -> Vector2 {
        match self {
            Self::Left => Vector2::new(
                reference.left - subject.right - gap,
                reference.center().y - subject.center().y,
            ),
            Self::Right => Vector2::new(
                reference.right - subject.left + gap,
                reference.center().y - subject.center().y,
            ),
            Self::Top => Vector2::new(
                reference.center().x - subject.center().x,
                reference.top - subject.bottom - gap,
            ),
            Self::Bottom => Vector2::new(
                reference.center().x - subject.center().x,
                reference.bottom - subject.top + gap,
            ),
        }
    }

    pub(crate) fn aligned(self, subject: Rect, reference: Rect) -> Vector2 {
        match self {
            Self::Left => Vector2::new(reference.left - subject.left, 0.0),
            Self::Right => Vector2::new(reference.right - subject.right, 0.0),
            Self::Top => Vector2::new(0.0, reference.top - subject.top),
            Self::Bottom => Vector2::new(0.0, reference.bottom - subject.bottom),
        }
    }
}

#[derive(Clone, Debug)]
pub(super) struct Relative {
    pub(super) node: super::Node,
    pub(super) reference: super::Node,
    side: Side,
    gap: Option<f64>,
}

pub(super) fn relative(
    node: super::Node,
    reference: super::Node,
    side: Side,
    gap: Option<f64>,
) -> super::Node {
    super::Node::new(super::node::NodeKind::Relative(Relative {
        node,
        reference,
        side,
        gap,
    }))
}

impl Relative {
    pub(super) fn translation(
        &self,
        subject: Rect,
        reference: Rect,
    ) -> Result<Transform, ValidationError> {
        if self.gap.is_some_and(|gap| !gap.is_finite()) {
            return Err(ValidationError::invalid(
                "layout",
                "gap",
                "Relative layout gap must be finite",
            ));
        }
        let offset = match self.gap {
            Some(gap) => self.side.adjacent(subject, reference, gap),
            None => self.side.aligned(subject, reference),
        };
        if !offset.x.is_finite() || !offset.y.is_finite() {
            return Err(ValidationError::invalid(
                "layout",
                "translation",
                "Relative layout offset must remain finite",
            ));
        }
        Ok(Transform::translate(offset.x, offset.y))
    }
}

#[derive(Clone, Copy, Debug)]
pub(super) enum Bounds {
    Empty,
    Known(Rect),
}

impl Bounds {
    pub fn declared(rect: Rect, path: &str) -> Result<Self, ValidationError> {
        if !rect.is_finite() || rect.right < rect.left || rect.bottom < rect.top {
            return Err(ValidationError::invalid(
                path,
                "layout_bounds",
                "Require finite, noninverted declared layout bounds",
            ));
        }
        Ok(Self::Known(rect))
    }

    pub fn rect(self, _path: &str) -> Result<Rect, ValidationError> {
        match self {
            Self::Empty => Ok(Rect::default()),
            Self::Known(rect) => Ok(rect),
        }
    }

    pub fn union(self, other: Self) -> Self {
        match (self, other) {
            (Self::Empty, bounds) | (bounds, Self::Empty) => bounds,
            (Self::Known(first), Self::Known(second)) => Self::Known(Rect::new(
                first.left.min(second.left),
                first.top.min(second.top),
                first.right.max(second.right),
                first.bottom.max(second.bottom),
            )),
        }
    }

    pub fn transformed(self, transform: Transform, path: &str) -> Result<Self, ValidationError> {
        let Self::Known(rect) = self else {
            return Ok(self);
        };
        let corners = [
            Point::new(rect.left, rect.top),
            Point::new(rect.right, rect.top),
            Point::new(rect.right, rect.bottom),
            Point::new(rect.left, rect.bottom),
        ]
        .map(|point| transform.apply(point));
        if corners
            .iter()
            .any(|point| !point.x.is_finite() || !point.y.is_finite())
        {
            return Err(ValidationError::invalid(
                path,
                "transform",
                "Transformed layout bounds must remain finite",
            ));
        }
        Ok(Self::Known(Rect::new(
            corners
                .iter()
                .map(|point| point.x)
                .fold(f64::INFINITY, f64::min),
            corners
                .iter()
                .map(|point| point.y)
                .fold(f64::INFINITY, f64::min),
            corners
                .iter()
                .map(|point| point.x)
                .fold(f64::NEG_INFINITY, f64::max),
            corners
                .iter()
                .map(|point| point.y)
                .fold(f64::NEG_INFINITY, f64::max),
        )))
    }
}

#[derive(Clone, Copy, Debug)]
pub(super) struct Placement {
    pub affine: Transform,
    pub position: Option<Point>,
    pub translation: Vector2,
    pub rotation: f64,
    pub scale: Vector2,
    pub pivot: Pivot,
}

impl Default for Placement {
    fn default() -> Self {
        Self {
            affine: Transform::IDENTITY,
            position: None,
            translation: Vector2::ZERO,
            rotation: 0.0,
            scale: Vector2::new(1.0, 1.0),
            pivot: Pivot::Center,
        }
    }
}

pub(super) struct Resolved {
    pub transform: Transform,
}

impl Placement {
    pub fn resolve(self, bounds: Bounds, path: &str) -> Result<Resolved, ValidationError> {
        if !self.affine.is_finite() {
            return Err(ValidationError::invalid(
                path,
                "transform",
                "Local affine matrix must be finite",
            ));
        }
        let placed_bounds = bounds.transformed(self.affine, path)?;
        let finite = [
            self.translation.x,
            self.translation.y,
            self.rotation,
            self.scale.x,
            self.scale.y,
        ]
        .iter()
        .all(|value| value.is_finite())
            && self
                .position
                .is_none_or(|position| position.x.is_finite() && position.y.is_finite())
            && match self.pivot {
                Pivot::Relative(horizontal, vertical) => {
                    horizontal.is_finite() && vertical.is_finite()
                }
                Pivot::Point(point) => point.x.is_finite() && point.y.is_finite(),
                _ => true,
            };
        if !finite {
            return Err(ValidationError::invalid(
                path,
                "transform",
                "Placement parameters must be finite",
            ));
        }
        let needs_pivot =
            self.position.is_some() || self.rotation != 0.0 || self.scale != Vector2::new(1.0, 1.0);
        let pivot = if !needs_pivot {
            Point::ORIGIN
        } else if let Pivot::Point(point) = self.pivot {
            point
        } else {
            placed_bounds.rect(path)?.anchor(self.pivot)
        };
        let offset = self
            .position
            .map_or(Vector2::ZERO, |position| position - pivot)
            + self.translation;
        let transform = if self.rotation == 0.0 && self.scale == Vector2::new(1.0, 1.0) {
            Transform::translate(offset.x, offset.y)
        } else {
            Transform::translate(-pivot.x, -pivot.y)
                .then(Transform::scale_non_uniform(self.scale.x, self.scale.y))
                .then(Transform::rotate(self.rotation))
                .then(Transform::translate(pivot.x + offset.x, pivot.y + offset.y))
        };
        let transform = self.affine.then(transform);
        if !transform.is_finite() {
            return Err(ValidationError::invalid(
                path,
                "transform",
                "Resolved placement must remain finite",
            ));
        }
        Ok(Resolved { transform })
    }
}
