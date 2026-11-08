//! Logical drawing contracts and bounded rectangle/path fill commands.

mod blend;
mod canvas;
mod clip;
mod command;
mod context;
pub mod paint;
mod stroke;

pub use crate::shape::{Shape, Styled};
pub use blend::{BlendMode, BlendSpace, FillRule, MaskMode};
pub use canvas::Canvas;
pub use clip::{ClipGeometry, ClipMode, PathClip, RectClip, ShapeClip};
pub use command::DrawCommand;
pub use context::{DrawContext, Drawable, RenderQuality};

pub use paint::{
    GradientExtend, GradientSpace, GradientUnits, ImagePattern, LinearGradient, Paint,
    RadialGradient, SweepGradient,
};
pub use stroke::{LineCap, LineJoin, MAX_STROKE_VERBS, StrokeStyle};

use std::sync::Arc;

use crate::validation::ValidationState;
use crate::{Path2D, Rect, Transform, ValidationError, ValidationLimits};

/// Rejects nonfinite/inverted corners or nonfinite spans; zero-area rectangles are valid.
pub fn validate_rect(bounds: Rect, path: &str, field: &str) -> Result<(), ValidationError> {
    if !bounds.is_finite()
        || bounds.right < bounds.left
        || bounds.bottom < bounds.top
        || !bounds.width().is_finite()
        || !bounds.height().is_finite()
    {
        return Err(ValidationError::invalid(
            path,
            field,
            "Require finite, noninverted rectangle bounds and finite spans",
        ));
    }
    Ok(())
}

#[cfg(test)]
mod tests;
