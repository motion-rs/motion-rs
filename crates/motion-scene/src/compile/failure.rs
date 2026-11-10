//! A node that reports a deferred content failure when the scene is prepared.

use motion_core::{Canvas, DrawContext, Drawable, Node, Rect, ValidationError};

/// Builds a node whose bounds and drawing fail with `error`.
///
/// Content such as simulations and charts can only fail once a frame's context
/// exists; returning this node surfaces the diagnostic through the normal
/// validation and render error path instead of panicking.
pub(crate) fn failure(error: ValidationError) -> Node {
    Node::from(Failure(error))
}

struct Failure(ValidationError);

impl Drawable for Failure {
    fn bounds(&self, _: &DrawContext) -> Result<Rect, ValidationError> {
        Err(self.0.clone())
    }

    fn draw(&self, _: &DrawContext, _: &mut Canvas<'_>) -> Result<(), ValidationError> {
        Err(self.0.clone())
    }
}
