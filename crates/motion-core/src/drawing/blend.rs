//! Blending, fill and mask interpretation.

/// Linear-light layer blending, with explicit compositing space.
///
/// Separable modes evaluate straight linear channels with source-over alpha.
/// Extended RGB is not gamut-clipped. Plus instead adds premultiplied RGB and
/// clamps the summed alpha to one. Dodge/Burn retain their formula's intrinsic
/// endpoints and saturation; other modes extend their piecewise formulas without
/// input clipping. Nonseparable modes use hue/saturation/luminosity replacement.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
#[cfg_attr(
    feature = "serde",
    derive(serde::Serialize, serde::Deserialize),
    serde(rename_all = "snake_case")
)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
pub enum BlendMode {
    /// Premultiplied source-over; the default.
    #[default]
    SourceOver,
    /// Unclipped additive RGB with saturated additive alpha.
    Plus,
    /// Product of straight source and backdrop channels.
    Multiply,
    /// Source plus backdrop minus their product.
    Screen,
    /// Minimum of straight source and backdrop channels.
    Darken,
    /// Maximum of straight source and backdrop channels.
    Lighten,
    /// Absolute difference of straight channels.
    Difference,
    /// Source plus backdrop minus twice their product.
    Exclusion,
    /// Multiply or screen, selected by the backdrop channel at one half.
    Overlay,
    /// Multiply or screen, selected by the source channel at one half.
    HardLight,
    /// Piecewise soft lighting, with the low-backdrop cubic and upper square root.
    SoftLight,
    /// Backdrop divided by one minus source, saturated above at one.
    ///
    /// Nonpositive backdrop returns zero before source at or above one returns one.
    ColorDodge,
    /// One minus the saturated ratio of one minus backdrop to source.
    ///
    /// Backdrop at or above one returns one before nonpositive source returns zero.
    ColorBurn,
    /// Source hue with backdrop saturation and luminosity.
    Hue,
    /// Source saturation with backdrop hue and luminosity.
    Saturation,
    /// Source hue/saturation with backdrop luminosity.
    Color,
    /// Source luminosity with backdrop hue/saturation.
    Luminosity,
    /// Internal linear-only additive capture used by sample accumulation.
    #[doc(hidden)]
    LinearPlus,
}

/// Color space used for compositing, independent of stored linear pixels.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum BlendSpace {
    /// Composite straight linear RGB, retaining the default.
    #[default]
    LinearRgb,
    /// Composite encoded sRGB, then return to linear-premultiplied storage.
    Srgb,
}

/// How closed contours determine the filled interior.
///
/// Recording retains both rules without silently substituting one for
/// the default NonZero rule.
///
/// ```
/// use motion_core::prelude::*;
/// use motion_core::{
///     DisplayList, RasterLimits, ResourcePool, ValidatedDisplayList, MAX_PATH_VERBS,
///     MAX_STROKE_VERBS, MAX_SVG_PATH_BYTES, sample_offsets,
/// };
/// let path = Path2D::from_rect(Rect::new(0.0, 0.0, 10.0, 20.0))?;
/// let fill = path.fill(Color::WHITE);
/// let native = DrawContext::default().record(&fill, ValidationLimits::default())?;
/// assert!(matches!(
///     native[0],
///     DrawCommand::Fill {
///         rule: FillRule::NonZero,
///         ..
///     }
/// ));
/// let even = DrawContext::default().record(
///     &fill.fill_rule(FillRule::EvenOdd),
///     ValidationLimits::default(),
/// )?;
/// assert!(matches!(
///     even[0],
///     DrawCommand::Fill {
///         rule: FillRule::EvenOdd,
///         ..
///     }
/// ));
/// # Ok::<(), motion_core::ValidationError>(())
/// ```
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
#[cfg_attr(
    feature = "serde",
    derive(serde::Serialize, serde::Deserialize),
    serde(rename_all = "snake_case")
)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
pub enum FillRule {
    /// Nonzero signed winding count; the default.
    #[default]
    NonZero,
    /// Odd crossing count.
    EvenOdd,
}

/// How linear-premultiplied content becomes a layer's mask coverage.
///
/// Coverage is computed in the compositing color space and clamped
/// to 0..=1 before optional inversion, without altering working colors.
/// Alpha-zero pixels have zero ordinary coverage, even with hidden extended
/// RGB; inverted modes therefore retain source content at those pixels.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
#[cfg_attr(
    feature = "serde",
    derive(serde::Serialize, serde::Deserialize),
    serde(rename_all = "snake_case")
)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
pub enum MaskMode {
    /// Use only the completed mask's alpha.
    #[default]
    Alpha,
    /// One minus the completed mask's alpha, including transparent artwork.
    InvertedAlpha,
    /// Use Rec.709 luminance of linear premultiplied RGB, already alpha-weighted.
    Luminance,
    /// One minus clamped linear-premultiplied luminance; alpha is not inverted first.
    InvertedLuminance,
}
