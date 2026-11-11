//! Measured row/column arrangement inside the bounded node traversal.

use super::spatial::{Bounds, Placement, Resolved};
use crate::{Node, ValidationError, Vector2};

#[cfg(test)]
mod tests;

/// Cross-axis alignment of measured child layout bounds.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Alignment {
    /// Align upper edges in rows, or left edges in columns; the default.
    #[default]
    Start,
    /// Center each child within the largest cross-axis extent.
    Center,
    /// Align lower edges in rows, or right edges in columns.
    End,
    /// Align real text baselines in rows, including measured text parts.
    /// Columns, rotated/upside-down nodes and baseline-less children are unsupported.
    Baseline,
}

/// A measured layout container, evaluated before its own node placement.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Layout {
    /// Arrange children horizontally, with vertical alignment.
    Row {
        /// Finite separation between nonempty children; negatives overlap.
        gap: f64,
        /// Alignment on the vertical axis.
        alignment: Alignment,
    },
    /// Arrange children vertically, with horizontal alignment.
    Column {
        /// Finite separation between nonempty children; negatives overlap.
        gap: f64,
        /// Alignment on the horizontal axis.
        alignment: Alignment,
    },
}

/// Builds a horizontal measured layout; defaults to zero gap and Start alignment.
///
/// Children are positioned in insertion order, then painted in stable z order.
/// Empty nodes consume no gap; declared points and lines still occupy a slot.
/// A child's existing transformed AABB supplies its size; its previous position
/// is replaced by arrangement. Parent placement and opacity apply afterwards.
///
/// ```
/// use motion_core::prelude::*;
/// use motion_core::{
///     DisplayList, RasterLimits, ResourceStore, ValidatedDisplayList, MAX_PATH_VERBS,
///     MAX_STROKE_VERBS, MAX_SVG_PATH_BYTES, sample_offsets,
/// };
/// let tile = || Node::from(Rect::new(0.0, 0.0, 10.0, 20.0).fill(Color::rgb(20, 150, 125)));
/// let row = Row::new().gap(12.0).children([tile(), tile()]);
/// assert_eq!(
///     row.measure(ValidationLimits::default())?,
///     Rect::new(0.0, 0.0, 32.0, 20.0)
/// );
/// let column = Column::new()
///     .gap(4.0)
///     .alignment(Alignment::End)
///     .children([row, tile()])
///     .at(Point::new(100.0, 80.0));
/// assert_eq!(
///     column.measure(ValidationLimits::default())?,
///     Rect::new(84.0, 58.0, 116.0, 102.0)
/// );
/// let list = column.compile(
///     Size::new(200.0, 160.0),
///     PixelSize::new(200, 160),
///     ValidationLimits::default(),
/// )?;
/// assert_eq!(list.commands.len(), 3);
/// # Ok::<(), motion_core::ValidationError>(())
/// ```
#[derive(Clone, Copy, Debug, Default)]
pub struct Row {
    gap: f64,
    alignment: Alignment,
}

impl Row {
    /// Creates a horizontal layout builder.
    pub fn new() -> Self {
        Self::default()
    }

    /// Replaces finite spacing; negative spacing intentionally overlaps.
    pub fn gap(mut self, gap: f64) -> Self {
        self.gap = gap;
        self
    }

    /// Replaces cross-axis alignment.
    pub fn alignment(mut self, alignment: Alignment) -> Self {
        self.alignment = alignment;
        self
    }

    /// Captures children for deferred, bounded measurement and arrangement.
    pub fn children<Child: Into<Node>>(self, children: impl IntoIterator<Item = Child>) -> Node {
        Node::layout(
            Layout::Row {
                gap: self.gap,
                alignment: self.alignment,
            },
            children,
        )
    }
}

/// Builds a vertical measured layout; defaults to zero gap and Start alignment.
#[derive(Clone, Copy, Debug, Default)]
pub struct Column {
    gap: f64,
    alignment: Alignment,
}

impl Column {
    /// Creates a vertical layout builder.
    pub fn new() -> Self {
        Self::default()
    }

    /// Replaces finite spacing; negative spacing intentionally overlaps.
    pub fn gap(mut self, gap: f64) -> Self {
        self.gap = gap;
        self
    }

    /// Replaces cross-axis alignment.
    pub fn alignment(mut self, alignment: Alignment) -> Self {
        self.alignment = alignment;
        self
    }

    /// Captures children for deferred, bounded measurement and arrangement.
    pub fn children<Child: Into<Node>>(self, children: impl IntoIterator<Item = Child>) -> Node {
        Node::layout(
            Layout::Column {
                gap: self.gap,
                alignment: self.alignment,
            },
            children,
        )
    }
}

impl Layout {
    pub(super) fn baselines(
        self,
        children: &[Node],
        context: &crate::DrawContext,
        path: &str,
    ) -> Result<Vec<Option<f64>>, ValidationError> {
        if !matches!(
            self,
            Self::Row {
                alignment: Alignment::Baseline,
                ..
            }
        ) {
            return Ok(vec![None; children.len()]);
        }
        children
            .iter()
            .enumerate()
            .map(|(index, child)| child.baseline(context, &format!("{path}.children[{index}]")))
            .collect()
    }

    pub(super) fn placements(
        self,
        children: &[(usize, Bounds)],
        baselines: &[Option<f64>],
        path: &str,
    ) -> Result<Vec<Option<(Resolved, Bounds)>>, ValidationError> {
        let (horizontal, gap, alignment) = match self {
            Self::Row { gap, alignment } => (true, gap, alignment),
            Self::Column { gap, alignment } => (false, gap, alignment),
        };
        if !gap.is_finite() {
            return Err(ValidationError::invalid(
                path,
                "gap",
                "Layout gap must be finite",
            ));
        }
        if alignment == Alignment::Baseline && !horizontal {
            return Err(ValidationError::unsupported(
                path,
                "alignment",
                "Baseline alignment is supported only in horizontal rows",
            ));
        }
        let mut cross_extent = 0.0_f64;
        let mut common_baseline = 0.0_f64;
        for (index, bounds) in children {
            if matches!(bounds, Bounds::Empty) {
                continue;
            }
            let rect = bounds.rect(&format!("{path}.children[{index}]"))?;
            if alignment == Alignment::Baseline {
                let baseline = baselines[*index].ok_or_else(|| {
                    ValidationError::unsupported(
                        path,
                        "alignment",
                        "Baseline requires text metrics for every nonempty child",
                    )
                })?;
                let offset = baseline - rect.top;
                if !offset.is_finite() {
                    return Err(ValidationError::invalid(
                        path,
                        "baseline",
                        "Baseline offset must be finite",
                    ));
                }
                common_baseline = common_baseline.max(offset);
            }
            if !rect.width().is_finite() || !rect.height().is_finite() {
                return Err(ValidationError::invalid(
                    path,
                    "layout_bounds",
                    "Layout spans must be finite",
                ));
            }
            cross_extent = cross_extent.max(if horizontal {
                rect.height()
            } else {
                rect.width()
            });
        }
        let mut cursor = 0.0_f64;
        let mut occupied = false;
        let mut placements = Vec::with_capacity(children.len());
        for (index, bounds) in children {
            if matches!(bounds, Bounds::Empty) {
                placements.push(None);
                continue;
            }
            let child_path = format!("{path}.children[{index}]");
            let rect = bounds.rect(&child_path)?;
            if occupied {
                cursor += gap;
            }
            let (main_size, cross_size) = if horizontal {
                (rect.width(), rect.height())
            } else {
                (rect.height(), rect.width())
            };
            let cross = match alignment {
                Alignment::Start => 0.0,
                Alignment::Center => (cross_extent - cross_size) * 0.5,
                Alignment::End => cross_extent - cross_size,
                Alignment::Baseline => {
                    common_baseline - (baselines[*index].expect("validated baseline") - rect.top)
                }
            };
            let offset = if horizontal {
                Vector2::new(cursor - rect.left, cross - rect.top)
            } else {
                Vector2::new(cross - rect.left, cursor - rect.top)
            };
            let placement = Placement {
                translation: offset,
                ..Placement::default()
            }
            .resolve(*bounds, &child_path)?;
            let placed_bounds = bounds.transformed(placement.transform, &child_path)?;
            placements.push(Some((placement, placed_bounds)));
            cursor += main_size;
            if !cursor.is_finite() {
                return Err(ValidationError::invalid(
                    path,
                    "layout_bounds",
                    "Layout extent must remain finite",
                ));
            }
            occupied = true;
        }
        Ok(placements)
    }
}
