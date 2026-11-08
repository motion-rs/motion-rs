//! Device drawing command records.

use super::*;

/// A fill or isolated-layer boundary; more primitives await their contracts.
#[derive(Clone, Debug, PartialEq)]
#[non_exhaustive]
pub enum DrawCommand {
    /// Particle instances, independent of path strips.
    ParticleInstances {
        /// Immutable validated instance/style/program payload.
        batch: Arc<dyn crate::instances::InstanceBatch>,
        /// Local-to-device mapping.
        transform: Transform,
    },
    /// Begins a source-only filter capture in an independent geometric domain.
    PushFilterLayer {
        /// Device bounds of the input; preparation derives intermediate/output bounds.
        bounds: Rect,
        /// Validated device-unit filters in declaration order.
        filters: Vec<crate::Filter>,
    },
    /// Begins isolated composition of subsequent commands.
    PushLayer {
        /// Group opacity, retained as f64 until preparation.
        opacity: f64,
        /// Conservative device-coordinate capture bounds, not a clip.
        bounds: Rect,
        /// Mode applied once against the parent when this layer completes.
        blend_mode: BlendMode,
        /// Optional path coverage applied once before layer opacity and blending.
        clip: Option<PathClip>,
    },
    /// Begins isolation with an additional geometric rectangle clip.
    PushRectLayer {
        /// Opacity applied after the completed subtree is clipped.
        opacity: f64,
        /// Conservative device capture bounds, not a clipping region.
        bounds: Rect,
        /// Mode applied once against the immediate parent backdrop.
        blend_mode: BlendMode,
        /// Independent path coverage, retaining its multiplicative semantics.
        clip: Option<PathClip>,
        /// Device rectangle intersected with ancestor geometric clips.
        rect_clip: RectClip,
    },
    /// Begins isolation with geometric Shape intersection and optional rectangle.
    PushShapeLayer {
        /// Opacity applied after the subtree is clipped.
        opacity: f64,
        /// Conservative device capture bounds, not a clipping region.
        bounds: Rect,
        /// Mode applied once against the immediate parent backdrop.
        blend_mode: BlendMode,
        /// Independent multiplicative path coverage.
        clip: Option<PathClip>,
        /// Optional additional geometric rectangle on this same layer.
        rect_clip: Option<RectClip>,
        /// Shape region intersected with all geometric ancestors.
        shape_clip: ShapeClip,
    },
    /// Ends the most recent isolated layer.
    PopLayer,
    /// Begins isolated mask artwork for the current completed layer.
    BeginMask {
        /// Extraction mode applied once at EndMask.
        mode: MaskMode,
    },
    /// Applies completed mask artwork; only PopLayer may follow in that layer.
    EndMask,
    /// Fills an axis-aligned logical rectangle, preserving its opposite corners.
    FillRect {
        /// Local f64 bounds, independent of the output scale.
        bounds: Rect,
        /// Local-to-device mapping; identity in standalone Canvas recording.
        transform: Transform,
        /// Floating paint.
        paint: Paint,
    },
    /// Fills a logical path; open contours close implicitly for filling.
    Fill {
        /// Shared checked f64 geometry, flattened only by the backend.
        path: Arc<Path2D>,
        /// Local-to-device mapping; identity in standalone Canvas recording.
        transform: Transform,
        /// floating paint.
        paint: Paint,
        /// Explicit contour interpretation, retained without approximation.
        rule: FillRule,
    },
}
