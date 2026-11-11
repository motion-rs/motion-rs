//! Matched source and processed specimens for spatial and color filters.

use super::atlas::{self, Page};
use crate::{DemoMetadata, DemoParameters};
use motion::prelude::*;
use motion::render::FilterOperation;

pub(super) const METADATA: DemoMetadata = DemoMetadata::new(
    "filter-atlas",
    "Filter Atlas",
    "Rendering",
    &["filters", "comparison", "explicit-time"],
);

pub(super) const PAGES: &[Page] = &[
    Page {
        title: "Blur",
        lines: [
            "Gaussian-style blur spreads",
            "premultiplied color across",
            "transparent source borders.",
        ],
    },
    Page {
        title: "Drop shadow",
        lines: [
            "A blurred alpha silhouette",
            "is offset behind the source;",
            "the original remains sharp.",
        ],
    },
    Page {
        title: "Glow",
        lines: [
            "Bright source pixels add",
            "a single-scale light halo",
            "without replacing the source.",
        ],
    },
    Page {
        title: "Color adjustment",
        lines: [
            "Exposure, contrast and hue",
            "operate on linear color;",
            "alpha remains unchanged.",
        ],
    },
    Page {
        title: "Bloom",
        lines: [
            "Soft highlight extraction",
            "feeds several blur octaves",
            "for a broad light envelope.",
        ],
    },
    Page {
        title: "Halation",
        lines: [
            "Bright edges receive a warm",
            "red-orange halo assembled",
            "from ordinary primitives.",
        ],
    },
    Page {
        title: "Anamorphic",
        lines: [
            "A bright source produces",
            "horizontal tinted streaks",
            "with a narrow vertical blur.",
        ],
    },
    Page {
        title: "Lens distortion",
        lines: [
            "Radial sampling bends the",
            "source around its center;",
            "auto-fit protects corners.",
        ],
    },
    Page {
        title: "Chromatic dispersion",
        lines: [
            "Color channels sample at",
            "different radial distances",
            "to separate bright edges.",
        ],
    },
    Page {
        title: "Lens vignette",
        lines: [
            "Cosine-fourth falloff",
            "darkens the outer field",
            "around the optical center.",
        ],
    },
    Page {
        title: "Aperture bokeh",
        lines: [
            "A six-blade aperture shapes",
            "defocused highlights with",
            "bounded multi-tap sampling.",
        ],
    },
    Page {
        title: "Radial blur",
        lines: [
            "Samples travel toward an",
            "explicit light position",
            "with decaying ray weights.",
        ],
    },
    Page {
        title: "Lens flare",
        lines: [
            "Mirrored highlight ghosts",
            "and a radial halo share",
            "an explicit optical center.",
        ],
    },
    Page {
        title: "Relight",
        lines: [
            "Luminance becomes height;",
            "surface normals respond to",
            "a moving light direction.",
        ],
    },
    Page {
        title: "Seeded grain",
        lines: [
            "Scene-linear monochrome",
            "noise has explicit seeds",
            "and preserves source alpha.",
        ],
    },
    Page {
        title: "Directional blur",
        lines: [
            "Independent axis sigmas",
            "stretch a custom blur",
            "along the horizontal axis.",
        ],
    },
    Page {
        title: "Filter order",
        lines: [
            "Exposure before a threshold",
            "changes extracted energy;",
            "reversing the chain differs.",
        ],
    },
];

pub(super) fn timeline(parameters: DemoParameters) -> Result<Timeline, motion::ErrorReport> {
    atlas::timeline(parameters, METADATA, PAGES, move |page, progress, _| {
        let source = atlas::specimen(progress)?;
        let amount = 0.25 + 0.75 * progress;
        let filter = match page {
            0 => Filter::Blur {
                sigma: 12.0 * amount,
            },
            1 => Filter::DropShadow {
                offset: Vector2::new(18.0, 20.0),
                sigma: 9.0 * amount,
                color: Color::from_hex(atlas::GOLD),
            },
            2 => Filter::Glow {
                threshold: 0.5,
                sigma: 14.0,
                intensity: 1.8 * amount,
            },
            3 => Filter::ColorAdjust {
                exposure: 0.5,
                contrast: 1.2,
                saturation: 0.4 + progress,
                hue_degrees: 90.0 * progress,
            },
            4 => Filter::Bloom {
                threshold: 0.6,
                knee: 0.2,
                radius: 5.0,
                levels: 3,
                falloff: 0.55,
                intensity: 1.5 * amount,
                tint: Color::WHITE,
            },
            5 => Filter::halation(2.0 * amount, 12.0)?,
            6 => Filter::Optics(Optics::Anamorphic {
                threshold: 0.6,
                length: 32.0 * amount,
                stretch: 12.0,
                tint: Color::from_hex(atlas::BLUE),
                intensity: 1.5,
            }),
            7 => Filter::Optics(Optics::Lens {
                distortion: 0.65 * amount,
                chromatic: 0.0,
                auto_fit: true,
                center: None,
            }),
            8 => Filter::Optics(Optics::Lens {
                distortion: 0.0,
                chromatic: 0.07 * amount,
                auto_fit: true,
                center: None,
            }),
            9 => Filter::Optics(Optics::LensVignette {
                angle_of_view: 125.0,
                strength: amount,
                roundness: 0.5,
            }),
            10 => Filter::Optics(Optics::Bokeh {
                radius: 14.0 * amount,
                shape: ApertureShape::Polygon {
                    sides: 6,
                    rotation: progress,
                },
                highlight_boost: 1.2,
            }),
            11 => Filter::Optics(Optics::RadialBlur {
                center: Point::new(220.0, 100.0),
                strength: 0.5 * amount,
                samples: 16,
                decay: 0.92,
            }),
            12 => Filter::Optics(Optics::LensFlare {
                center: Point::new(300.0, 185.0),
                ghosts: 4,
                threshold: 0.6,
                intensity: 2.0 * amount,
                tint: Color::from_hex(atlas::GOLD),
            }),
            13 => Filter::Optics(Optics::Relight {
                height_from: HeightFrom::Luma,
                light_dir: [math::cos(progress * 6.0), math::sin(progress * 6.0), 0.7],
                color: Color::from_hex(atlas::GOLD),
                ambient: 0.25,
                specular: 1.5,
            }),
            14 => Filter::grain(parameters.seed, 0.12 * amount),
            15 => Filter::Custom(vec![FilterOperation::BlurXY {
                sigma_x: 22.0 * amount,
                sigma_y: 1.0,
            }]),
            _ => {
                let exposure = Filter::ColorAdjust {
                    exposure: 1.5 * amount,
                    contrast: 1.0,
                    saturation: 1.0,
                    hue_degrees: 0.0,
                };
                let glow = Filter::Glow {
                    threshold: 0.9,
                    sigma: 18.0,
                    intensity: 1.0,
                };
                return Ok(atlas::pair(
                    source.clone().filter(glow.clone()).filter(exposure.clone()),
                    source.filter(exposure).filter(glow),
                    "Glow / exposure",
                    "Exposure / glow",
                ));
            }
        };
        Ok(atlas::comparison(source.clone(), source.filter(filter)))
    })
}
