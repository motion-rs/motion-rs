//! Immutable Node snapshots, scene evaluation and composition.

use std::sync::Arc;

use crate::drawing::BlendMode;
use crate::validation::ValidationState;
use crate::{
    DrawContext, Drawable, Duration, Pivot, Point, Rect, SceneContext, Time, ValidationError,
    ValidationLimits,
};

pub mod camera;
mod compile;
pub mod context;
pub mod elevation;
mod layout;
pub mod sampling;
pub mod skeleton;
mod spatial;
pub mod timeline;
pub mod transition;

pub use layout::{Alignment, Column, Layout, Row};
pub use spatial::Side;
use spatial::{Bounds, Placement};

mod modifiers;
mod node;
pub mod scene;

pub use compile::NodeInspection;
pub use modifiers::NodeModifiers;
pub use node::{IntoNodeGroup, IteratorGroup, Node, TupleGroup};
use node::{NodeKind, NodeMask};
pub use scene::{Scene, WithDuration};

#[cfg(test)]
mod tests;
