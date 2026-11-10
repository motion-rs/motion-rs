//! Seeded scalar and derivative fields sampled directly from the shared noise APIs.

use super::atlas::{self, Page};
use crate::{DemoMetadata, DemoParameters};
use motion::noise::{self, DomainWarp, Fractal};
use motion::prelude::*;
use std::sync::Arc;

pub(super) const METADATA: DemoMetadata = DemoMetadata::new(
    "noise-atlas",
    "Noise Atlas",
    "Procedural",
    &[
        "seeded-noise",
        "fractal-fields",
        "domain-warp",
        "analytic-derivatives",
    ],
);

/// Fixed field dimensions in texels; unrelated to the user density parameter.
const FIELD_SIZE: [u32; 2] = [160, 88];

pub(super) const PAGES: &[Page] = &[
    Page {
        title: "Value noise",
        lines: [
            "Hashed lattice values use",
            "quintic interpolation to",
            "form a continuous field.",
        ],
    },
    Page {
        title: "Perlin noise",
        lines: [
            "Lattice gradients create",
            "smooth signed detail with",
            "analytic spatial derivatives.",
        ],
    },
    Page {
        title: "Simplex noise",
        lines: [
            "Triangular cells produce",
            "a continuous gradient field",
            "without square-grid edges.",
        ],
    },
    Page {
        title: "3D Perlin slice",
        lines: [
            "Time moves a slice through",
            "a three-dimensional field",
            "rather than shifting a bitmap.",
        ],
    },
    Page {
        title: "Worley distance",
        lines: [
            "Distance to the nearest",
            "seeded feature point creates",
            "a cellular scalar field.",
        ],
    },
    Page {
        title: "Worley cell edges",
        lines: [
            "The second-nearest distance",
            "minus the nearest distance",
            "reveals cell boundaries.",
        ],
    },
    Page {
        title: "Fractal octaves",
        lines: [
            "Normalized fBm combines",
            "five frequency octaves",
            "with decreasing amplitudes.",
        ],
    },
    Page {
        title: "Turbulence",
        lines: [
            "Absolute Perlin octaves",
            "create folded details while",
            "retaining explicit seeds.",
        ],
    },
    Page {
        title: "Ridged fractal",
        lines: [
            "Squared inverted absolute",
            "noise forms sharp ridges",
            "across multiple octaves.",
        ],
    },
    Page {
        title: "Domain warp",
        lines: [
            "A seeded displacement field",
            "warps coordinates before",
            "sampling the final material.",
        ],
    },
    Page {
        title: "Analytic gradient",
        lines: [
            "Arrows show exact spatial",
            "derivatives of Perlin noise",
            "without finite differences.",
        ],
    },
    Page {
        title: "Curl field",
        lines: [
            "Rotating the gradient gives",
            "a divergence-free velocity",
            "field from one potential.",
        ],
    },
];

pub(super) fn timeline(parameters: DemoParameters) -> motion::Result<Timeline> {
    let seed = parameters.seed as u32 ^ (parameters.seed >> 32) as u32;
    atlas::timeline(parameters, METADATA, PAGES, move |page, progress, _| {
        if page >= 10 {
            return Ok(arrows(page, progress, seed));
        }
        let image = field_image(page, progress, seed)?;
        Ok(Image::from_asset(Arc::new(image))
            .pivot(Pivot::Point(Point::ORIGIN))
            .scale_xy(
                448.0 / f64::from(FIELD_SIZE[0]),
                240.0 / f64::from(FIELD_SIZE[1]),
            )
            .translate(32.0, 32.0))
    })
}

fn field_image(page: usize, progress: f64, seed: u32) -> motion::Result<ImageAsset> {
    let mut pixels = Vec::with_capacity((FIELD_SIZE[0] * FIELD_SIZE[1] * 4) as usize);
    for row in 0..FIELD_SIZE[1] {
        for column in 0..FIELD_SIZE[0] {
            let point = [
                f64::from(column) / 28.0 + progress * 1.4,
                f64::from(row) / 28.0,
            ];
            let value = scalar(page, point, progress, seed)?.clamp(0.0, 1.0);
            pixels.extend_from_slice(&[
                (24.0 + value * 231.0) as u8,
                (46.0 + value * 153.0) as u8,
                (55.0 + value * 64.0) as u8,
                255,
            ]);
        }
    }
    ImageAsset::from_rgba8(FIELD_SIZE[0], FIELD_SIZE[1], pixels)
}

fn scalar(page: usize, point: [f64; 2], progress: f64, seed: u32) -> motion::Result<f64> {
    let fractal = Fractal {
        octaves: 5,
        ..Fractal::default()
    };
    Ok(match page {
        0 => noise::value2(point, seed) * 0.5 + 0.5,
        1 => noise::perlin2(point, seed) * 0.8 + 0.5,
        2 => noise::simplex2(point, seed) * 0.5 + 0.5,
        3 => noise::perlin3([point[0], point[1], progress * 2.0], seed) * 0.8 + 0.5,
        4 => noise::worley2(point, seed).0,
        5 => {
            let (nearest, second) = noise::worley2(point, seed);
            (second - nearest) * 2.0
        }
        6 => noise::fbm(|point| noise::perlin2(point, seed), point, fractal)? * 1.4 + 0.5,
        7 => noise::turbulence2(point, seed, fractal)? * 2.0,
        8 => noise::ridged2(point, seed, fractal)?,
        _ => {
            noise::simplex2(
                noise::domain_warp(
                    point,
                    DomainWarp {
                        seed,
                        strength: 1.3,
                        ..DomainWarp::default()
                    },
                )?,
                seed,
            ) * 0.5
                + 0.5
        }
    })
}

fn arrows(page: usize, progress: f64, seed: u32) -> Node {
    let mut nodes = Vec::new();
    for row in 0..7 {
        for column in 0..12 {
            let origin = Point::new(
                47.0 + f64::from(column) * 38.0,
                38.0 + f64::from(row) * 38.0,
            );
            let point = [
                f64::from(column) * 0.3 + progress * 1.4,
                f64::from(row) * 0.3,
            ];
            let velocity = if page == 10 {
                noise::perlin2_d(point, seed).1
            } else {
                noise::curl2(point, seed)
            };
            let end = Point::new(origin.x + velocity[0] * 15.0, origin.y + velocity[1] * 15.0);
            let mut path = Path2D::new();
            path.move_to(origin).line_to(end);
            nodes.push(Node::from(path.stroke(1.7, Color::from_hex(atlas::MINT))));
            nodes.push(
                Circle::new(end, 2.5)
                    .fill(Color::from_hex(atlas::GOLD))
                    .into(),
            );
        }
    }
    Node::group(nodes)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn scalar_fields_replay_and_change_with_explicit_time() -> motion::Result {
        for page in 0..10 {
            let early = scalar(page, [0.37, 1.23], 0.2, 17)?;
            let late = scalar(page, [0.91, 1.23], 0.8, 17)?;
            assert!(early.is_finite() && late.is_finite());
            assert_eq!(early, scalar(page, [0.37, 1.23], 0.2, 17)?);
            assert_ne!(early, late, "field {page}");
        }
        Ok(())
    }
}
