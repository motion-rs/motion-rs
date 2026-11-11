//! Explicit scene-linear to display-linear transforms, before output quantization.

pub use motion_core::Cdl;
use motion_core::ValidationError;

/// Linear segments in the tabulated AgX curve.
const AGX_SEGMENTS: usize = 4096;

/// Base work units per pixel for any non-identity transform.
const BASE_WORK: u64 = 1;

/// Work units per pixel for a non-default CDL look.
const LOOK_WORK: u64 = 4;

/// Work units per pixel for a highlight compression curve.
const TONE_MAP_WORK: u64 = 16;

/// Work units per pixel for each 3D LUT lookup.
const LUT_WORK: u64 = 4;

/// Additional work units per pixel for each LUT with a 1D shaper.
const SHAPER_WORK: u64 = 6;

/// Work units per pixel for encoded-domain film grain.
const GRAIN_WORK: u64 = 32;

/// Highlight compression; the default `None` leaves scene values unchanged.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub enum ToneMap {
    /// Leaves extended linear RGB unchanged.
    #[default]
    None,
    /// Khronos PBR Neutral highlight compression.
    PbrNeutral,
    /// ACES fitted RRT/ODT.
    Aces,
    /// AgX log-domain contrast and highlight desaturation.
    AgX,
    /// Uncharted 2 curve normalized at the specified scene-linear white.
    Hable {
        /// Positive scene-linear reference white.
        white: f32,
    },
}

/// Encoding applied to display-linear video before the BT.709 matrix.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum OutputTransfer {
    /// Existing sRGB output, including its linear toe.
    #[default]
    Srgb,
    /// A pure 2.4 display gamma for explicitly selected video delivery.
    Gamma24,
}

/// Deterministic display-encoded grain, addressed by absolute output coordinates.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct FilmGrain {
    /// Explicit sequence seed.
    pub seed: u64,
    /// Midtone noise amplitude.
    pub amount: f32,
    /// Grid spacing in output pixels, at least one.
    pub size: f32,
    /// Exponent of the midtone response, strictly positive.
    pub luma_response: f32,
    /// Zero for monochrome, one for independent channel grain.
    pub chroma: f32,
}

/// Opt-in output transform. Alpha and scene compositing are never changed.
///
/// Fields are listed in processing order: exposure, look, look LUT, tone map with reference
/// white, saturation, display LUT, output transfer and finally encoded-domain grain.
#[derive(Clone, Debug, PartialEq)]
pub struct DisplayTransform {
    /// Finite exposure in stops.
    pub exposure_stops: f32,
    /// Scene-linear grading before highlight compression.
    pub look: Cdl,
    /// Optional look LUT, applied before tone mapping.
    pub look_lut: Option<std::sync::Arc<motion_core::Lut3d>>,
    /// Encoding expected by the look LUT.
    pub lut_input: motion_core::LutInput,
    /// Highlight compression curve.
    pub tone_map: ToneMap,
    /// Optional positive scene reference white: curve(rgb*k)/curve(k).
    pub reference_white: Option<f32>,
    /// Nonnegative display-linear chroma multiplier.
    pub saturation: f32,
    /// Optional print/display LUT in encoded sRGB, after tone mapping.
    pub display_lut: Option<std::sync::Arc<motion_core::Lut3d>>,
    /// Explicit video transfer; RGBA output remains sRGB.
    pub transfer: OutputTransfer,
    /// Optional encoded-domain grain after the output transfer.
    pub grain: Option<FilmGrain>,
}

impl Default for DisplayTransform {
    fn default() -> Self {
        Self {
            exposure_stops: 0.0,
            look: Cdl::default(),
            look_lut: None,
            lut_input: motion_core::LutInput::Linear,
            tone_map: ToneMap::None,
            reference_white: None,
            saturation: 1.0,
            display_lut: None,
            transfer: OutputTransfer::Srgb,
            grain: None,
        }
    }
}

impl DisplayTransform {
    /// Logical arithmetic/sampling units per output pixel; identity adds no work.
    pub(crate) fn work_per_pixel(&self) -> u64 {
        if self.is_identity() {
            return 0;
        }
        BASE_WORK
            + u64::from(self.look != Cdl::default()) * LOOK_WORK
            + u64::from(self.tone_map != ToneMap::None) * TONE_MAP_WORK
            + u64::from(self.look_lut.is_some()) * LUT_WORK
            + u64::from(self.display_lut.is_some()) * LUT_WORK
            + self
                .look_lut
                .iter()
                .chain(&self.display_lut)
                .filter(|lut| lut.shaper().is_some())
                .count() as u64
                * SHAPER_WORK
            + u64::from(self.grain.is_some()) * GRAIN_WORK
    }

    #[cfg(feature = "gpu")]
    pub(crate) fn gpu_words(&self, frame: u64) -> [u32; 32] {
        let mut words = [0; 32];
        let (curve, white) = match self.tone_map {
            ToneMap::None => (0.0, 1.0),
            ToneMap::PbrNeutral => (1.0, 1.0),
            ToneMap::Aces => (2.0, 1.0),
            ToneMap::AgX => (3.0, 1.0),
            ToneMap::Hable { white } => (4.0, white),
        };
        words[..4].copy_from_slice(
            &[self.exposure_stops.exp2(), self.saturation, curve, white].map(f32::to_bits),
        );
        for (index, values) in [self.look.slope, self.look.offset, self.look.power]
            .into_iter()
            .enumerate()
        {
            words[4 + index * 4..7 + index * 4].copy_from_slice(&values.map(f32::to_bits));
        }
        words[7] = self.look.saturation.to_bits();
        words[16] = u32::from(!self.is_identity());
        words[17] = u32::from(self.transfer == OutputTransfer::Gamma24);
        if let Some(grain) = self.grain {
            words[18] = grain.seed as u32;
            words[19] = (grain.seed >> 32) as u32;
            words[22] = grain.amount.to_bits();
            words[23] = grain.size.to_bits();
            words[24] = grain.luma_response.to_bits();
            words[25] = grain.chroma.to_bits();
        }
        words[20] = frame as u32;
        words[21] = (frame >> 32) as u32;
        let (input, minimum, maximum) = match self.lut_input {
            motion_core::LutInput::Linear => (0, 0.0, 1.0),
            motion_core::LutInput::SrgbEncoded => (1, 0.0, 1.0),
            motion_core::LutInput::Log { minimum, maximum } => (2, minimum, maximum),
        };
        words[26] = input;
        let _ = (minimum, maximum);
        let reference = self.reference_white.unwrap_or(1.0);
        let normalization = if self.reference_white.is_some() {
            self.curve([reference; 3])
        } else {
            [1.0; 3]
        };
        words[28..31].copy_from_slice(&normalization.map(f32::to_bits));
        words[31] = reference.to_bits();
        words
    }

    /// Whether all output processing can use the unchanged fast path.
    pub fn is_identity(&self) -> bool {
        *self == Self::default()
    }

    /// Rejects nonfinite and unsupported settings before rendering.
    pub fn validate(&self) -> Result<(), ValidationError> {
        self.lut_input.validate()?;
        let invalid = || {
            ValidationError::invalid(
                "display",
                "parameters",
                "Require finite display parameters within their documented ranges",
            )
        };
        if !self.exposure_stops.is_finite()
            || !self.exposure_stops.exp2().is_finite()
            || self.exposure_stops.exp2() == 0.0
            || !self.saturation.is_finite()
            || self.saturation < 0.0
            || self
                .look
                .slope
                .iter()
                .chain(&self.look.offset)
                .chain(&self.look.power)
                .any(|value| !value.is_finite())
            || self
                .look
                .slope
                .iter()
                .chain(&self.look.power)
                .any(|value| *value < 0.0)
            || !self.look.saturation.is_finite()
            || self.look.saturation < 0.0
        {
            return Err(invalid());
        }
        if let ToneMap::Hable { white } = self.tone_map
            && (!white.is_finite() || white <= 0.0)
        {
            return Err(invalid());
        }
        if let Some(white) = self.reference_white
            && (!white.is_finite()
                || white <= 0.0
                || self
                    .curve([white; 3])
                    .iter()
                    .any(|value| !value.is_finite() || *value <= 0.0))
        {
            return Err(invalid());
        }
        if let Some(grain) = self.grain
            && (!grain.amount.is_finite()
                || grain.amount < 0.0
                || !grain.size.is_finite()
                || grain.size < 1.0
                || !grain.luma_response.is_finite()
                || grain.luma_response <= 0.0
                || !grain.chroma.is_finite()
                || !(0.0..=1.0).contains(&grain.chroma))
        {
            return Err(invalid());
        }
        Ok(())
    }

    /// Transforms straight scene-linear RGB without changing alpha.
    pub fn apply(&self, rgb: [f32; 3]) -> [f32; 3] {
        let gain = if self.exposure_stops == 0.0 {
            1.0
        } else {
            self.exposure_stops.exp2()
        };
        let rgb = self.look.apply(rgb.map(|value| value * gain));
        let rgb = self
            .look_lut
            .as_ref()
            .map_or(rgb, |lut| lut.apply(rgb, self.lut_input));
        let reference = self.reference_white.unwrap_or(1.0);
        let mut rgb = self.curve(rgb.map(|value| value * reference));
        if self.reference_white.is_some() {
            let white = self.curve([reference; 3]);
            rgb = std::array::from_fn(|channel| rgb[channel] / white[channel]);
        }
        let rgb = saturate(rgb, self.saturation);
        self.display_lut.as_ref().map_or(rgb, |lut| {
            lut.apply(rgb, motion_core::LutInput::SrgbEncoded)
        })
    }

    fn curve(&self, rgb: [f32; 3]) -> [f32; 3] {
        match self.tone_map {
            ToneMap::None => rgb,
            ToneMap::PbrNeutral => neutral(rgb),
            ToneMap::Aces => aces(rgb),
            ToneMap::AgX => agx(rgb),
            ToneMap::Hable { white } => rgb.map(|value| hable(value.max(0.0)) / hable(white)),
        }
    }

    /// Adds grain to encoded RGB using an explicit frame number and global pixel origin.
    pub fn encoded_grain(&self, rgb: [f32; 3], x: u32, y: u32, frame: u64) -> [f32; 3] {
        let Some(grain) = self.grain else {
            return rgb;
        };
        let luma = luminance(rgb).clamp(0.0, 1.0);
        let response = 4.0 * luma * (1.0 - luma);
        let response = if grain.luma_response == 1.0 {
            response
        } else {
            response.powf(grain.luma_response)
        };
        let amplitude = grain.amount * response;
        let position = [x as f32 / grain.size, y as f32 / grain.size];
        let base = grain.seed ^ frame;
        let [common, red, green, blue] = motion_core::noise::film_grain_multi(
            position,
            [0, 1, 2, 3].map(|channel: u32| {
                if channel == 0 {
                    base
                } else {
                    base ^ u64::from(channel.wrapping_mul(0x9e3779b9))
                }
            }),
        );
        let independent = [red, green, blue];
        std::array::from_fn(|channel| {
            (rgb[channel] + amplitude * (common + (independent[channel] - common) * grain.chroma))
                .clamp(0.0, 1.0)
        })
    }
}

#[derive(Clone, Copy, Default)]
pub(crate) struct DisplaySettings<'a> {
    pub(crate) display: Option<&'a DisplayTransform>,
    pub(crate) origin: [u32; 2],
    pub(crate) frame: u64,
}

impl DisplaySettings<'_> {
    pub(crate) fn encoded(self, pixel: [f32; 4], x: u32, y: u32, video: bool) -> [f32; 3] {
        self.encoded_cached(pixel, x, y, video, &mut EncodedCache::default())
    }

    pub(crate) fn encoded_cached(
        self,
        pixel: [f32; 4],
        x: u32,
        y: u32,
        video: bool,
        cache: &mut EncodedCache,
    ) -> [f32; 3] {
        let key = pixel.map(f32::to_bits);
        if !cache.valid || cache.key != key {
            *cache = EncodedCache {
                key,
                value: self.graded(pixel, video),
                valid: true,
            };
        }
        let rgb = cache.value;
        self.display.map_or(rgb, |display| {
            display.encoded_grain(rgb, self.origin[0] + x, self.origin[1] + y, self.frame)
        })
    }

    fn graded(self, pixel: [f32; 4], video: bool) -> [f32; 3] {
        let straight = [pixel[0], pixel[1], pixel[2]].map(|value| {
            if pixel[3] > 0.0 {
                value / pixel[3]
            } else {
                0.0
            }
        });
        let linear = self
            .display
            .map_or(straight, |display| display.apply(straight));
        linear.map(|value| {
            let value = value.clamp(0.0, 1.0);
            if video
                && self
                    .display
                    .is_some_and(|display| display.transfer == OutputTransfer::Gamma24)
            {
                value.powf(1.0 / 2.4)
            } else {
                encode_srgb(value)
            }
        })
    }
}

/// Remembers the grain-free result for the previous source pixel; flat regions repeat
/// exactly, and the transform is a pure function of the pixel bits.
#[derive(Default)]
pub(crate) struct EncodedCache {
    key: [u32; 4],
    value: [f32; 3],
    valid: bool,
}

/// sRGB OETF for a value already clamped to 0..=1; the platform `powf` is several times
/// faster than the portable one used by the general-purpose helper.
fn encode_srgb(value: f32) -> f32 {
    if value <= 0.0031308 {
        value * 12.92
    } else {
        1.055 * value.powf(1.0 / 2.4) - 0.055
    }
}

fn luminance(rgb: [f32; 3]) -> f32 {
    rgb[0] * 0.2126 + rgb[1] * 0.7152 + rgb[2] * 0.0722
}

fn saturate(rgb: [f32; 3], saturation: f32) -> [f32; 3] {
    if saturation == 1.0 {
        return rgb;
    }
    let luma = luminance(rgb);
    rgb.map(|value| luma + (value - luma) * saturation)
}

fn neutral(mut rgb: [f32; 3]) -> [f32; 3] {
    rgb = rgb.map(|value| value.max(0.0));
    let minimum = rgb.into_iter().fold(f32::INFINITY, f32::min);
    let offset = if minimum < 0.08 {
        minimum - 6.25 * minimum * minimum
    } else {
        0.04
    };
    rgb = rgb.map(|value| value - offset);
    let peak = rgb.into_iter().fold(0.0, f32::max);
    if peak < 0.76 {
        return rgb;
    }
    let compressed = 1.0 - 0.24 * 0.24 / (peak + 0.24 - 0.76);
    let desaturation = 1.0 - 1.0 / (0.15 * (peak - compressed) + 1.0);
    rgb.map(|value| {
        let value = value * compressed / peak;
        value + (compressed - value) * desaturation
    })
}

fn matrix(rgb: [f32; 3], rows: [[f32; 3]; 3]) -> [f32; 3] {
    rows.map(|row| row[0] * rgb[0] + row[1] * rgb[1] + row[2] * rgb[2])
}

fn aces(rgb: [f32; 3]) -> [f32; 3] {
    let rgb = matrix(
        rgb,
        [
            [0.59719, 0.35458, 0.04823],
            [0.076, 0.90834, 0.01566],
            [0.0284, 0.13383, 0.83777],
        ],
    );
    let fitted = rgb.map(|value| {
        let value = value.max(0.0);
        ((value * (value + 0.0245786) - 0.000090537)
            / (value * (0.983729 * value + 0.432951) + 0.238081))
            .max(0.0)
    });
    matrix(
        fitted,
        [
            [1.60475, -0.53108, -0.07367],
            [-0.10208, 1.10813, -0.00605],
            [-0.00327, -0.07276, 1.07602],
        ],
    )
    .map(|value| value.max(0.0))
}

/// The polynomial and its 2.2 power depend only on the log coordinate, so they are
/// tabulated once; linear interpolation error is below 2e-6.
fn agx_table() -> &'static [f32; AGX_SEGMENTS + 2] {
    static TABLE: std::sync::OnceLock<[f32; AGX_SEGMENTS + 2]> = std::sync::OnceLock::new();
    TABLE.get_or_init(|| {
        std::array::from_fn(|index| {
            let value = index.min(AGX_SEGMENTS) as f64 / AGX_SEGMENTS as f64;
            let squared = value * value;
            (15.5 * squared * squared * squared - 40.14 * squared * squared * value
                + 31.96 * squared * squared
                - 6.868 * squared * value
                + 0.4298 * squared
                + 0.1191 * value
                - 0.00232)
                .clamp(0.0, 1.0)
                .powf(2.2) as f32
        })
    })
}

/// log2 for positive normal floats, accurate to about 1e-7 absolute.
fn fast_log2(value: f32) -> f32 {
    let bits = value.to_bits();
    let mut exponent = ((bits >> 23) & 0xff) as i32 - 127;
    let mut mantissa = f32::from_bits((bits & 0x007f_ffff) | 0x3f80_0000);
    if mantissa > std::f32::consts::SQRT_2 {
        mantissa *= 0.5;
        exponent += 1;
    }
    let s = (mantissa - 1.0) / (mantissa + 1.0);
    let squared = s * s;
    let series = s * (1.0 + squared * (1.0 / 3.0 + squared * (0.2 + squared * (1.0 / 7.0))));
    exponent as f32 + series * (2.0 * std::f32::consts::LOG2_E)
}

fn agx_channel(value: f32) -> f32 {
    let coordinate = ((fast_log2(value.max(1e-10)) + 12.47393) / 16.5).clamp(0.0, 1.0);
    let scaled = coordinate * AGX_SEGMENTS as f32;
    let index = scaled as usize;
    let fraction = scaled - index as f32;
    let table = agx_table();
    table[index] + (table[index + 1] - table[index]) * fraction
}

fn agx(rgb: [f32; 3]) -> [f32; 3] {
    let rgb = matrix(
        rgb,
        [
            [0.842479, 0.0784336, 0.079224],
            [0.042328, 0.878468, 0.079166],
            [0.042375, 0.0784336, 0.879143],
        ],
    );
    let encoded = rgb.map(agx_channel);
    matrix(
        encoded,
        [
            [1.196879, -0.0980209, -0.0990297],
            [-0.0528969, 1.151903, -0.0989612],
            [-0.0529716, -0.0980435, 1.151074],
        ],
    )
    .map(|value| value.max(0.0))
}

fn hable(value: f32) -> f32 {
    (value * (0.15 * value + 0.05) + 0.004) / (value * (0.15 * value + 0.5) + 0.06) - 0.004 / 0.06
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tabulated_agx_matches_the_direct_curve_and_multi_grain_matches_single_calls() {
        let mut worst = 0.0_f32;
        for step in 0..=200_000 {
            // Log-spaced from far below black to well above any HDR highlight.
            let value = (step as f32 / 200_000.0 * 40.0 - 24.0).exp2();
            let t = ((f64::from(value).max(1e-10).log2() + 12.47393) / 16.5).clamp(0.0, 1.0);
            let squared = t * t;
            let exact = (15.5 * squared * squared * squared - 40.14 * squared * squared * t
                + 31.96 * squared * squared
                - 6.868 * squared * t
                + 0.4298 * squared
                + 0.1191 * t
                - 0.00232)
                .clamp(0.0, 1.0)
                .powf(2.2);
            worst = worst.max((f64::from(agx_channel(value)) - exact).abs() as f32);
        }
        assert!(worst < 2e-6, "{worst}");
        for value in [f32::INFINITY, f32::NAN, 0.0, -1.0, f32::MAX] {
            assert!(agx_channel(value).is_finite());
        }
        for seed in [0, 7, u64::MAX] {
            let position = [12.3, 45.6];
            let seeds = [seed, seed ^ 1, seed ^ 2, seed ^ 3];
            let multi = motion_core::noise::film_grain_multi(position, seeds);
            for (value, seed) in multi.into_iter().zip(seeds) {
                assert_eq!(
                    value.to_bits(),
                    motion_core::noise::film_grain(position, seed).to_bits()
                );
            }
        }
    }

    #[test]
    fn film_grain_samples_the_shared_n0_lattice_hash() {
        let uniform = |x: i32, y: i32, seed: u32| {
            (motion_core::noise::hash2(x, y, seed) >> 8) as f32 / 16777216.0
        };
        for seed in [0, 42, u64::MAX, 0xdead_beef_1234_5678] {
            let folded = seed as u32 ^ (seed >> 32) as u32;
            for (x, y) in [(0, 0), (3, 7), (1919, 1079)] {
                let expected =
                    (uniform(x, y, folded) + uniform(x, y, folded ^ 0xa511e9b3) - 1.0) * 2.4494898;
                let actual = motion_core::noise::film_grain([x as f32, y as f32], seed);
                assert_eq!(actual.to_bits(), expected.to_bits());
            }
        }
    }

    #[test]
    fn curves_are_finite_black_and_neutral_monotone() {
        for tone_map in [
            ToneMap::None,
            ToneMap::PbrNeutral,
            ToneMap::Aces,
            ToneMap::AgX,
            ToneMap::Hable { white: 11.2 },
        ] {
            let display = DisplayTransform {
                tone_map,
                ..DisplayTransform::default()
            };
            let mut previous = [0.0; 3];
            for step in 0..=4096 {
                let value = step as f32 / 256.0;
                let actual = display.apply([value; 3]);
                assert!(
                    actual
                        .iter()
                        .all(|value| value.is_finite() && *value >= 0.0)
                );
                for channel in 0..3 {
                    assert!(
                        actual[channel] + 1e-5 >= previous[channel],
                        "{tone_map:?}: {value}"
                    );
                }
                previous = actual;
            }
            assert!(
                display
                    .apply([0.0; 3])
                    .iter()
                    .all(|value| value.abs() <= 1e-5)
            );
        }
    }

    #[test]
    fn reference_white_maps_to_one_and_default_preserves_hdr() {
        let input = [-0.02, 0.18, 16.0];
        assert_eq!(DisplayTransform::default().apply(input), input);
        for tone_map in [ToneMap::Aces, ToneMap::AgX] {
            let display = DisplayTransform {
                tone_map,
                reference_white: Some(1.0),
                ..DisplayTransform::default()
            };
            assert_eq!(display.apply([1.0; 3]), [1.0; 3]);
            assert!(
                DisplayTransform {
                    reference_white: Some(0.0),
                    ..display
                }
                .validate()
                .is_err()
            );
        }
    }

    #[test]
    fn grain_is_repeatable_frame_variant_and_dark_white_silent() {
        let display = DisplayTransform {
            grain: Some(FilmGrain {
                seed: 42,
                amount: 0.02,
                size: 1.5,
                luma_response: 1.0,
                chroma: 0.2,
            }),
            ..DisplayTransform::default()
        };
        let first = display.encoded_grain([0.5; 3], 23, 41, 7);
        assert_eq!(first, display.encoded_grain([0.5; 3], 23, 41, 7));
        assert_ne!(first, display.encoded_grain([0.5; 3], 23, 41, 8));
        assert_eq!(display.encoded_grain([0.0; 3], 0, 0, 0), [0.0; 3]);
        assert_eq!(display.encoded_grain([1.0; 3], 0, 0, 0), [1.0; 3]);
    }

    #[test]
    fn output_identity_alpha_and_display_budget() {
        let frame = crate::cpu::LinearFrame::constant(
            motion_core::PixelSize::new(8, 8),
            [0.5, 0.25, 2.0, 0.5],
        );
        let limits = motion_core::RasterLimits::default();
        let format = crate::PixelFormat::SrgbRgba8 {
            premultiplied: false,
        };
        let identity = crate::FrameBuffer::from_linear(&frame, format, limits).unwrap();
        let transformed = crate::FrameBuffer::from_linear_display(
            &frame,
            format,
            limits,
            &DisplayTransform::default(),
            0,
        )
        .unwrap();
        assert_eq!(identity, transformed);
        let display = DisplayTransform {
            tone_map: ToneMap::AgX,
            ..DisplayTransform::default()
        };
        let transformed =
            crate::FrameBuffer::from_linear_display(&frame, format, limits, &display, 0).unwrap();
        assert_eq!(
            identity.planes()[0].bytes()[3],
            transformed.planes()[0].bytes()[3]
        );
        assert!(
            crate::FrameBuffer::from_linear_display(
                &frame,
                format,
                motion_core::RasterLimits {
                    max_work: 64,
                    ..limits
                },
                &display,
                0
            )
            .is_err()
        );
    }
}
