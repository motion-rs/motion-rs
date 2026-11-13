//! Linear-light filter data, validation and conservative device bounds.

use crate::drawing::BlendMode;
use crate::{Color, Rect, Transform, ValidationError, Vector2};

/// Ordered, source-only effects on a completed subtree.
///
/// Distances use local design units. Blur is an isotropic Gaussian in device
/// space, scaled by the largest singular value of the composed transform (also
/// under shear/nonuniform scale). Blur uses three normalized box passes with
/// conservative ceil(3*sigma) guard bounds; small sigma uses fractional edge
/// weights rather than silently becoming an identity operation.
/// Offsets transform as vectors. Filters do not change layout or pivot bounds.
#[derive(Clone, Debug, PartialEq)]
pub enum Filter {
    /// Convolves premultiplied RGBA with transparent-black boundary conditions.
    Blur {
        /// Standard deviation; finite and nonnegative.
        sigma: f64,
    },
    /// Places a colored blurred alpha silhouette behind the original source.
    DropShadow {
        /// Local displacement.
        offset: Vector2,
        /// Nonnegative blur standard deviation.
        sigma: f64,
        /// Straight encoded-sRGB shadow color.
        color: Color,
    },
    /// Adds blurred Rec.709 highlight energy to the original source.
    Glow {
        /// Nonnegative straight linear luminance cutoff.
        threshold: f64,
        /// Nonnegative blur standard deviation.
        sigma: f64,
        /// Nonnegative premultiplied contribution multiplier.
        intensity: f64,
    },
    /// Normalized multi-scale highlight bloom with transparent-black boundaries.
    ///
    /// Level `k` blurs the highlights downsampled `2^k` times by `radius` level pixels, which
    /// spans `radius * 2^k` device pixels; levels are upsampled and summed coarse to fine.
    Bloom {
        /// Nonnegative straight linear luminance threshold.
        threshold: f64,
        /// Nonnegative soft-knee half width.
        knee: f64,
        /// Base local standard deviation.
        radius: f64,
        /// Number of pyramid levels, in 1..=8.
        levels: u8,
        /// Nonnegative relative weight decay per octave.
        falloff: f64,
        /// Nonnegative added-energy multiplier.
        intensity: f64,
        /// Encoded-sRGB bloom tint.
        tint: Color,
    },
    /// Straight-linear slope/offset/power grading on a subtree.
    Cdl(crate::Cdl),
    /// Immutable three-dimensional color look on a subtree.
    Lut {
        /// Validated shared cube data.
        lut: std::sync::Arc<crate::Lut3d>,
        /// Input/output encoding around interpolation.
        input: crate::LutInput,
    },
    /// Lens-space resampling, aperture blur or height-field lighting.
    Optics(crate::Optics),
    /// Straight linear RGB: exposure, contrast about 0.18, Rec.709 saturation,
    /// then gray-axis hue rotation. Alpha is unchanged; extended RGB is retained.
    ColorAdjust {
        /// Finite exposure in stops, with representable positive gain.
        exposure: f64,
        /// Finite nonnegative contrast multiplier.
        contrast: f64,
        /// Finite nonnegative saturation multiplier.
        saturation: f64,
        /// Finite wrapped angle in degrees.
        hue_degrees: f64,
    },
    /// Applies primitives in order. CompositeWithSource always refers to this
    /// filter's input, not to the result of the preceding primitive.
    Custom(Vec<FilterOperation>),
}

/// Backend-independent primitives for a custom filter.
#[derive(Clone, Debug, PartialEq)]
pub enum FilterOperation {
    /// Transparent-border affine resampling, with local coordinates.
    Transform(Transform),
    /// Axis-aligned Gaussian standard deviations in local design units.
    BlurXY {
        /// Horizontal standard deviation.
        sigma_x: f64,
        /// Vertical standard deviation.
        sigma_y: f64,
    },
    /// Gaussian blur of premultiplied RGBA.
    Blur {
        /// Nonnegative local standard deviation.
        sigma: f64,
    },
    /// Transparent-border bilinear displacement.
    Offset {
        /// Signed local horizontal displacement.
        x: f64,
        /// Signed local vertical displacement.
        y: f64,
    },
    /// Row-major 4x5 matrix applied to straight linear RGBA and a constant one.
    /// Alpha alone clamps to 0..=1, followed by premultiplication; zero alpha
    /// always has zero RGB. Matrix biases act only within current effect bounds.
    ColorMatrix([f64; 20]),
    /// Keeps the fraction max(luminance-value,0)/luminance of premultiplied RGBA.
    Threshold {
        /// Finite nonnegative straight linear luminance cutoff.
        value: f64,
    },
    /// Deterministic alpha-preserving monochrome noise.
    Noise {
        /// Explicit seed.
        seed: u64,
        /// Nonnegative amplitude.
        amplitude: f64,
    },
    /// Bilinear channel displacements; output alpha is the maximum sampled alpha.
    ChannelOffset {
        /// Red displacement.
        red: Vector2,
        /// Green displacement.
        green: Vector2,
        /// Blue displacement.
        blue: Vector2,
    },
    /// Composites current result over the saved filter input using a mode.
    CompositeWithSource {
        /// Linear-light layer blend mode.
        mode: BlendMode,
    },
}

fn invalid(path: &str, message: &str) -> ValidationError {
    ValidationError::invalid(path, "filters", message)
}

fn finite(value: f64, path: &str) -> Result<(), ValidationError> {
    if value.is_finite() {
        Ok(())
    } else {
        Err(invalid(path, "Filter parameters must be finite"))
    }
}

fn nonnegative(value: f64, path: &str) -> Result<(), ValidationError> {
    finite(value, path)?;
    if value >= 0.0 {
        Ok(())
    } else {
        Err(invalid(path, "Filter parameter must be nonnegative"))
    }
}

fn sigma(value: f64, path: &str) -> Result<(), ValidationError> {
    nonnegative(value, path)?;
    finite(value * 3.0, path)
}

/// Uses one variance-matched box for Draft; Final retains the exact three-pass policy.
pub fn quality_box_radii(
    sigma: f64,
    quality: crate::RenderQuality,
) -> Result<[u32; 3], ValidationError> {
    let radii = box_radii(sigma)?;
    if quality == crate::RenderQuality::Final || sigma < 1.0 {
        return Ok(radii);
    }
    Ok([0, 0, (libm::sqrt(3.0) * sigma).round() as u32])
}

/// Chooses three box radii approximating a previously validated nonnegative blur sigma.
/// Rejects support outside the index range; small positive sigmas retain a fractional pass.
pub fn box_radii(sigma: f64) -> Result<[u32; 3], ValidationError> {
    let ideal = libm::sqrt(4.0 * sigma * sigma + 1.0);
    if !ideal.is_finite() || ideal > (u32::MAX - 2) as f64 {
        return Err(invalid(
            "filter.blur",
            "Box blur support exceeds the supported index range",
        ));
    }
    let mut lower = libm::floor(ideal).max(1.0) as u32;
    if lower.is_multiple_of(2) {
        lower -= 1;
    }
    let width = f64::from(lower);
    let count = ((12.0 * sigma * sigma - 3.0 * width * width - 12.0 * width - 9.0)
        / (-4.0 * width - 4.0))
        .round()
        .clamp(0.0, 3.0) as usize;
    if lower == 1 && count == 3 && sigma > 0.0 {
        return Ok([0, 0, 1]);
    }
    Ok(std::array::from_fn(|index| {
        if index < count {
            (lower - 1) / 2
        } else {
            lower.div_ceil(2)
        }
    }))
}

/// Weight for a tap in `0..2 * radius + 1`, including the fractional small-sigma kernel.
pub fn box_weight(sigma: f64, radius: usize, index: usize) -> f64 {
    if sigma * sigma <= 1.0 / 3.0 && radius == 1 {
        if index == 1 {
            1.0 - sigma * sigma
        } else {
            sigma * sigma * 0.5
        }
    } else {
        1.0 / (radius * 2 + 1) as f64
    }
}

fn vector(value: Vector2, path: &str) -> Result<(), ValidationError> {
    finite(value.x, path)?;
    finite(value.y, path)
}

/// Encloses both filter regions without discarding zero-area endpoints.
pub fn union(first: Rect, second: Rect) -> Rect {
    Rect::new(
        first.left.min(second.left),
        first.top.min(second.top),
        first.right.max(second.right),
        first.bottom.max(second.bottom),
    )
}

/// Rounds every side outward to integral device coordinates for filter support.
pub fn raster_bounds(bounds: Rect) -> Rect {
    Rect::new(
        bounds.left.floor(),
        bounds.top.floor(),
        bounds.right.ceil(),
        bounds.bottom.ceil(),
    )
}

/// Maps a device region to the next coarser Bloom pyramid level, where pixel `(x, y)` averages
/// the 2x2 block starting at `(2x, 2y)` of the finer level. Sides round outward.
pub fn bloom_down(bounds: Rect) -> Rect {
    Rect::new(
        (bounds.left * 0.5).floor(),
        (bounds.top * 0.5).floor(),
        (bounds.right * 0.5).ceil(),
        (bounds.bottom * 0.5).ceil(),
    )
}

/// Region of the finer level reached when a coarser region is upsampled bilinearly at
/// `((x + 0.5) / 2 - 0.5, (y + 0.5) / 2 - 0.5)`: one finer pixel past each doubled side.
pub fn bloom_up(bounds: Rect) -> Rect {
    Rect::new(
        bounds.left * 2.0 - 1.0,
        bounds.top * 2.0 - 1.0,
        bounds.right * 2.0 + 1.0,
        bounds.bottom * 2.0 + 1.0,
    )
}

/// Output region of a Bloom pyramid over `source`: each level is downsampled from the previous
/// one, blurred by `radius` level pixels and upsampled into the next finer level.
pub fn bloom_bounds(source: Rect, radius: f64, levels: u8) -> Rect {
    let levels = usize::from(levels.clamp(1, 8));
    let mut domains = [raster_bounds(source); 8];
    for level in 1..levels {
        domains[level] = bloom_down(domains[level - 1]);
    }
    let mut accumulated = blur(domains[levels - 1], radius);
    for domain in domains[..levels - 1].iter().rev() {
        accumulated = union(bloom_up(accumulated), blur(*domain, radius));
    }

    union(domains[0], accumulated)
}

fn offset(bounds: Rect, x: f64, y: f64) -> Rect {
    Rect::new(
        bounds.left + x,
        bounds.top + y,
        bounds.right + x,
        bounds.bottom + y,
    )
}

fn blur(bounds: Rect, sigma: f64) -> Rect {
    let radius = (3.0 * sigma).ceil();
    Rect::new(
        bounds.left - radius,
        bounds.top - radius,
        bounds.right + radius,
        bounds.bottom + radius,
    )
}

impl FilterOperation {
    /// Validates finite parameters and operation-specific ranges at a diagnostic path.
    pub fn validate(&self, path: &str) -> Result<(), ValidationError> {
        match self {
            Self::Transform(transform) => {
                transform
                    .coefficients()
                    .iter()
                    .try_for_each(|value| finite(*value, path))?;
                if transform.inverse().is_none() {
                    return Err(invalid(path, "Filter transform must be invertible"));
                }
                Ok(())
            }
            Self::BlurXY { sigma_x, sigma_y } => {
                sigma(*sigma_x, path)?;
                sigma(*sigma_y, path)
            }
            Self::Blur { sigma: value } => sigma(*value, path),
            Self::Offset { x, y } => {
                finite(*x, path)?;
                finite(*y, path)
            }
            Self::ColorMatrix(matrix) => matrix.iter().try_for_each(|value| finite(*value, path)),
            Self::Threshold { value } => nonnegative(*value, path),
            Self::Noise { amplitude, .. } => nonnegative(*amplitude, path),
            Self::ChannelOffset { red, green, blue } => {
                vector(*red, path)?;
                vector(*green, path)?;
                vector(*blue, path)
            }
            Self::CompositeWithSource { .. } => Ok(()),
        }
    }

    /// Propagates device-space output support through this validated primitive.
    /// `source` is the original saved input used by source-compositing operations.
    pub fn bounds(&self, bounds: Rect, source: Rect) -> Rect {
        let bounds = raster_bounds(bounds);
        raster_bounds(match self {
            Self::Transform(transform) => {
                let points = [
                    crate::Point::new(bounds.left, bounds.top),
                    crate::Point::new(bounds.right, bounds.top),
                    crate::Point::new(bounds.right, bounds.bottom),
                    crate::Point::new(bounds.left, bounds.bottom),
                ]
                .map(|point| transform.apply(point));
                Rect::new(
                    points
                        .iter()
                        .map(|point| point.x)
                        .fold(f64::INFINITY, f64::min),
                    points
                        .iter()
                        .map(|point| point.y)
                        .fold(f64::INFINITY, f64::min),
                    points
                        .iter()
                        .map(|point| point.x)
                        .fold(f64::NEG_INFINITY, f64::max),
                    points
                        .iter()
                        .map(|point| point.y)
                        .fold(f64::NEG_INFINITY, f64::max),
                )
            }
            Self::BlurXY { sigma_x, sigma_y } => Rect::new(
                bounds.left - (sigma_x * 3.0).ceil(),
                bounds.top - (sigma_y * 3.0).ceil(),
                bounds.right + (sigma_x * 3.0).ceil(),
                bounds.bottom + (sigma_y * 3.0).ceil(),
            ),
            Self::Blur { sigma } => blur(bounds, *sigma),
            Self::Offset { x, y } => offset(bounds, *x, *y),
            Self::ChannelOffset { red, green, blue } => union(
                union(
                    offset(bounds, red.x, red.y),
                    offset(bounds, green.x, green.y),
                ),
                offset(bounds, blue.x, blue.y),
            ),
            Self::CompositeWithSource { .. } => union(bounds, source),
            _ => bounds,
        })
    }
}

impl Filter {
    /// Horizontal anamorphic highlight streak using normalized axis blur.
    pub fn anamorphic(
        intensity: f64,
        sigma_x: f64,
        sigma_y: f64,
        tint: Color,
    ) -> Result<Self, ValidationError> {
        nonnegative(intensity, "anamorphic")?;
        sigma(sigma_x, "anamorphic")?;
        sigma(sigma_y, "anamorphic")?;
        let tint = tint.to_linear_premultiplied();
        let mut matrix = [0.0; 20];
        matrix[0] = tint[0] * intensity;
        matrix[6] = tint[1] * intensity;
        matrix[12] = tint[2] * intensity;
        matrix[18] = 1.0;
        Ok(Self::Custom(vec![
            FilterOperation::Threshold { value: 0.9 },
            FilterOperation::BlurXY { sigma_x, sigma_y },
            FilterOperation::ColorMatrix(matrix),
            FilterOperation::CompositeWithSource {
                mode: BlendMode::Plus,
            },
        ]))
    }

    /// Seeded zero-mean monochrome scene-linear noise, preserving alpha.
    /// Each output pixel hashes its position relative to the filtered frame, so the
    /// pattern is stable under unrelated scene changes; vary `seed` per frame for animation.
    /// This is scene-space noise; display-domain film grain is `FilmGrain`.
    pub fn grain(seed: u64, amplitude: f64) -> Self {
        Self::Custom(vec![FilterOperation::Noise { seed, amplitude }])
    }

    /// Red-orange highlight halation composed from existing primitives.
    pub fn halation(strength: f64, radius: f64) -> Result<Self, ValidationError> {
        nonnegative(strength, "halation")?;
        sigma(radius, "halation")?;
        Ok(Self::Custom(vec![
            FilterOperation::Threshold { value: 0.9 },
            FilterOperation::Blur { sigma: radius },
            FilterOperation::ColorMatrix([
                0.9 * strength,
                0.0,
                0.0,
                0.0,
                0.0,
                0.0,
                0.25 * strength,
                0.0,
                0.0,
                0.0,
                0.0,
                0.0,
                0.08 * strength,
                0.0,
                0.0,
                0.0,
                0.0,
                0.0,
                1.0,
                0.0,
            ]),
            FilterOperation::CompositeWithSource {
                mode: BlendMode::Plus,
            },
        ]))
    }

    /// Checks parameters and every custom primitive, including invisible effects.
    pub fn validate(&self, path: &str) -> Result<(), ValidationError> {
        match self {
            Self::Blur { sigma: value } => sigma(*value, path),
            Self::Cdl(cdl) => cdl.validate(),
            Self::Optics(optics) => optics.validate(),
            Self::Lut { input, .. } => input.validate(),
            Self::Bloom {
                threshold,
                knee,
                radius,
                levels,
                falloff,
                intensity,
                ..
            } => {
                if !(1..=8).contains(levels) {
                    return Err(invalid(path, "Bloom levels must be in 1..=8"));
                }
                nonnegative(*threshold, path)?;
                nonnegative(*knee, path)?;
                nonnegative(*falloff, path)?;
                finite(libm::pow(*falloff, f64::from(*levels - 1)), path)?;
                nonnegative(*intensity, path)?;
                sigma(*radius * f64::from(1u32 << (*levels - 1)), path)
            }
            Self::DropShadow {
                offset,
                sigma: value,
                ..
            } => {
                vector(*offset, path)?;
                sigma(*value, path)
            }
            Self::Glow {
                threshold,
                sigma: value,
                intensity,
            } => {
                nonnegative(*threshold, path)?;
                sigma(*value, path)?;
                nonnegative(*intensity, path)
            }
            Self::ColorAdjust {
                exposure,
                contrast,
                saturation,
                hue_degrees,
            } => {
                finite(*exposure, path)?;
                let gain = libm::exp2(*exposure);
                if !gain.is_finite() || gain == 0.0 {
                    return Err(invalid(path, "Exposure gain must be finite and positive"));
                }
                nonnegative(*contrast, path)?;
                nonnegative(*saturation, path)?;
                finite(*hue_degrees, path)
            }
            Self::Custom(operations) => {
                operations
                    .iter()
                    .enumerate()
                    .try_for_each(|(index, operation)| {
                        operation.validate(&format!("{path}.operations[{index}]"))
                    })
            }
        }
    }

    /// Structural charge: one filter plus each explicitly stored custom primitive.
    pub fn structural_cost(&self) -> usize {
        1 + match self {
            Self::Custom(operations) => operations.len(),
            _ => 0,
        }
    }

    /// Retained custom-primitive payload, excluding vector capacity and allocation overhead.
    pub fn heap_bytes(&self) -> u64 {
        match self {
            Self::Lut { lut, .. } => lut.data().len() as u64 * 12,
            Self::Custom(operations) => {
                (operations.len() * std::mem::size_of::<FilterOperation>()) as u64
            }
            _ => 0,
        }
    }

    /// Maps local distances and offset vectors to device coordinates and validates the result.
    /// Isotropic blur uses the transform's largest singular value, including nonuniform scales.
    pub fn device(&self, transform: Transform, path: &str) -> Result<Self, ValidationError> {
        self.validate(path)?;
        let [xx, yx, xy, yy, _, _] = transform.coefficients();
        let scale = (libm::hypot(xx + yy, yx - xy) * 0.5) + (libm::hypot(xx - yy, yx + xy) * 0.5);
        let mapped =
            |value: Vector2| Vector2::new(xx * value.x + xy * value.y, yx * value.x + yy * value.y);
        let inverse = if matches!(self, Self::Custom(operations) if operations.iter().any(|operation| matches!(operation, FilterOperation::Transform(_))))
        {
            transform
                .inverse()
                .ok_or_else(|| invalid(path, "Filter mapping must be invertible"))?
        } else {
            Transform::IDENTITY
        };
        let result = match self {
            Self::Optics(optics) => Self::Optics(optics.device(transform, scale)),
            Self::Blur { sigma } => Self::Blur {
                sigma: sigma * scale,
            },
            Self::Bloom {
                threshold,
                knee,
                radius,
                levels,
                falloff,
                intensity,
                tint,
            } => Self::Bloom {
                threshold: *threshold,
                knee: *knee,
                radius: radius * scale,
                levels: *levels,
                falloff: *falloff,
                intensity: *intensity,
                tint: *tint,
            },
            Self::DropShadow {
                offset,
                sigma,
                color,
            } => Self::DropShadow {
                offset: mapped(*offset),
                sigma: sigma * scale,
                color: *color,
            },
            Self::Glow {
                threshold,
                sigma,
                intensity,
            } => Self::Glow {
                threshold: *threshold,
                sigma: sigma * scale,
                intensity: *intensity,
            },
            Self::Custom(operations) => Self::Custom(
                operations
                    .iter()
                    .map(|operation| match operation {
                        FilterOperation::BlurXY { sigma_x, sigma_y } => FilterOperation::BlurXY {
                            sigma_x: sigma_x * scale,
                            sigma_y: sigma_y * scale,
                        },
                        FilterOperation::Transform(local) => {
                            FilterOperation::Transform(inverse.then(*local).then(transform))
                        }
                        FilterOperation::Blur { sigma } => FilterOperation::Blur {
                            sigma: sigma * scale,
                        },
                        FilterOperation::Offset { x, y } => {
                            let value = mapped(Vector2::new(*x, *y));
                            FilterOperation::Offset {
                                x: value.x,
                                y: value.y,
                            }
                        }
                        FilterOperation::ChannelOffset { red, green, blue } => {
                            FilterOperation::ChannelOffset {
                                red: mapped(*red),
                                green: mapped(*green),
                                blue: mapped(*blue),
                            }
                        }
                        _ => operation.clone(),
                    })
                    .collect(),
            ),
            _ => self.clone(),
        };
        result.validate(path)?;
        Ok(result)
    }

    /// Returns conservative device-space `(capture, output)` regions for validated parameters.
    /// Custom chains retain intermediate support; neither region changes logical layout bounds.
    pub fn extent(&self, source: Rect, path: &str) -> Result<(Rect, Rect), ValidationError> {
        let source = raster_bounds(source);
        let mut capture = source;
        let output = match self {
            Self::Optics(optics) => optics.bounds(source),
            Self::Blur { sigma } => blur(source, *sigma),
            Self::Bloom { radius, levels, .. } => bloom_bounds(source, *radius, *levels),
            Self::DropShadow {
                offset: shift,
                sigma,
                ..
            } => {
                let blurred = blur(source, *sigma);
                capture = union(capture, blurred);

                union(source, raster_bounds(offset(blurred, shift.x, shift.y)))
            }
            Self::Glow { sigma, .. } => blur(source, *sigma),
            Self::Custom(operations) => {
                let mut current = source;
                for operation in operations {
                    current = operation.bounds(current, source);
                    crate::drawing::validate_rect(current, path, "filters")?;
                    capture = union(capture, current);
                }
                current
            }
            _ => source,
        };
        let output = raster_bounds(output);
        capture = union(capture, output);
        crate::drawing::validate_rect(capture, path, "filters")?;
        Ok((output, capture))
    }
}
