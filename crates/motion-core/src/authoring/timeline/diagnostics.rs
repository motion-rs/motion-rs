//! Non-fatal observations recorded while building a timeline.

use crate::{Duration, Time};
use std::sync::Arc;

/// Boundary that moved when scheduling onto the timeline's frame grid.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum BoundaryEdge {
    /// Clip activation start.
    Start,
    /// Clip exclusive end.
    End,
    /// Trailing track cursor, including an explicit gap.
    Cursor,
    /// Explicit timeline duration override.
    Duration,
}

/// Structured warning; construction never writes warnings to stderr.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct BoundaryAdjustment {
    /// Track insertion index, absent for a timeline duration override.
    pub track: Option<usize>,
    /// Stable clip insertion ID, absent for cursors or duration overrides.
    pub clip: Option<usize>,
    /// Adjusted boundary category.
    pub edge: BoundaryEdge,
    /// Original tick position.
    pub requested: Time,
    /// Nearest representable frame boundary; ties choose the later frame.
    pub aligned: Time,
    /// Exact aligned frame index.
    pub frame: i64,
}

/// One non-fatal observation about a built timeline.
#[derive(Clone, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub enum TimelineWarning {
    /// A boundary moved onto the frame grid.
    Boundary(BoundaryAdjustment),
    /// The explicit duration is shorter than the content, so the picture is cut
    /// off. Audio is not trimmed; the output range limits it.
    Truncated {
        /// Explicit, frame-aligned duration.
        duration: Duration,
        /// Frame-aligned end of the tracks and audio.
        content: Duration,
    },
}

/// Warnings collected by a timeline builder; fatal errors are returned by `build`.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Diagnostics {
    warnings: Arc<Vec<TimelineWarning>>,
}

impl Diagnostics {
    pub(super) fn new(warnings: Vec<TimelineWarning>) -> Self {
        Self {
            warnings: Arc::new(warnings),
        }
    }

    /// All warnings in the order they were recorded.
    pub fn warnings(&self) -> &[TimelineWarning] {
        &self.warnings
    }

    /// Frame-grid adjustments, without other warning kinds.
    pub fn boundary_adjustments(&self) -> impl Iterator<Item = &BoundaryAdjustment> {
        self.warnings.iter().filter_map(|warning| match warning {
            TimelineWarning::Boundary(adjustment) => Some(adjustment),
            _ => None,
        })
    }

    /// Whether the timeline built without any warning.
    pub fn is_empty(&self) -> bool {
        self.warnings.is_empty()
    }
}
