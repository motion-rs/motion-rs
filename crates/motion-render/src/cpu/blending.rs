//! Linear-premultiplied isolated-layer blending without output quantization.

use super::blend;
use motion_core::ValidationError;
use motion_core::drawing::BlendMode;

fn hard_light(backdrop: f64, source: f64) -> f64 {
    if source <= 0.5 {
        2.0 * backdrop * source
    } else {
        1.0 - 2.0 * (1.0 - backdrop) * (1.0 - source)
    }
}

pub(crate) fn covered(
    destination: &mut [f32; 4],
    source: [f32; 4],
    coverage: f32,
    space: motion_core::drawing::BlendSpace,
    path: &str,
) -> Result<(), ValidationError> {
    if space == motion_core::drawing::BlendSpace::LinearRgb {
        return blend(destination, source, coverage, path);
    }
    composite_in_space(
        destination,
        source.map(|value| value * coverage),
        BlendMode::SourceOver,
        space,
        path,
    )
}

pub(crate) fn composite_in_space(
    destination: &mut [f32; 4],
    source: [f32; 4],
    mode: BlendMode,
    space: motion_core::drawing::BlendSpace,
    path: &str,
) -> Result<(), ValidationError> {
    if source == [0.0; 4] {
        return Ok(());
    }
    if space == motion_core::drawing::BlendSpace::LinearRgb || mode == BlendMode::LinearPlus {
        return composite(destination, source, mode, path);
    }
    let encode = |pixel: [f32; 4]| -> [f32; 4] {
        if pixel[3] == 0.0 {
            return [0.0; 4];
        }
        [0, 1, 2, 3].map(|channel| {
            if channel == 3 {
                pixel[3]
            } else {
                (motion_core::color::linear_to_srgb(
                    f64::from(pixel[channel]) / f64::from(pixel[3]),
                ) * f64::from(pixel[3])) as f32
            }
        })
    };
    let mut result = encode(*destination);
    composite(&mut result, encode(source), mode, path)?;
    let alpha = f64::from(result[3]);
    let decoded = if alpha == 0.0 {
        [0.0; 4]
    } else {
        [0, 1, 2, 3].map(|channel| {
            if channel == 3 {
                result[3]
            } else {
                (motion_core::color::srgb_to_linear(f64::from(result[channel]) / alpha) * alpha)
                    as f32
            }
        })
    };
    if decoded.iter().any(|value| !value.is_finite()) {
        return Err(ValidationError::invalid(
            path,
            "color",
            "Encoded-space blending overflowed working storage",
        ));
    }
    *destination = decoded;
    Ok(())
}

/// `composite_in_space(.., BlendMode::Plus, LinearRgb)` without mode dispatch: premultiplied
/// channels add in f64 and alpha saturates at one, bit for bit as the generic path.
#[inline]
pub(crate) fn plus_linear(
    destination: &mut [f32; 4],
    source: [f32; 4],
    path: &str,
) -> Result<(), ValidationError> {
    if source == [0.0; 4] {
        return Ok(());
    }
    let result = [
        (f64::from(source[0]) + f64::from(destination[0])) as f32,
        (f64::from(source[1]) + f64::from(destination[1])) as f32,
        (f64::from(source[2]) + f64::from(destination[2])) as f32,
        (f64::from(source[3]) + f64::from(destination[3])).min(1.0) as f32,
    ];
    if !(result[0].is_finite()
        & result[1].is_finite()
        & result[2].is_finite()
        & result[3].is_finite())
    {
        return Err(ValidationError::invalid(
            path,
            "color",
            "layer blending overflowed finite f32 working storage",
        ));
    }
    *destination = result;
    Ok(())
}

fn luminosity(color: [f64; 3]) -> f64 {
    color[0] * 0.3 + color[1] * 0.59 + color[2] * 0.11
}

fn saturation(color: [f64; 3]) -> f64 {
    color.into_iter().fold(f64::NEG_INFINITY, f64::max)
        - color.into_iter().fold(f64::INFINITY, f64::min)
}

fn set_saturation(color: [f64; 3], value: f64) -> [f64; 3] {
    let minimum = color.into_iter().fold(f64::INFINITY, f64::min);
    let range = saturation(color);
    if range == 0.0 {
        [0.0; 3]
    } else {
        color.map(|channel| (channel - minimum) / range * value)
    }
}

fn set_luminosity(color: [f64; 3], value: f64) -> [f64; 3] {
    let delta = value - luminosity(color);
    let mut color = color.map(|channel| channel + delta);
    if !(0.0..=1.0).contains(&value) {
        return color;
    }
    let minimum = color.into_iter().fold(f64::INFINITY, f64::min);
    let maximum = color.into_iter().fold(f64::NEG_INFINITY, f64::max);
    if minimum < 0.0 && value != minimum {
        color = color.map(|channel| value + (channel - value) * value / (value - minimum));
    }
    if maximum > 1.0 && maximum != value {
        color = color.map(|channel| value + (channel - value) * (1.0 - value) / (maximum - value));
    }
    color
}

fn nonseparable(backdrop: [f64; 3], source: [f64; 3], mode: BlendMode) -> [f64; 3] {
    match mode {
        BlendMode::Hue => set_luminosity(
            set_saturation(source, saturation(backdrop)),
            luminosity(backdrop),
        ),
        BlendMode::Saturation => set_luminosity(
            set_saturation(backdrop, saturation(source)),
            luminosity(backdrop),
        ),
        BlendMode::Color => set_luminosity(source, luminosity(backdrop)),
        BlendMode::Luminosity => set_luminosity(backdrop, luminosity(source)),
        _ => unreachable!("nonseparable mode"),
    }
}

pub(crate) fn composite(
    destination: &mut [f32; 4],
    source: [f32; 4],
    mode: BlendMode,
    path: &str,
) -> Result<(), ValidationError> {
    composite_mode(destination, source, mode, path)
}

fn composite_mode(
    destination: &mut [f32; 4],
    source: [f32; 4],
    mode: BlendMode,
    path: &str,
) -> Result<(), ValidationError> {
    let mode = if mode == BlendMode::LinearPlus {
        BlendMode::Plus
    } else {
        mode
    };
    if mode == BlendMode::SourceOver {
        return blend(destination, source, 1.0, path);
    }
    let source_alpha = f64::from(source[3]);
    let backdrop_alpha = f64::from(destination[3]);
    let mixed_rgb = if matches!(
        mode,
        BlendMode::Hue | BlendMode::Saturation | BlendMode::Color | BlendMode::Luminosity
    ) {
        let backdrop = std::array::from_fn(|channel| {
            if backdrop_alpha > 0.0 {
                f64::from(destination[channel]) / backdrop_alpha
            } else {
                0.0
            }
        });
        let source = std::array::from_fn(|channel| {
            if source_alpha > 0.0 {
                f64::from(source[channel]) / source_alpha
            } else {
                0.0
            }
        });
        Some(nonseparable(backdrop, source, mode))
    } else {
        None
    };
    let mut result = [0.0; 4];
    for channel in 0..3 {
        let source_value = f64::from(source[channel]);
        let backdrop_value = f64::from(destination[channel]);
        let value = if mode == BlendMode::Plus {
            source_value + backdrop_value
        } else {
            let straight_source = if source_alpha > 0.0 {
                source_value / source_alpha
            } else {
                0.0
            };
            let straight_backdrop = if backdrop_alpha > 0.0 {
                backdrop_value / backdrop_alpha
            } else {
                0.0
            };
            let mixed = match mode {
                BlendMode::Multiply => straight_source * straight_backdrop,
                BlendMode::Screen => {
                    straight_source + straight_backdrop - straight_source * straight_backdrop
                }
                BlendMode::Darken => straight_source.min(straight_backdrop),
                BlendMode::Lighten => straight_source.max(straight_backdrop),
                BlendMode::Difference => (straight_backdrop - straight_source).abs(),
                BlendMode::Exclusion => {
                    straight_source + straight_backdrop - 2.0 * straight_source * straight_backdrop
                }
                BlendMode::Overlay => hard_light(straight_source, straight_backdrop),
                BlendMode::HardLight => hard_light(straight_backdrop, straight_source),
                BlendMode::SoftLight => {
                    if straight_source <= 0.5 {
                        straight_backdrop
                            - (1.0 - 2.0 * straight_source)
                                * straight_backdrop
                                * (1.0 - straight_backdrop)
                    } else {
                        let curve = if straight_backdrop <= 0.25 {
                            ((16.0 * straight_backdrop - 12.0) * straight_backdrop + 4.0)
                                * straight_backdrop
                        } else {
                            libm::sqrt(straight_backdrop)
                        };
                        straight_backdrop
                            + (2.0 * straight_source - 1.0) * (curve - straight_backdrop)
                    }
                }
                BlendMode::ColorDodge => {
                    if straight_backdrop <= 0.0 {
                        0.0
                    } else if straight_source >= 1.0 {
                        1.0
                    } else {
                        (straight_backdrop / (1.0 - straight_source)).min(1.0)
                    }
                }
                BlendMode::ColorBurn => {
                    if straight_backdrop >= 1.0 {
                        1.0
                    } else if straight_source <= 0.0 {
                        0.0
                    } else {
                        1.0 - ((1.0 - straight_backdrop) / straight_source).min(1.0)
                    }
                }
                BlendMode::SourceOver | BlendMode::Plus | BlendMode::LinearPlus => {
                    unreachable!("handled separately")
                }
                BlendMode::Hue
                | BlendMode::Saturation
                | BlendMode::Color
                | BlendMode::Luminosity => mixed_rgb.unwrap()[channel],
            };
            source_value * (1.0 - backdrop_alpha)
                + backdrop_value * (1.0 - source_alpha)
                + source_alpha * backdrop_alpha * mixed
        };
        result[channel] = value as f32;
    }
    result[3] = if mode == BlendMode::Plus {
        (source_alpha + backdrop_alpha).min(1.0)
    } else {
        source_alpha + backdrop_alpha * (1.0 - source_alpha)
    } as f32;
    if result.iter().any(|value| !value.is_finite()) {
        return Err(ValidationError::invalid(
            path,
            "color",
            "layer blending overflowed finite f32 working storage",
        ));
    }
    *destination = result;
    Ok(())
}

#[cfg(test)]
mod tests;
