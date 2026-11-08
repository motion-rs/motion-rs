//! Bounded physical lens and height-field effect descriptions.

use crate::{Color, Point, Rect, Transform, ValidationError};

/// Fewest and most Bokeh aperture taps; the kernel array holds the maximum.
const BOKEH_TAPS: [u32; 2] = [8, 64];

/// Bokeh taps per square pixel of radius. Vogel-spiral taps in a disc of radius `r` lie about
/// `r * sqrt(pi / n)` apart, so `1.4 * r * r` taps keep that spacing near 1.5 pixels.
const BOKEH_TAP_DENSITY: f64 = 1.4;

/// Aperture footprint used for highlight bokeh.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub enum ApertureShape {
    /// Circular aperture.
    #[default]
    Disc,
    /// Regular aperture polygon with 3..=16 blades.
    Polygon {
        /// Blade count.
        sides: u8,
        /// Rotation in radians.
        rotation: f64,
    },
}

/// Source for the relighting height field.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum HeightFrom {
    /// Straight-linear Rec.709 luminance.
    Luma,
    /// Source opacity.
    Alpha,
}

/// Opt-in spatial effects on a completed subtree.
#[derive(Clone, Debug, PartialEq)]
pub enum Optics {
    /// Mirrored bright-source ghosts plus a restrained radial halo.
    LensFlare {
        /// Local optical center.
        center: Point,
        /// Ghost count, in 1..=8.
        ghosts: u8,
        /// Nonnegative highlight cutoff.
        threshold: f64,
        /// Nonnegative added-energy multiplier.
        intensity: f64,
        /// Encoded-sRGB ghost tint.
        tint: Color,
    },
    /// Radial distortion and channel-dependent radial dispersion.
    Lens {
        /// Distortion coefficient, in -0.3..=1.
        distortion: f64,
        /// Dispersion coefficient, in -0.1..=0.1.
        chromatic: f64,
        /// Shrinks sample coordinates to avoid corner holes.
        auto_fit: bool,
        /// Local center; None uses effect bounds.
        center: Option<Point>,
    },
    /// Physical cos^4 falloff.
    LensVignette {
        /// Full diagonal field of view, in (0, 170) degrees.
        angle_of_view: f64,
        /// Effect strength, in 0..=1.
        strength: f64,
        /// Zero elliptical, one circular.
        roundness: f64,
    },
    /// Bounded aperture blur with highlight-weighted sampling. Small radii use fewer taps; see
    /// [`Optics::bokeh_taps`].
    Bokeh {
        /// Local blur radius.
        radius: f64,
        /// Aperture footprint.
        shape: ApertureShape,
        /// Nonnegative highlight weighting.
        highlight_boost: f64,
    },
    /// Bounded sampling from each pixel toward a light source.
    RadialBlur {
        /// Local light source position.
        center: Point,
        /// Ray length fraction, in 0..=1.
        strength: f64,
        /// Number of taps, in 1..=64.
        samples: u8,
        /// Relative weight decay, in 0..=1.
        decay: f64,
    },
    /// Anamorphic horizontal streak: highlights above `threshold` are stretched along the
    /// device X axis with a tinted Gaussian and added back to the source.
    Anamorphic {
        /// Nonnegative luminance cutoff applied to straight-linear highlights.
        threshold: f64,
        /// Horizontal streak standard deviation in local units.
        length: f64,
        /// Horizontal-to-vertical aspect ratio, in 1..=64; vertical sigma is `length / stretch`.
        stretch: f64,
        /// Encoded-sRGB streak tint; its alpha is ignored.
        tint: Color,
        /// Nonnegative added-energy multiplier.
        intensity: f64,
    },
    /// Sobel height normals, Lambert diffuse and Blinn-Phong specular.
    Relight {
        /// Height field source.
        height_from: HeightFrom,
        /// Finite nonzero direction toward the light.
        light_dir: [f64; 3],
        /// Light color.
        color: Color,
        /// Nonnegative ambient multiplier.
        ambient: f64,
        /// Nonnegative specular multiplier.
        specular: f64,
    },
}

impl Optics {
    /// Validates bounded finite parameters before capture or device initialization.
    pub fn validate(&self) -> Result<(), ValidationError> {
        let invalid = || {
            ValidationError::invalid(
                "optics",
                "parameters",
                "Require finite bounded optical parameters",
            )
        };
        let valid = match self {
            Self::LensFlare {
                center,
                ghosts,
                threshold,
                intensity,
                ..
            } => {
                center.x.is_finite()
                    && center.y.is_finite()
                    && (1..=8).contains(ghosts)
                    && threshold.is_finite()
                    && *threshold >= 0.0
                    && intensity.is_finite()
                    && *intensity >= 0.0
            }
            Self::Lens {
                distortion,
                chromatic,
                center,
                ..
            } => {
                distortion.is_finite()
                    && (-0.3..=1.0).contains(distortion)
                    && chromatic.is_finite()
                    && (-0.1..=0.1).contains(chromatic)
                    && center.is_none_or(|point| point.x.is_finite() && point.y.is_finite())
            }
            Self::LensVignette {
                angle_of_view,
                strength,
                roundness,
            } => {
                angle_of_view.is_finite()
                    && *angle_of_view > 0.0
                    && *angle_of_view < 170.0
                    && (0.0..=1.0).contains(strength)
                    && (0.0..=1.0).contains(roundness)
            }
            Self::Bokeh {
                radius,
                shape,
                highlight_boost,
            } => {
                radius.is_finite()
                    && *radius >= 0.0
                    && (radius * 3.0).is_finite()
                    && highlight_boost.is_finite()
                    && *highlight_boost >= 0.0
                    && match shape {
                        ApertureShape::Disc => true,
                        ApertureShape::Polygon { sides, rotation } => {
                            (3..=16).contains(sides) && rotation.is_finite()
                        }
                    }
            }
            Self::RadialBlur {
                center,
                strength,
                samples,
                decay,
            } => {
                center.x.is_finite()
                    && center.y.is_finite()
                    && (0.0..=1.0).contains(strength)
                    && (0.0..=1.0).contains(decay)
                    && (1..=64).contains(samples)
            }
            Self::Anamorphic {
                threshold,
                length,
                stretch,
                intensity,
                ..
            } => {
                threshold.is_finite()
                    && *threshold >= 0.0
                    && length.is_finite()
                    && *length >= 0.0
                    && (length * 3.0).is_finite()
                    && (1.0..=64.0).contains(stretch)
                    && intensity.is_finite()
                    && *intensity >= 0.0
            }
            Self::Relight {
                light_dir,
                ambient,
                specular,
                ..
            } => {
                light_dir.iter().all(|value| value.is_finite())
                    && light_dir
                        .iter()
                        .map(|value| value * value)
                        .sum::<f64>()
                        .is_finite()
                    && light_dir.iter().map(|value| value * value).sum::<f64>() > 0.0
                    && ambient.is_finite()
                    && *ambient >= 0.0
                    && specular.is_finite()
                    && *specular >= 0.0
            }
        };
        if valid { Ok(()) } else { Err(invalid()) }
    }

    /// Maps local positions/radii to conservative device-space parameters.
    pub fn device(&self, transform: Transform, scale: f64) -> Self {
        match self {
            Self::LensFlare {
                center,
                ghosts,
                threshold,
                intensity,
                tint,
            } => Self::LensFlare {
                center: transform.apply(*center),
                ghosts: *ghosts,
                threshold: *threshold,
                intensity: *intensity,
                tint: *tint,
            },
            Self::Lens {
                distortion,
                chromatic,
                auto_fit,
                center,
            } => Self::Lens {
                distortion: *distortion,
                chromatic: *chromatic,
                auto_fit: *auto_fit,
                center: center.map(|center| transform.apply(center)),
            },
            Self::Bokeh {
                radius,
                shape,
                highlight_boost,
            } => Self::Bokeh {
                radius: radius * scale,
                shape: *shape,
                highlight_boost: *highlight_boost,
            },
            Self::RadialBlur {
                center,
                strength,
                samples,
                decay,
            } => Self::RadialBlur {
                center: transform.apply(*center),
                strength: *strength,
                samples: *samples,
                decay: *decay,
            },
            Self::Anamorphic {
                threshold,
                length,
                stretch,
                tint,
                intensity,
            } => Self::Anamorphic {
                threshold: *threshold,
                length: length * scale,
                stretch: *stretch,
                tint: *tint,
                intensity: *intensity,
            },
            _ => self.clone(),
        }
    }

    /// Horizontal and vertical Gaussian sigmas of an anamorphic streak.
    pub fn streak_sigmas(&self) -> Option<[f64; 2]> {
        match self {
            Self::Anamorphic {
                length, stretch, ..
            } => Some([*length, *length / *stretch]),
            _ => None,
        }
    }

    /// Conservative output support; radial optics retain the original frame bounds.
    pub fn bounds(&self, source: Rect) -> Rect {
        match self {
            Self::Bokeh { radius, .. } => {
                let radius = radius.ceil();
                Rect::new(
                    source.left - radius,
                    source.top - radius,
                    source.right + radius,
                    source.bottom + radius,
                )
            }
            Self::Anamorphic { .. } => {
                let [x, y] = self.streak_sigmas().expect("anamorphic sigmas");
                crate::FilterOperation::BlurXY {
                    sigma_x: x,
                    sigma_y: y,
                }
                .bounds(source, source)
            }
            _ => source,
        }
    }

    /// Aperture taps used for a Bokeh `radius`: `ceil(1.4 * radius^2)` within 8..=64, so tap
    /// spacing stays near 1.5 pixels until the 64-tap maximum. Non-finite radii use the maximum.
    pub fn bokeh_taps(radius: f64) -> u32 {
        let [minimum, maximum] = BOKEH_TAPS;
        let wanted = (BOKEH_TAP_DENSITY * radius * radius).ceil();
        if wanted.is_finite() {
            (wanted as u32).clamp(minimum, maximum)
        } else {
            maximum
        }
    }

    /// Shared CPU/GPU numerical contract, with up to 64 deterministic aperture offsets. For
    /// Bokeh, `values[3]` holds the tap count and offsets past it are zero.
    pub fn parameters(&self, bounds: Rect) -> ([f64; 32], [[f32; 2]; 64]) {
        let mut values = [0.0; 32];
        let mut kernel = [[0.0; 2]; 64];
        match self {
            Self::LensFlare {
                center,
                ghosts,
                threshold,
                intensity,
                tint,
            } => {
                let tint = tint.to_linear_premultiplied();
                values[..10].copy_from_slice(&[
                    5.0,
                    center.x,
                    center.y,
                    f64::from(*ghosts),
                    *threshold,
                    *intensity,
                    tint[0],
                    tint[1],
                    tint[2],
                    libm::hypot(bounds.width(), bounds.height()).max(1.0) * 0.5,
                ]);
                values[10] = tint[3];
            }
            Self::Lens {
                distortion,
                chromatic,
                auto_fit,
                center,
            } => {
                let center = center.unwrap_or(bounds.center());
                let radius = [
                    Point::new(bounds.left, bounds.top),
                    Point::new(bounds.right, bounds.top),
                    Point::new(bounds.left, bounds.bottom),
                    Point::new(bounds.right, bounds.bottom),
                ]
                .map(|point| libm::hypot(point.x - center.x, point.y - center.y))
                .into_iter()
                .fold(0.0, f64::max);
                values[..7].copy_from_slice(&[
                    0.0,
                    *distortion,
                    *chromatic,
                    f64::from(*auto_fit),
                    center.x,
                    center.y,
                    radius.max(1e-6),
                ]);
            }
            Self::LensVignette {
                angle_of_view,
                strength,
                roundness,
            } => {
                let center = bounds.center();
                values[..8].copy_from_slice(&[
                    1.0,
                    libm::tan(angle_of_view.to_radians() * 0.5),
                    *strength,
                    *roundness,
                    center.x,
                    center.y,
                    (bounds.width() * 0.5).max(1e-6),
                    (bounds.height() * 0.5).max(1e-6),
                ]);
            }
            Self::Bokeh {
                radius,
                shape,
                highlight_boost,
            } => {
                let taps = Self::bokeh_taps(*radius);
                values[..4].copy_from_slice(&[2.0, *radius, *highlight_boost, f64::from(taps)]);
                for (index, tap) in kernel.iter_mut().take(taps as usize).enumerate() {
                    let angle = index as f64 * 2.399963229728653;
                    let mut distance = libm::sqrt((index as f64 + 0.5) / f64::from(taps)) * radius;
                    if let ApertureShape::Polygon { sides, rotation } = shape {
                        let sector = std::f64::consts::TAU / f64::from(*sides);
                        distance *= libm::cos(std::f64::consts::PI / f64::from(*sides))
                            / libm::cos(
                                (angle - rotation + sector * 0.5).rem_euclid(sector) - sector * 0.5,
                            );
                    }
                    *tap = [
                        (libm::cos(angle) * distance) as f32,
                        (libm::sin(angle) * distance) as f32,
                    ];
                }
            }
            Self::RadialBlur {
                center,
                strength,
                samples,
                decay,
            } => {
                values[..6].copy_from_slice(&[
                    3.0,
                    center.x,
                    center.y,
                    *strength,
                    f64::from(*samples),
                    *decay,
                ]);
            }
            Self::Anamorphic { .. } => values[0] = 6.0,
            Self::Relight {
                height_from,
                light_dir,
                color,
                ambient,
                specular,
            } => {
                let length = libm::sqrt(light_dir.iter().map(|value| value * value).sum());
                let color = color.to_linear_premultiplied();
                values[..10].copy_from_slice(&[
                    4.0,
                    f64::from(*height_from == HeightFrom::Alpha),
                    light_dir[0] / length,
                    light_dir[1] / length,
                    light_dir[2] / length,
                    color[0],
                    color[1],
                    color[2],
                    *ambient,
                    *specular,
                ]);
            }
        }
        (values, kernel)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bokeh_taps_scale_with_radius_and_fill_only_the_used_offsets() {
        assert_eq!(Optics::bokeh_taps(0.0), 8);
        assert_eq!(Optics::bokeh_taps(3.0), 13);
        assert_eq!(Optics::bokeh_taps(6.0), 51);
        assert_eq!(Optics::bokeh_taps(16.0), 64);
        assert_eq!(Optics::bokeh_taps(f64::MAX), 64);
        for (radius, shape) in [
            (2.5, ApertureShape::Disc),
            (
                12.0,
                ApertureShape::Polygon {
                    sides: 6,
                    rotation: 0.3,
                },
            ),
        ] {
            let optics = Optics::Bokeh {
                radius,
                shape,
                highlight_boost: 1.0,
            };
            let (values, kernel) = optics.parameters(Rect::new(0.0, 0.0, 64.0, 64.0));
            let taps = Optics::bokeh_taps(radius) as usize;
            assert_eq!(values[3], taps as f64);
            for (index, tap) in kernel.iter().enumerate() {
                let distance = f64::from(tap[0]).hypot(f64::from(tap[1]));
                if index < taps {
                    assert!(distance > 0.0 && distance <= radius + 1e-4, "{index}");
                } else {
                    assert_eq!(*tap, [0.0; 2]);
                }
            }
            // Used taps still reach the aperture edge; polygon edges sit at cos(pi / sides).
            let reach = kernel[..taps]
                .iter()
                .map(|tap| f64::from(tap[0]).hypot(f64::from(tap[1])))
                .fold(0.0, f64::max);
            assert!(reach > radius * 0.85, "{reach}");
        }
    }
}
