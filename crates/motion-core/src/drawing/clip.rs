//! Coverage masks and geometric clip domains.

use super::*;

/// How a subtree clip combines with its geometric ancestors.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum ClipMode {
    /// Multiply separately rasterized clip coverage, preserving mask semantics.
    #[default]
    Coverage,
    /// Intersect geometry before coverage quantization.
    Geometric,
}

impl From<PathClip> for ShapeClip {
    fn from(clip: PathClip) -> Self {
        Self {
            geometry: ClipGeometry::Path(clip.path),
            transform: clip.transform,
            rule: clip.rule,
        }
    }
}

impl From<RectClip> for ShapeClip {
    fn from(clip: RectClip) -> Self {
        Self {
            geometry: ClipGeometry::Rectangle(clip.bounds),
            transform: clip.transform,
            rule: FillRule::NonZero,
        }
    }
}

/// A checked path used as a coverage mask on an isolated subtree.
///
/// Transform and rule remain editable and validate during compilation/preflight.
/// The transform is node-local in authoring and device-space in a DisplayList.
#[derive(Clone, Debug, PartialEq)]
pub struct PathClip {
    /// Shared geometry; open contours close implicitly for filling.
    pub path: Arc<Path2D>,
    /// Mapping from path coordinates into the owning coordinate system.
    pub transform: Transform,
    /// Explicit filled-interior rule.
    pub rule: FillRule,
}

impl PathClip {
    /// Creates a nonzero clip with an identity local transform.
    pub fn new(path: Path2D) -> Self {
        Self {
            path: Arc::new(path),
            transform: Transform::IDENTITY,
            rule: FillRule::NonZero,
        }
    }
}

impl From<Path2D> for PathClip {
    fn from(path: Path2D) -> Self {
        Self::new(path)
    }
}

/// A local rectangle whose transformed region intersects ancestor rectangles.
///
/// Unlike PathClip, nested rectangles intersect before coverage quantization.
/// Raw bounds and transforms validate during authoring/DisplayList preflight.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct RectClip {
    /// Opposite local corners; zero area is an empty clipping region.
    pub bounds: Rect,
    /// Local mapping in authoring, device mapping in a DisplayList.
    pub transform: Transform,
}

impl RectClip {
    /// Creates a local rectangle without narrowing or unchecked path conversion.
    pub const fn new(bounds: Rect) -> Self {
        Self {
            bounds,
            transform: Transform::IDENTITY,
        }
    }
}

impl From<Rect> for RectClip {
    fn from(bounds: Rect) -> Self {
        Self::new(bounds)
    }
}

/// A snapshotted Shape used for geometric intersection, not coverage multiplication.
#[derive(Clone, Debug, PartialEq)]
pub struct ShapeClip {
    /// Snapshot of the local clipping geometry, not a bounding-box substitute.
    pub geometry: ClipGeometry,
    /// Local mapping in authoring, device mapping in a DisplayList.
    pub transform: Transform,
    /// Interior interpretation of this shape independently of its ancestors.
    pub rule: FillRule,
}

#[derive(Clone, Debug, PartialEq)]
/// Immutable rectangle or path geometry used to intersect geometric clipping domains.
pub enum ClipGeometry {
    /// Raw opposite-corner rectangle, validated before geometric intersection.
    Rectangle(Rect),
    /// Shared path whose contour interpretation comes from its owning clip.
    Path(Arc<Path2D>),
}

impl ShapeClip {
    /// Captures a shape once; trait objects are accepted without retaining them.
    ///
    /// Exact rectangles retain raw bounds so invalid rectangles fail validation
    /// rather than panicking in Shape::path. Other shapes share a checked path.
    ///
    /// ```
    /// use motion_core::prelude::*;
    /// use motion_core::{
    ///     DisplayList, RasterLimits, ResourcePool, ValidatedDisplayList, MAX_PATH_VERBS,
    ///     MAX_STROKE_VERBS, MAX_SVG_PATH_BYTES, sample_offsets,
    /// };
    /// let path = Path2D::from_rect(Rect::new(0.0, 0.0, 1.0, 1.0))?;
    /// let shape: &dyn Shape = &path;
    /// let mut clip = ShapeClip::new(shape);
    /// clip.rule = FillRule::EvenOdd;
    /// clip.transform = Transform::translate(0.5, 0.0);
    /// let frame = Node::from(Rect::new(0.0, 0.0, 2.0, 1.0).fill(Color::WHITE))
    ///     .clip_with(clip, motion_core::ClipMode::Geometric)
    ///     .compile(
    ///         Size::new(2.0, 1.0),
    ///         PixelSize::new(2, 1),
    ///         ValidationLimits::default(),
    ///     )?;
    /// # Ok::<(), motion_core::ValidationError>(())
    /// ```
    pub fn new(shape: &(impl Shape + ?Sized)) -> Self {
        Self {
            geometry: shape.as_rect().map_or_else(
                || ClipGeometry::Path(Arc::new(shape.path())),
                ClipGeometry::Rectangle,
            ),
            transform: Transform::IDENTITY,
            rule: FillRule::NonZero,
        }
    }

    /// Checks geometry and transform, returning the path-verb count for structural charging.
    pub fn validate(&self, path: &str) -> Result<usize, ValidationError> {
        match &self.geometry {
            ClipGeometry::Rectangle(bounds) => {
                crate::display_list::mapped_bounds(*bounds, self.transform, path)?;
                Ok(0)
            }
            ClipGeometry::Path(geometry) => {
                crate::display_list::validate_path(geometry, self.transform, path)?;
                Ok(geometry.verbs().len())
            }
        }
    }
}

impl<Geometry: Shape> From<Geometry> for ShapeClip {
    fn from(shape: Geometry) -> Self {
        Self::new(&shape)
    }
}
