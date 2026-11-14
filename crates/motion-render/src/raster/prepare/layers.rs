//! Reference plus-layer capture trimming after backend coverage preparation.

use crate::prepared::{FrameRecords, PixelRect, PreparedItem};
use motion_core::ValidationError;
use motion_core::budget::Budget;
use std::mem::size_of;

pub(super) fn trim_plus_layers(
    frame: &mut FrameRecords,
    budget: &mut Budget,
) -> Result<(), ValidationError> {
    struct Capture {
        index: Option<usize>,
        content: Option<PixelRect>,
    }
    let union = |content: &mut Option<PixelRect>, bounds: PixelRect| {
        if bounds.pixels() > 0 {
            *content = Some(content.map_or(bounds, |old| PixelRect {
                left: old.left.min(bounds.left),
                top: old.top.min(bounds.top),
                right: old.right.max(bounds.right),
                bottom: old.bottom.max(bounds.bottom),
            }));
        }
    };
    let mut captures: Vec<Capture> = Vec::new();
    budget.work(frame.items.len() as u64, "prepared.layers")?;
    for index in 0..frame.items.len() {
        match &frame.items[index] {
            PreparedItem::PushLayer { .. } => budget.push(
                &mut captures,
                Capture {
                    index: Some(index),
                    content: None,
                },
                "prepared.layers",
            )?,
            PreparedItem::BeginMask { .. } => {
                if let Some(capture) = captures.last_mut()
                    && let Some(owner) = capture.index
                    && let PreparedItem::PushLayer { layer } = &frame.items[owner]
                {
                    capture.content = Some(layer.bounds);
                }
                budget.push(
                    &mut captures,
                    Capture {
                        index: None,
                        content: None,
                    },
                    "prepared.layers",
                )?;
            }
            PreparedItem::EndMask | PreparedItem::PopLayer => {
                let capture = captures.pop().ok_or_else(|| {
                    ValidationError::invalid("prepared.layers", "scope", "Unbalanced capture")
                })?;
                budget.release(size_of::<Capture>() as u64);
                if let Some(owner) = capture.index {
                    let PreparedItem::PushLayer { layer } = &mut frame.items[owner] else {
                        return Err(ValidationError::invalid(
                            "prepared.layers",
                            "scope",
                            "Invalid capture owner",
                        ));
                    };
                    if layer.filters.is_none()
                        && layer.alpha_clip.is_none()
                        && layer.geometric_clip.is_none()
                        && matches!(
                            layer.blend_mode,
                            motion_core::BlendMode::Plus | motion_core::BlendMode::LinearPlus
                        )
                    {
                        layer.bounds = layer.bounds.intersect(capture.content.unwrap_or_default());
                    }
                    if let Some(parent) = captures.last_mut() {
                        union(&mut parent.content, layer.bounds);
                    }
                }
            }
            PreparedItem::Fill { strips, spans, .. } => {
                budget.work((strips.len() + spans.len()) as u64, "prepared.layers")?;
                if let Some(capture) = captures.last_mut() {
                    for strip in &frame.strips[strips.clone()] {
                        union(
                            &mut capture.content,
                            PixelRect {
                                left: strip.x,
                                top: strip.y,
                                right: strip.x + strip.width,
                                bottom: strip.y + strip.height,
                            },
                        );
                    }
                    for span in &frame.spans[spans.clone()] {
                        union(
                            &mut capture.content,
                            PixelRect {
                                left: span.start_x,
                                top: span.y,
                                right: span.end_x,
                                bottom: span.y + span.height,
                            },
                        );
                    }
                }
            }
            PreparedItem::ParticleInstances { .. } => {
                if let Some(capture) = captures.last_mut()
                    && let Some(owner) = capture.index
                    && let PreparedItem::PushLayer { layer } = &frame.items[owner]
                {
                    capture.content = Some(layer.bounds);
                }
            }
        }
    }
    Ok(())
}
