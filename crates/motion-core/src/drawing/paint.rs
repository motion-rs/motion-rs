//! Gradient and static image-pattern descriptions.
//!
//! ```
//! use motion_core::prelude::*;
//! use motion_core::{
//!     DisplayList, RasterLimits, ResourceStore, ValidatedDisplayList, MAX_PATH_VERBS,
//!     MAX_STROKE_VERBS, MAX_SVG_PATH_BYTES, sample_offsets,
//! };
//! let brush = LinearGradient::new(Point::ORIGIN, Point::new(1.0, 0.0))
//!     .stop(0.0, Color::from_hex(0x1080c0))
//!     .stop(1.0, Color::WHITE)
//!     .space(GradientSpace::Oklab);
//! Node::from(Rect::new(0.0, 0.0, 8.0, 8.0).fill(brush)).compile(
//!     Size::new(8.0, 8.0),
//!     PixelSize::new(8, 8),
//!     ValidationLimits::default(),
//! )?;
//! # Ok::<(), motion_core::Error>(())
//! ```

use std::sync::Arc;

use super::AtmospherePaint;
use crate::{Color, MeshPaint, Point, Rect, Transform, ValidationError};

/// A preloaded static image sampled in local pixel coordinates, without I/O.
#[derive(Clone, Debug)]
pub struct ImagePattern {
    /// Shared immutable pixels and mip levels; no decoder or renderer is retained.
    pub image: PatternImage,
    /// Maps image pixel coordinates into shape-local coordinates.
    pub transform: Transform,
    /// Addressing policy outside the image's normalized extent.
    pub extend: GradientExtend,
    /// Texel filtering and transform-derived mip selection policy.
    pub sampling: crate::Sampling,
    /// Makes samples outside the image extent transparent instead of extending edges.
    pub decal: bool,
}

/// Shared image sampling contract captured by a paint, independent of content decoders.
#[derive(Clone, Debug)]
pub struct PatternImage(Arc<dyn crate::pixels::ImagePixels>);

impl PatternImage {
    /// Base mip width in pixels.
    pub fn width(&self) -> u32 {
        self.0.width()
    }

    /// Base mip height in pixels.
    pub fn height(&self) -> u32 {
        self.0.height()
    }

    /// Number of retained mip levels, including the base image.
    pub fn levels(&self) -> usize {
        self.0.level_count()
    }

    /// Width and height of a valid retained mip level.
    pub fn size(&self, level: usize) -> (u32, u32) {
        self.0.level_size(level)
    }

    /// Reads a linear-premultiplied texel; level and pixel indices must be valid.
    pub fn texel(&self, level: usize, column: usize, row: usize) -> [f32; 4] {
        self.0.texel(level, column, row)
    }

    /// Appends a whole level in row-major order, four `f32` per texel.
    pub fn extend_level(&self, level: usize, out: &mut Vec<f32>) {
        self.0.extend_level(level, out);
    }

    /// Stable identity of the shared pixel storage, for deduplicating per-frame accounting.
    pub fn identity(&self) -> usize {
        Arc::as_ptr(&self.0).cast::<()>() as usize
    }

    /// Non-owning handle that recognizes this exact shared pixel storage without keeping it alive.
    ///
    /// The handle keeps the allocation reserved, so its address cannot be reused by another
    /// image while the handle exists.
    pub fn downgrade(&self) -> WeakPatternImage {
        WeakPatternImage(Arc::downgrade(&self.0))
    }

    fn same(&self, other: &Self) -> bool {
        Arc::ptr_eq(&self.0, &other.0) || self.0.content_eq(other.0.as_ref())
    }
}

/// Weak identity of a [`PatternImage`]'s shared pixel storage.
#[derive(Clone, Debug)]
pub struct WeakPatternImage(std::sync::Weak<dyn crate::pixels::ImagePixels>);

impl WeakPatternImage {
    /// Whether `image` shares this handle's exact pixel storage.
    pub fn refers_to(&self, image: &PatternImage) -> bool {
        std::sync::Weak::ptr_eq(&self.0, &Arc::downgrade(&image.0))
    }
}

impl PartialEq for ImagePattern {
    fn eq(&self, other: &Self) -> bool {
        self.transform == other.transform
            && self.extend == other.extend
            && self.sampling == other.sampling
            && self.decal == other.decal
            && self.image.same(&other.image)
    }
}

impl ImagePattern {
    /// Captures an already resolved backend-independent image result.
    pub fn from_pixels(image: Arc<dyn crate::ImagePixels>) -> Self {
        Self {
            image: PatternImage(image),
            transform: Transform::IDENTITY,
            extend: GradientExtend::Repeat,
            sampling: crate::Sampling::Trilinear,
            decal: false,
        }
    }

    /// Captures a linear RGBA16F mip chain without decoding/copying pixels.
    pub fn from_asset<Image: crate::pixels::ImagePixels>(image: Arc<Image>) -> Self {
        Self {
            image: PatternImage(image),
            transform: Transform::IDENTITY,
            extend: GradientExtend::Repeat,
            sampling: crate::Sampling::Trilinear,
            decal: false,
        }
    }

    /// Selects nearest, bilinear or transform-derived trilinear sampling.
    pub fn sampling(mut self, sampling: crate::Sampling) -> Self {
        self.sampling = sampling;
        self
    }

    /// Maps image pixels into shape-local coordinates.
    pub fn transform(mut self, transform: Transform) -> Self {
        self.transform = transform;
        self
    }

    /// Selects clamped, repeated or mirrored image edges.
    pub fn extend(mut self, extend: GradientExtend) -> Self {
        self.extend = extend;
        self
    }

    /// Rejects nonfinite or noninvertible image-to-shape transforms.
    pub fn validate(&self) -> Result<(), ValidationError> {
        if !self.transform.is_finite() || self.transform.inverse().is_none() {
            return Err(invalid(
                "Image pattern transform must be finite and invertible",
            ));
        }
        Ok(())
    }
}

impl From<ImagePattern> for super::Paint {
    fn from(value: ImagePattern) -> Self {
        Self::Pattern(value)
    }
}

/// Color interpolation space for gradients.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
#[cfg_attr(
    feature = "serde",
    derive(serde::Serialize, serde::Deserialize),
    serde(rename_all = "snake_case")
)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
pub enum GradientSpace {
    /// Perceptual Oklab, matching Color interpolation.
    #[default]
    Oklab,
    /// Straight linear-light RGB and linear alpha.
    LinearRgb,
    /// Straight encoded sRGB and linear alpha.
    Srgb,
}

/// Coordinates used by gradient geometry.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
#[cfg_attr(
    feature = "serde",
    derive(serde::Serialize, serde::Deserialize),
    serde(rename_all = "snake_case")
)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
pub enum GradientUnits {
    /// Coordinates are fractions of the original shape's local bounds.
    #[default]
    ObjectBoundingBox,
    /// Coordinates are local logical drawing units.
    UserSpace,
}

/// Behavior outside a gradient's normalized interval.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
#[cfg_attr(
    feature = "serde",
    derive(serde::Serialize, serde::Deserialize),
    serde(rename_all = "snake_case")
)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
pub enum GradientExtend {
    /// Hold endpoint colors.
    #[default]
    Pad,
    /// Repeat every unit interval.
    Repeat,
    /// Reflect every second interval.
    Reflect,
}

impl GradientExtend {
    /// Maps a normalized coordinate into the selected clamped or periodic interval.
    pub fn apply(self, position: f64) -> f64 {
        match self {
            Self::Pad => position.clamp(0.0, 1.0),
            Self::Repeat => position.rem_euclid(1.0),
            Self::Reflect => {
                let value = position.rem_euclid(2.0);
                if value > 1.0 { 2.0 - value } else { value }
            }
        }
    }
}

#[derive(Clone, Debug, PartialEq)]
/// Gradient geometry in the coordinate system selected by [`GradientUnits`].
pub enum Geometry {
    /// Projection from the first point to the second point.
    Linear(Point, Point),
    /// Distance from a center divided by a positive radius.
    Radial(Point, f64),
    /// Focal radial gradient: center, positive radius and focus strictly inside the circle.
    RadialFocal(Point, f64, Point),
    /// Angular gradient around a center, with increasing start/end radians.
    Sweep(Point, f64, f64),
}

#[derive(Clone, Debug, PartialEq)]
/// Shared gradient description retained by paint and validated before preparation.
pub struct Gradient {
    /// Finite, nondegenerate shape of the gradient coordinate field.
    pub geometry: Geometry,
    /// Two to 64 ordered stops including positions zero and one; equal positions make hard stops.
    pub stops: Arc<[(f64, Color)]>,
    /// Straight-color interpolation space; sampled output is linear premultiplied RGBA.
    pub space: GradientSpace,
    /// Whether geometry uses local logical units or normalized shape bounds.
    pub units: GradientUnits,
    /// Coordinate policy before interpolation between the stop colors.
    pub extend: GradientExtend,
}

fn invalid(message: &str) -> ValidationError {
    ValidationError::invalid("paint.gradient", "parameters", message)
}

impl Gradient {
    fn new(geometry: Geometry) -> Self {
        Self {
            geometry,
            stops: Arc::from([]),
            space: GradientSpace::default(),
            units: GradientUnits::default(),
            extend: GradientExtend::default(),
        }
    }

    fn stop(mut self, position: f64, color: Color) -> Self {
        if self.stops.len() < 65 {
            let mut stops = self.stops.to_vec();
            stops.push((position, color));
            self.stops = stops.into();
        }
        self
    }

    /// Checks ordered endpoint-inclusive stops and finite, nondegenerate geometry.
    pub fn validate(&self) -> Result<(), ValidationError> {
        if !(2..=64).contains(&self.stops.len())
            || self.stops.first().map(|stop| stop.0) != Some(0.0)
            || self.stops.last().map(|stop| stop.0) != Some(1.0)
            || self
                .stops
                .iter()
                .any(|(position, _)| !position.is_finite() || !(0.0..=1.0).contains(position))
            || self.stops.windows(2).any(|pair| pair[0].0 > pair[1].0)
        {
            return Err(invalid(
                "Require 2..=64 ordered stops including zero and one",
            ));
        }
        match self.geometry {
            Geometry::Linear(first, second)
                if [first.x, first.y, second.x, second.y]
                    .iter()
                    .all(|value| value.is_finite())
                    && first != second
                    && first.distance(second).is_finite() =>
            {
                Ok(())
            }
            Geometry::Radial(center, radius)
                if center.x.is_finite()
                    && center.y.is_finite()
                    && radius.is_finite()
                    && radius > 0.0 =>
            {
                Ok(())
            }
            Geometry::RadialFocal(center, radius, focus)
                if center.x.is_finite()
                    && center.y.is_finite()
                    && focus.x.is_finite()
                    && focus.y.is_finite()
                    && radius.is_finite()
                    && radius > 0.0
                    && center.distance(focus) < radius =>
            {
                Ok(())
            }
            Geometry::Sweep(center, start, end)
                if center.x.is_finite()
                    && center.y.is_finite()
                    && start.is_finite()
                    && end.is_finite()
                    && end > start
                    && end - start <= std::f64::consts::TAU =>
            {
                Ok(())
            }
            _ => Err(invalid(
                "Gradient geometry must be finite and nondegenerate",
            )),
        }
    }

    /// Converts a local point to an extended normalized coordinate.
    /// Object-bounds units require finite positive width and height; validate geometry first.
    pub fn position(&self, point: Point, bounds: Rect) -> Result<f64, ValidationError> {
        let point = match self.units {
            GradientUnits::UserSpace => point,
            GradientUnits::ObjectBoundingBox => {
                if !bounds.width().is_finite()
                    || !bounds.height().is_finite()
                    || bounds.width() <= 0.0
                    || bounds.height() <= 0.0
                {
                    return Err(invalid(
                        "Object bounding-box gradient requires finite nonzero shape bounds",
                    ));
                }
                Point::new(
                    (point.x - bounds.left) / bounds.width(),
                    (point.y - bounds.top) / bounds.height(),
                )
            }
        };
        let position = match self.geometry {
            Geometry::Linear(start, end) => {
                let direction = end - start;
                let length = direction.length();
                (point - start).dot(direction / length) / length
            }
            Geometry::Radial(center, radius) => point.distance(center) / radius,
            Geometry::RadialFocal(center, radius, focus) => {
                let offset = point - focus;
                let displacement = focus - center;
                let square = offset.dot(offset);
                if square == 0.0 {
                    0.0
                } else {
                    let projection = offset.dot(displacement);
                    square
                        / (-projection
                            + libm::sqrt(
                                projection * projection
                                    + square * (radius * radius - displacement.dot(displacement)),
                            ))
                }
            }
            Geometry::Sweep(center, start, end) => {
                let angle = libm::atan2(point.y - center.y, point.x - center.x);
                (angle - start).rem_euclid(std::f64::consts::TAU) / (end - start)
            }
        };
        if !position.is_finite() {
            return Err(invalid("Gradient sample position must remain finite"));
        }
        Ok(self.extend.apply(position))
    }

    /// Interpolates validated stops to linear-premultiplied RGBA at a normalized position.
    /// Call [`Self::validate`] first and apply coordinate extension before sampling.
    pub fn color(&self, position: f64) -> [f64; 4] {
        let upper = self
            .stops
            .partition_point(|(offset, _)| *offset <= position);
        if upper == self.stops.len() {
            return self.stops[upper - 1].1.to_linear_premultiplied();
        }
        let (start, first) = self.stops[upper.saturating_sub(1)];
        let (end, second) = self.stops[upper];
        let progress = ((position - start) / (end - start)).clamp(0.0, 1.0);
        if progress == 0.0 {
            return first.to_linear_premultiplied();
        }
        if progress == 1.0 {
            return second.to_linear_premultiplied();
        }
        match self.space {
            GradientSpace::Oklab => {
                let first = first.to_rgba_float();
                let second = second.to_rgba_float();
                let alpha = first[3] * (1.0 - progress) + second[3] * progress;
                if alpha == 0.0 {
                    return [0.0; 4];
                }
                let left = crate::color::srgb_to_oklab([first[0], first[1], first[2]]);
                let right = crate::color::srgb_to_oklab([second[0], second[1], second[2]]);
                let rgb = crate::color::oklab_to_srgb(std::array::from_fn(|index| {
                    left[index] * (1.0 - progress) + right[index] * progress
                }))
                .map(crate::color::srgb_to_linear);
                [rgb[0] * alpha, rgb[1] * alpha, rgb[2] * alpha, alpha]
            }
            GradientSpace::LinearRgb => {
                let first = first.to_rgba_float();
                let second = second.to_rgba_float();
                let alpha = first[3] * (1.0 - progress) + second[3] * progress;
                std::array::from_fn(|channel| {
                    if channel == 3 {
                        alpha
                    } else {
                        (crate::color::srgb_to_linear(first[channel]) * (1.0 - progress)
                            + crate::color::srgb_to_linear(second[channel]) * progress)
                            * alpha
                    }
                })
            }
            GradientSpace::Srgb => {
                let first = first.to_rgba_float();
                let second = second.to_rgba_float();
                let alpha = first[3] * (1.0 - progress) + second[3] * progress;
                let decode = crate::color::srgb_to_linear;
                [
                    decode(first[0] * (1.0 - progress) + second[0] * progress) * alpha,
                    decode(first[1] * (1.0 - progress) + second[1] * progress) * alpha,
                    decode(first[2] * (1.0 - progress) + second[2] * progress) * alpha,
                    alpha,
                ]
            }
        }
    }
}

macro_rules! gradient {
    ($name:ident, $variant:ident, $description:literal) => {
        #[doc = $description]
        #[derive(Clone, Debug, PartialEq)]
        pub struct $name(pub Gradient);
        impl $name {
            /// Appends a sorted color stop; invalid/excessive inputs fail recording.
            pub fn stop(self, position: f64, color: Color) -> Self {
                Self(self.0.stop(position, color))
            }
            /// Selects the interpolation space.
            pub fn space(mut self, space: GradientSpace) -> Self {
                self.0.space = space;
                self
            }
            /// Selects object-relative or local user-space coordinates.
            pub fn units(mut self, units: GradientUnits) -> Self {
                self.0.units = units;
                self
            }
            /// Selects pad/repeat/reflect outside the color interval.
            pub fn extend(mut self, extend: GradientExtend) -> Self {
                self.0.extend = extend;
                self
            }
            /// Checked linear-premultiplied sample, useful for reference checks.
            pub fn sample(&self, point: Point, bounds: Rect) -> Result<[f64; 4], ValidationError> {
                self.0.validate()?;
                Ok(self.0.color(self.0.position(point, bounds)?))
            }
        }
        impl From<$name> for super::Paint {
            fn from(value: $name) -> Self {
                Self::$variant(value)
            }
        }
    };
}

gradient!(
    LinearGradient,
    Linear,
    "A linear gradient in f64 authoring coordinates."
);

impl LinearGradient {
    /// Creates geometry without implicit stops.
    pub fn new(start: Point, end: Point) -> Self {
        Self(Gradient::new(Geometry::Linear(start, end)))
    }
}

gradient!(
    RadialGradient,
    Radial,
    "A circular radial gradient; object bounds can stretch it elliptically."
);

impl RadialGradient {
    /// Moves the zero-radius focus inside the outer circle. Invalid focus or
    /// degenerate geometry is rejected during recording/sampling.
    pub fn focal(mut self, focus: Point) -> Self {
        if let Geometry::Radial(center, radius) | Geometry::RadialFocal(center, radius, _) =
            self.0.geometry
        {
            self.0.geometry = Geometry::RadialFocal(center, radius, focus);
        }
        self
    }

    /// Creates geometry with a positive radius.
    pub fn new(center: Point, radius: f64) -> Self {
        Self(Gradient::new(Geometry::Radial(center, radius)))
    }
}

gradient!(
    SweepGradient,
    Sweep,
    "A clockwise angular gradient in radians."
);

impl SweepGradient {
    /// Creates an increasing angular interval at most one turn.
    pub fn new(center: Point, start_angle: f64, end_angle: f64) -> Self {
        Self(Gradient::new(Geometry::Sweep(
            center,
            start_angle,
            end_angle,
        )))
    }
}

/// Floating paint retained without implicit gamut clipping or byte quantization.
#[derive(Clone, Debug, PartialEq)]
#[non_exhaustive]
pub enum Paint {
    /// A straight-alpha, floating encoded-sRGB color.
    Solid(Color),
    /// Linear gradient with shared CPU-generated color data.
    Linear(LinearGradient),
    /// Circular/elliptical radial gradient.
    Radial(RadialGradient),
    /// Angular gradient.
    Sweep(SweepGradient),
    /// Bilinear linear-premultiplied sampling of a preloaded image.
    Pattern(ImagePattern),
    /// Analytic inverse-bilinear mesh evaluated independently by each backend.
    Mesh(MeshPaint),
    /// Seeded atmospheric noise and envelope evaluated at an explicit time.
    Atmosphere(AtmospherePaint),
}

impl Paint {
    /// Borrows gradient data for gradient paints, otherwise returns `None`.
    pub fn gradient(&self) -> Option<&Gradient> {
        match self {
            Self::Linear(value) => Some(&value.0),
            Self::Radial(value) => Some(&value.0),
            Self::Sweep(value) => Some(&value.0),
            _ => None,
        }
    }

    /// Validates gradient geometry/stops or image transforms without sampling pixels.
    pub fn validate(&self) -> Result<(), ValidationError> {
        match self {
            Self::Solid(_) => Ok(()),
            Self::Linear(value) => value.0.validate(),
            Self::Radial(value) => value.0.validate(),
            Self::Sweep(value) => value.0.validate(),
            Self::Pattern(value) => value.validate(),
            Self::Mesh(value) => value.validate(),
            Self::Atmosphere(value) => value.validate(),
        }
    }
}

impl From<Color> for Paint {
    fn from(color: Color) -> Self {
        Self::Solid(color)
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn focal_radial_uses_ray_circle_intersection_without_changing_end_colors() {
        use super::*;
        let gradient = RadialGradient::new(Point::new(0.5, 0.5), 0.5)
            .focal(Point::new(0.25, 0.5))
            .stop(0.0, Color::BLACK)
            .stop(1.0, Color::WHITE)
            .space(GradientSpace::LinearRgb);
        let bounds = Rect::new(0.0, 0.0, 1.0, 1.0);
        assert_eq!(
            gradient.sample(Point::new(0.25, 0.5), bounds).unwrap(),
            [0.0, 0.0, 0.0, 1.0]
        );
        assert_eq!(
            gradient.sample(Point::new(1.0, 0.5), bounds).unwrap(),
            [1.0; 4]
        );
        let center = gradient.sample(Point::new(0.5, 0.5), bounds).unwrap();
        assert!((center[0] - 1.0 / 3.0).abs() < 1e-12);
    }

    use super::*;

    #[test]
    fn gradients_preserve_spaces_units_extension_and_hard_stops() {
        let bounds = Rect::new(10.0, 20.0, 30.0, 40.0);
        let linear = LinearGradient::new(Point::ORIGIN, Point::new(1.0, 0.0))
            .stop(0.0, Color::BLACK)
            .stop(1.0, Color::WHITE)
            .space(GradientSpace::LinearRgb);
        assert_eq!(
            linear.sample(Point::new(20.0, 30.0), bounds).unwrap(),
            [0.5, 0.5, 0.5, 1.0]
        );
        assert_eq!(
            linear
                .clone()
                .extend(GradientExtend::Repeat)
                .sample(Point::new(40.0, 30.0), bounds)
                .unwrap(),
            [0.5, 0.5, 0.5, 1.0]
        );
        assert_eq!(
            linear
                .clone()
                .extend(GradientExtend::Reflect)
                .sample(Point::new(40.0, 30.0), bounds)
                .unwrap(),
            [0.5, 0.5, 0.5, 1.0]
        );
        let radial = RadialGradient::new(Point::new(0.5, 0.5), 0.5)
            .stop(0.0, Color::BLACK)
            .stop(1.0, Color::WHITE)
            .space(GradientSpace::LinearRgb);
        assert_eq!(
            radial.sample(Point::new(20.0, 30.0), bounds).unwrap(),
            [0.0, 0.0, 0.0, 1.0]
        );
        assert_eq!(
            radial.sample(Point::new(30.0, 30.0), bounds).unwrap(),
            [1.0; 4]
        );
        let sweep = SweepGradient::new(Point::ORIGIN, 0.0, std::f64::consts::TAU)
            .stop(0.0, Color::BLACK)
            .stop(1.0, Color::WHITE)
            .space(GradientSpace::LinearRgb)
            .units(GradientUnits::UserSpace);
        assert_eq!(
            sweep.sample(Point::new(0.0, 1.0), bounds).unwrap(),
            [0.25, 0.25, 0.25, 1.0]
        );
        assert!(
            LinearGradient::new(Point::ORIGIN, Point::ORIGIN)
                .stop(0.0, Color::WHITE)
                .stop(1.0, Color::BLACK)
                .sample(Point::ORIGIN, bounds)
                .is_err()
        );
        let hard = LinearGradient::new(Point::ORIGIN, Point::new(1.0, 0.0))
            .stop(0.0, Color::BLACK)
            .stop(0.5, Color::BLACK)
            .stop(0.5, Color::WHITE)
            .stop(1.0, Color::WHITE);
        assert_eq!(
            hard.sample(Point::new(20.0, 20.0), bounds).unwrap(),
            [1.0; 4]
        );
        let encoded = linear
            .clone()
            .space(GradientSpace::Srgb)
            .sample(Point::new(20.0, 30.0), bounds)
            .unwrap();
        assert!((encoded[0] - 0.21404114048223255).abs() < 1e-12);
        let perceptual = linear
            .clone()
            .space(GradientSpace::Oklab)
            .sample(Point::new(20.0, 30.0), bounds)
            .unwrap();
        for channel in &perceptual[..3] {
            assert!((*channel - 0.125).abs() < 1e-6);
        }
        let alpha = LinearGradient::new(Point::ORIGIN, Point::new(1.0, 0.0))
            .stop(0.0, Color::rgb(255, 0, 0))
            .stop(1.0, Color::rgb(0, 255, 0).with_alpha(0.0))
            .space(GradientSpace::LinearRgb);
        assert_eq!(
            alpha.sample(Point::new(20.0, 30.0), bounds).unwrap(),
            [0.25, 0.25, 0.0, 0.5]
        );
    }
}
