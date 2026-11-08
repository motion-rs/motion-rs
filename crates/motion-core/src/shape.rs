//! Logical shapes and styled geometry.

use crate::drawing::*;
use crate::{Path2D, Rect, ValidationError};

mod primitives;

pub use primitives::*;

/// Logical geometry independent of paint and node placement.
///
/// Custom geometry must keep path and bounds consistent and deterministic.
/// Direct path conversion requires valid geometry; invalid raw rectangles panic
/// there, while recording their filled nodes reports validation errors instead.
pub trait Shape: Send + Sync {
    /// Reports retained authoring errors before measuring or drawing.
    fn validate(&self) -> Result<(), ValidationError> {
        Ok(())
    }

    /// Returns a checked path snapshot without f32 narrowing.
    fn path(&self) -> Path2D;

    /// Geometric bounds; defaults to the path's cached curve bounds.
    fn bounding_box(&self) -> Rect {
        self.path().bounding_box()
    }

    /// Exact rectangle equivalent, when available, for a rule-independent fast path.
    fn as_rect(&self) -> Option<Rect> {
        None
    }

    /// Adds floating fill paint, using the nonzero rule by default.
    fn fill(self, paint: impl Into<Paint>) -> Styled<Self>
    where
        Self: Sized,
    {
        Styled {
            shape: self,
            fill: Some(paint.into()),
            stroke: None,
            stroke_style: StrokeStyle::default(),
            rule: FillRule::default(),
            trim: None,
            dash: None,
        }
    }

    /// Adds a bounded curve/path stroke without an implicit fill.
    fn stroke(self, width: f64, paint: impl Into<Paint>) -> Styled<Self>
    where
        Self: Sized,
    {
        Styled {
            shape: self,
            fill: None,
            stroke: Some(paint.into()),
            stroke_style: StrokeStyle {
                width,
                ..StrokeStyle::default()
            },
            rule: FillRule::default(),
            trim: None,
            dash: None,
        }
    }
}

impl Shape for Path2D {
    fn validate(&self) -> Result<(), ValidationError> {
        Path2D::validate(self)
    }

    fn path(&self) -> Path2D {
        self.clone()
    }

    fn bounding_box(&self) -> Rect {
        Path2D::bounding_box(self)
    }
}

impl Shape for Rect {
    fn path(&self) -> Path2D {
        Path2D::from_rect(*self).expect("Shape::path requires valid rectangle geometry")
    }

    fn bounding_box(&self) -> Rect {
        *self
    }

    fn as_rect(&self) -> Option<Rect> {
        Some(*self)
    }
}

/// geometry with optional fill/stroke paint and an explicit fill rule.
///
/// Fill is drawn before stroke; Node opacity applies once to their combined
/// result. Bounds conservatively include the stroke, not just its centerline.
#[derive(Clone, Debug)]
pub struct Styled<Geometry> {
    shape: Geometry,
    fill: Option<Paint>,
    stroke: Option<Paint>,
    stroke_style: StrokeStyle,
    rule: FillRule,
    trim: Option<(f64, f64)>,
    dash: Option<(std::sync::Arc<[f64]>, f64)>,
}

impl<Geometry: Shape> Styled<Geometry> {
    /// Clips source geometry by normalized arc distance before dashing.
    /// Bounds remain conservative and stable during growth animation.
    pub fn trim(mut self, start: f64, end: f64) -> Self {
        self.trim = Some((start, end));
        self
    }

    /// Replaces the dash pattern/phase. Invalid or more than 256 entries fail
    /// during drawing, even on hidden or zero-width artwork.
    pub fn dash(mut self, pattern: &[f64], offset: f64) -> Self {
        self.dash = Some((std::sync::Arc::from(pattern), offset));
        self
    }

    /// Replaces the floating fill paint without changing geometry or rule.
    pub fn fill(mut self, paint: impl Into<Paint>) -> Self {
        self.fill = Some(paint.into());
        self
    }

    /// Adds or replaces stroke paint/width, retaining cap and join selection.
    pub fn stroke(mut self, width: f64, paint: impl Into<Paint>) -> Self {
        self.stroke = Some(paint.into());
        self.stroke_style.width = width;
        self
    }

    /// Replaces stroke geometry style without enabling a missing stroke paint.
    pub fn stroke_style(mut self, style: StrokeStyle) -> Self {
        self.stroke_style = style;
        self
    }

    /// Replaces the contour rule; the default is [`FillRule::default`].
    pub fn fill_rule(mut self, rule: FillRule) -> Self {
        self.rule = rule;
        self
    }
}

impl Rect {
    /// Styles this rectangle without declaring a separate layout bound.
    ///
    /// The geometry itself supplies Drawable::bounds.
    pub fn fill(self, paint: impl Into<Paint>) -> Styled<Self> {
        Shape::fill(self, paint)
    }

    /// Styles this rectangle with a outline and no implicit fill.
    pub fn stroke(self, width: f64, paint: impl Into<Paint>) -> Styled<Self> {
        Shape::stroke(self, width, paint)
    }
}

impl<Geometry: Shape> Drawable for Styled<Geometry> {
    fn try_bounds(&self, context: &DrawContext) -> Result<Rect, ValidationError> {
        self.shape.validate()?;
        Ok(self.bounds(context))
    }

    fn bounds(&self, _: &DrawContext) -> Rect {
        let bounds = self.shape.bounding_box();
        if self.stroke.is_some() && validate_rect(bounds, "stroke", "bounds").is_ok() {
            if self.stroke_style.validate("stroke").is_err() {
                return Rect::new(f64::NAN, 0.0, 0.0, 0.0);
            }
            self.stroke_style.bounds(bounds)
        } else {
            bounds
        }
    }

    fn draw(&self, _: &DrawContext, canvas: &mut Canvas<'_>) -> Result<(), ValidationError> {
        self.shape.validate()?;
        if self.trim.is_some() || self.dash.is_some() {
            let mut geometry = self.shape.path();
            if let Some((start, end)) = self.trim {
                geometry = geometry.try_trim(start, end)?;
            }
            if let Some((pattern, offset)) = &self.dash {
                geometry = geometry.dashed(pattern, *offset)?;
            }
            if let Some(paint) = &self.fill {
                canvas.fill(&geometry, paint.clone(), self.rule)?;
            }
            if let Some(paint) = &self.stroke {
                canvas.stroke(&geometry, &self.stroke_style, paint.clone())?;
            }
            return Ok(());
        }
        if let Some(paint) = &self.fill {
            canvas.fill(&self.shape, paint.clone(), self.rule)?;
        }
        if let Some(paint) = &self.stroke {
            canvas.stroke(&self.shape, &self.stroke_style, paint.clone())?;
        }
        Ok(())
    }
}
