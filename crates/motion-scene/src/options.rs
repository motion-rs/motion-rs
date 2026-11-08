//! Serializable mirrors of engine enumerations, written as snake_case strings.

use motion_core::{
    BlendMode, ClipMode, Direction, FillRule, GradientExtend, GradientSpace, GradientUnits,
    IrisShape, LineCap, MaskMode, Pivot, Point, Sampling, Waveform,
};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

/// Local anchor for placement, rotation and scale; mirrors [`Pivot`].
#[derive(Clone, Copy, Debug, Default, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum PivotSpec {
    /// Upper-left corner.
    TopLeft,
    /// Midpoint of the upper edge.
    Top,
    /// Upper-right corner.
    TopRight,
    /// Midpoint of the left edge.
    Left,
    /// Bounds midpoint.
    #[default]
    Center,
    /// Midpoint of the right edge.
    Right,
    /// Lower-left corner.
    BottomLeft,
    /// Midpoint of the lower edge.
    Bottom,
    /// Lower-right corner.
    BottomRight,
    /// Text baseline origin, or bottom-left without baseline metrics.
    Baseline,
    /// Fractions `[x, y]` of the bounds, without clamping.
    Relative([f64; 2]),
    /// Explicit local point `[x, y]`.
    Point([f64; 2]),
}

impl From<PivotSpec> for Pivot {
    fn from(pivot: PivotSpec) -> Self {
        match pivot {
            PivotSpec::TopLeft => Self::TopLeft,
            PivotSpec::Top => Self::Top,
            PivotSpec::TopRight => Self::TopRight,
            PivotSpec::Left => Self::Left,
            PivotSpec::Center => Self::Center,
            PivotSpec::Right => Self::Right,
            PivotSpec::BottomLeft => Self::BottomLeft,
            PivotSpec::Bottom => Self::Bottom,
            PivotSpec::BottomRight => Self::BottomRight,
            PivotSpec::Baseline => Self::Baseline,
            PivotSpec::Relative([x, y]) => Self::Relative(x, y),
            PivotSpec::Point([x, y]) => Self::Point(Point::new(x, y)),
        }
    }
}

/// Subtree compositing mode; mirrors [`BlendMode`].
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum BlendModeSpec {
    /// Ordinary alpha compositing.
    #[default]
    SourceOver,
    /// Additive compositing.
    Plus,
    /// Multiply.
    Multiply,
    /// Screen.
    Screen,
    /// Darken.
    Darken,
    /// Lighten.
    Lighten,
    /// Difference.
    Difference,
    /// Exclusion.
    Exclusion,
    /// Overlay.
    Overlay,
    /// Hard light.
    HardLight,
    /// Soft light.
    SoftLight,
    /// Color dodge.
    ColorDodge,
    /// Color burn.
    ColorBurn,
    /// Hue.
    Hue,
    /// Saturation.
    Saturation,
    /// Color.
    Color,
    /// Luminosity.
    Luminosity,
    /// Linear-light additive compositing.
    LinearPlus,
}

impl From<BlendModeSpec> for BlendMode {
    fn from(mode: BlendModeSpec) -> Self {
        match mode {
            BlendModeSpec::SourceOver => Self::SourceOver,
            BlendModeSpec::Plus => Self::Plus,
            BlendModeSpec::Multiply => Self::Multiply,
            BlendModeSpec::Screen => Self::Screen,
            BlendModeSpec::Darken => Self::Darken,
            BlendModeSpec::Lighten => Self::Lighten,
            BlendModeSpec::Difference => Self::Difference,
            BlendModeSpec::Exclusion => Self::Exclusion,
            BlendModeSpec::Overlay => Self::Overlay,
            BlendModeSpec::HardLight => Self::HardLight,
            BlendModeSpec::SoftLight => Self::SoftLight,
            BlendModeSpec::ColorDodge => Self::ColorDodge,
            BlendModeSpec::ColorBurn => Self::ColorBurn,
            BlendModeSpec::Hue => Self::Hue,
            BlendModeSpec::Saturation => Self::Saturation,
            BlendModeSpec::Color => Self::Color,
            BlendModeSpec::Luminosity => Self::Luminosity,
            BlendModeSpec::LinearPlus => Self::LinearPlus,
        }
    }
}

/// Interior rule for self-intersecting fills; mirrors [`FillRule`].
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum FillRuleSpec {
    /// Nonzero winding.
    #[default]
    NonZero,
    /// Even-odd parity.
    EvenOdd,
}

impl From<FillRuleSpec> for FillRule {
    fn from(rule: FillRuleSpec) -> Self {
        match rule {
            FillRuleSpec::NonZero => Self::NonZero,
            FillRuleSpec::EvenOdd => Self::EvenOdd,
        }
    }
}

/// Open stroke end shape; mirrors [`LineCap`].
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum LineCapSpec {
    /// Flat end at the endpoint.
    #[default]
    Butt,
    /// Semicircular end.
    Round,
    /// Square end extending half the width.
    Square,
}

impl From<LineCapSpec> for LineCap {
    fn from(cap: LineCapSpec) -> Self {
        match cap {
            LineCapSpec::Butt => Self::Butt,
            LineCapSpec::Round => Self::Round,
            LineCapSpec::Square => Self::Square,
        }
    }
}

/// Stroke corner shape; the miter limit is a separate stroke field.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum LineJoinSpec {
    /// Sharp corner, beveled beyond the miter limit.
    #[default]
    Miter,
    /// Circular corner.
    Round,
    /// Straight cut corner.
    Bevel,
}

/// Clip application; mirrors [`ClipMode`].
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum ClipModeSpec {
    /// Multiply the completed subtree by path coverage.
    #[default]
    Coverage,
    /// Intersect geometry.
    Geometric,
}

impl From<ClipModeSpec> for ClipMode {
    fn from(mode: ClipModeSpec) -> Self {
        match mode {
            ClipModeSpec::Coverage => Self::Coverage,
            ClipModeSpec::Geometric => Self::Geometric,
        }
    }
}

/// Mask channel selection; mirrors [`MaskMode`].
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum MaskModeSpec {
    /// Mask alpha.
    #[default]
    Alpha,
    /// One minus mask alpha.
    InvertedAlpha,
    /// Mask luminance.
    Luminance,
    /// One minus mask luminance.
    InvertedLuminance,
}

impl From<MaskModeSpec> for MaskMode {
    fn from(mode: MaskModeSpec) -> Self {
        match mode {
            MaskModeSpec::Alpha => Self::Alpha,
            MaskModeSpec::InvertedAlpha => Self::InvertedAlpha,
            MaskModeSpec::Luminance => Self::Luminance,
            MaskModeSpec::InvertedLuminance => Self::InvertedLuminance,
        }
    }
}

/// Transition travel direction; mirrors [`Direction`].
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum DirectionSpec {
    /// Toward decreasing X.
    #[default]
    Left,
    /// Toward increasing X.
    Right,
    /// Toward decreasing Y.
    Up,
    /// Toward increasing Y.
    Down,
}

impl From<DirectionSpec> for Direction {
    fn from(direction: DirectionSpec) -> Self {
        match direction {
            DirectionSpec::Left => Self::Left,
            DirectionSpec::Right => Self::Right,
            DirectionSpec::Up => Self::Up,
            DirectionSpec::Down => Self::Down,
        }
    }
}

/// Iris reveal geometry; mirrors [`IrisShape`].
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum IrisShapeSpec {
    /// Circle reaching every corner at the end.
    #[default]
    Circle,
    /// Proportionally growing rectangle.
    Rectangle,
}

impl From<IrisShapeSpec> for IrisShape {
    fn from(shape: IrisShapeSpec) -> Self {
        match shape {
            IrisShapeSpec::Circle => Self::Circle,
            IrisShapeSpec::Rectangle => Self::Rectangle,
        }
    }
}

/// Gradient interpolation space; mirrors [`GradientSpace`].
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum GradientSpaceSpec {
    /// Perceptual Oklab.
    Oklab,
    /// Linear-light RGB.
    LinearRgb,
    /// Encoded sRGB.
    Srgb,
}

impl From<GradientSpaceSpec> for GradientSpace {
    fn from(space: GradientSpaceSpec) -> Self {
        match space {
            GradientSpaceSpec::Oklab => Self::Oklab,
            GradientSpaceSpec::LinearRgb => Self::LinearRgb,
            GradientSpaceSpec::Srgb => Self::Srgb,
        }
    }
}

/// Gradient coordinate system; mirrors [`GradientUnits`].
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum GradientUnitsSpec {
    /// Fractions of the painted bounds.
    ObjectBoundingBox,
    /// Local design units.
    UserSpace,
}

impl From<GradientUnitsSpec> for GradientUnits {
    fn from(units: GradientUnitsSpec) -> Self {
        match units {
            GradientUnitsSpec::ObjectBoundingBox => Self::ObjectBoundingBox,
            GradientUnitsSpec::UserSpace => Self::UserSpace,
        }
    }
}

/// Gradient behavior outside its stops; mirrors [`GradientExtend`].
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum GradientExtendSpec {
    /// Clamp to the end stops.
    Pad,
    /// Tile the stops.
    Repeat,
    /// Mirror alternate tiles.
    Reflect,
}

impl From<GradientExtendSpec> for GradientExtend {
    fn from(extend: GradientExtendSpec) -> Self {
        match extend {
            GradientExtendSpec::Pad => Self::Pad,
            GradientExtendSpec::Repeat => Self::Repeat,
            GradientExtendSpec::Reflect => Self::Reflect,
        }
    }
}

/// Oscillator wave shape; mirrors [`Waveform`].
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum WaveformSpec {
    /// Sine wave.
    #[default]
    Sine,
    /// Triangle wave.
    Triangle,
    /// Square wave.
    Square,
    /// Rising sawtooth.
    Sawtooth,
}

impl WaveformSpec {
    /// Reports the default sine wave, which documents omit.
    pub fn is_sine(&self) -> bool {
        matches!(self, Self::Sine)
    }
}

impl From<WaveformSpec> for Waveform {
    fn from(waveform: WaveformSpec) -> Self {
        match waveform {
            WaveformSpec::Sine => Self::Sine,
            WaveformSpec::Triangle => Self::Triangle,
            WaveformSpec::Square => Self::Square,
            WaveformSpec::Sawtooth => Self::Sawtooth,
        }
    }
}

/// Arrow head geometry; mirrors [`motion_core::ArrowHead`].
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum ArrowHeadSpec {
    /// Filled triangle.
    #[default]
    Triangle,
    /// Two open sides.
    Vee,
    /// Shaft only.
    None,
}

impl From<ArrowHeadSpec> for motion_core::ArrowHead {
    fn from(head: ArrowHeadSpec) -> Self {
        match head {
            ArrowHeadSpec::Triangle => Self::Triangle,
            ArrowHeadSpec::Vee => Self::Vee,
            ArrowHeadSpec::None => Self::None,
        }
    }
}

/// Image placement inside its viewport.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum FitSpec {
    /// Stretch each axis.
    Fill,
    /// Preserve aspect ratio inside the viewport.
    #[default]
    Contain,
    /// Preserve aspect ratio and crop.
    Cover,
    /// Keep source pixel dimensions.
    None,
}

/// Image reconstruction filter; mirrors [`Sampling`].
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum SamplingSpec {
    /// Nearest texel.
    Nearest,
    /// Bilinear base level.
    Bilinear,
    /// Bilinear between mip levels.
    #[default]
    Trilinear,
}

impl From<SamplingSpec> for Sampling {
    fn from(sampling: SamplingSpec) -> Self {
        match sampling {
            SamplingSpec::Nearest => Self::Nearest,
            SamplingSpec::Bilinear => Self::Bilinear,
            SamplingSpec::Trilinear => Self::Trilinear,
        }
    }
}

/// Paragraph alignment.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum TextAlignmentSpec {
    /// Paragraph start.
    #[default]
    Start,
    /// Paragraph end.
    End,
    /// Centered lines.
    Center,
    /// Justified nonfinal lines.
    Justified,
}
