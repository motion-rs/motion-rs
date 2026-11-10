//! Extended ink controls using retained full strokes, recorded timing and deterministic materials.

use super::atlas::{self, Page};
use crate::{DemoMetadata, DemoParameters};
use motion::ink::{
    BrushCap, BrushStroke, BrushStrokeBuilder, BrushTiming, Ink, InputSample, NibMode, Paper,
    PigmentMix,
};
use motion::prelude::*;
use std::sync::Arc;

pub(super) const METADATA: DemoMetadata = DemoMetadata::new(
    "ink-features",
    "Ink Features",
    "Brushes",
    &[
        "pressure-curves",
        "nib-orientation",
        "ink-materials",
        "recorded-input",
        "pigment-mixing",
        "timed-writing",
    ],
);

/// Per-stroke material distance-evaluation allowance for complete multi-bristle specimens.
const MATERIAL_WORK: u64 = 128_000_000;

/// Ninety-six intervals keep five 24-sided nib sweeps within the per-path verb allowance.
const BRISTLE_SAMPLES: usize = 96;

pub(super) const PAGES: &[Page] = &[
    Page {
        title: "Pressure curve",
        lines: [
            "Ordered pressure controls",
            "vary the nib width along",
            "the complete path length.",
        ],
    },
    Page {
        title: "Pressure function",
        lines: [
            "A pure analytic function",
            "drives repeated width pulses",
            "without recorded state.",
        ],
    },
    Page {
        title: "Fixed nib",
        lines: [
            "An elliptical nib keeps",
            "a fixed angle while the",
            "path changes direction.",
        ],
    },
    Page {
        title: "Following nib",
        lines: [
            "The same elliptical nib",
            "rotates with the tangent",
            "plus an explicit offset.",
        ],
    },
    Page {
        title: "Seeded bristles",
        lines: [
            "Five offset bristles share",
            "one continuous path with",
            "seeded spread and jitter.",
        ],
    },
    Page {
        title: "Endpoint caps",
        lines: [
            "A tapered entry and flat",
            "exit alter the nib imprint",
            "at the path endpoints.",
        ],
    },
    Page {
        title: "Gradient paint",
        lines: [
            "A spatial gradient replaces",
            "solid paint before material",
            "coverage is attenuated.",
        ],
    },
    Page {
        title: "Ink depletion",
        lines: [
            "Load decreases along the",
            "full stroke, revealing dry",
            "gaps without path splitting.",
        ],
    },
    Page {
        title: "Dry fibers",
        lines: [
            "Dry coverage follows path",
            "arc length and transverse",
            "distance across the nib.",
        ],
    },
    Page {
        title: "Grain and edges",
        lines: [
            "Seeded granulation mottles",
            "coverage while roughness",
            "displaces the geometric rim.",
        ],
    },
    Page {
        title: "Paper texture",
        lines: [
            "A generated height texture",
            "repeats under the stroke",
            "and attenuates pigment.",
        ],
    },
    Page {
        title: "Intersection build-up",
        lines: [
            "Coverage from crossing",
            "segments adds density",
            "inside one retained stroke.",
        ],
    },
    Page {
        title: "Rim darkening",
        lines: [
            "The stroke's geometric rim",
            "receives darker color with",
            "directional material detail.",
        ],
    },
    Page {
        title: "Path wobble",
        lines: [
            "A fixed seeded displacement",
            "adds irregularity without",
            "changing on replay.",
        ],
    },
    Page {
        title: "Frame-seeded boil",
        lines: [
            "Explicit frame numbers",
            "change wobble every two",
            "frames without history.",
        ],
    },
    Page {
        title: "Analytic wetness",
        lines: [
            "Age expands the wash halo",
            "according to drying time;",
            "no fluid solver is involved.",
        ],
    },
    Page {
        title: "Recorded input",
        lines: [
            "Position, pressure and tilt",
            "retain recorded timestamps",
            "including a stationary pause.",
        ],
    },
    Page {
        title: "Timed pen lifts",
        lines: [
            "A monotone timing curve",
            "and explicit pen-up pause",
            "control separate subpaths.",
        ],
    },
    Page {
        title: "Brush eraser",
        lines: [
            "The written stroke becomes",
            "an inverted-alpha mask",
            "over the colored source.",
        ],
    },
    Page {
        title: "Pigment mixing",
        lines: [
            "Linear-light mixing and",
            "an approximate six-band",
            "pigment model are compared.",
        ],
    },
    Page {
        title: "Height and relight",
        lines: [
            "A separate grayscale ridge",
            "drives surface lighting",
            "above the colored stroke.",
        ],
    },
];

pub(super) fn timeline(parameters: DemoParameters) -> motion::Result<Timeline> {
    let seed = parameters.seed as u32 ^ (parameters.seed >> 32) as u32;
    let builders = (0..PAGES.len())
        .map(|page| configured(page, seed))
        .collect::<motion::Result<Vec<_>>>()?;
    let strokes = builders
        .iter()
        .map(|builder| builder.clone().build())
        .collect::<Result<Vec<_>, _>>()?;
    let recorded = recording()?;
    let timing = BrushTiming {
        duration: 2.8.seconds(),
        curve: vec![(0.0, 0.0), (0.25, 0.1), (0.65, 0.85), (1.0, 1.0)],
        lift_pause: 0.65.seconds(),
    };
    atlas::timeline(parameters, METADATA, PAGES, move |page, progress, _| {
        let reveal = (0.08 + progress * 1.1).min(1.0);
        let stroke = strokes[page].clone();
        let ink = match page {
            14 => Node::from(
                builders[page]
                    .clone()
                    .boil(3.0, 0.05, seed, (progress * 120.0) as u64)
                    .build()?
                    .write(reveal),
            ),
            15 => Node::from(
                builders[page]
                    .clone()
                    .try_wetness((progress * 4.0).seconds(), 10.0, 1.4.seconds())?
                    .build()?,
            ),
            16 => Node::from(recorded.clone().write_with((progress * 4.0).seconds())?),
            17 => Node::from(stroke.write_timed((progress * 3.45).seconds(), &timing)?),
            18 => stroke.write(reveal).erase(atlas::specimen(progress)?),
            19 => pigment(progress)?,
            20 => {
                let relief = builders[page]
                    .clone()
                    .ink(Ink::new(0).height_map(2.0))
                    .build()?
                    .write(reveal);
                Node::group([
                    Node::from(stroke.write(reveal)),
                    Node::from(relief)
                        .filter(Filter::Optics(Optics::Relight {
                            height_from: HeightFrom::Luma,
                            light_dir: [math::cos(progress * 6.0), -0.5, 0.8],
                            color: Color::WHITE,
                            ambient: 0.35,
                            specular: 1.0,
                        }))
                        .blend_mode(BlendMode::Multiply),
                ])
            }
            _ => Node::from(stroke.write(reveal)),
        };
        Ok(Node::group([
            Node::from(atlas::VIEWPORT.fill(Color::WHITE)),
            ink,
        ]))
    })
}

fn configured(page: usize, seed: u32) -> motion::Result<BrushStrokeBuilder> {
    let base = BrushStroke::builder(stroke_path(page)?, 28.0, Color::from_hex(0x167d71))
        .material_work_limit(MATERIAL_WORK);
    Ok(match page {
        0 => base.pressure_curve(vec![
            (0.0, 0.15),
            (0.25, 1.8),
            (0.55, 0.3),
            (0.8, 1.5),
            (1.0, 0.1),
        ]),
        1 => base.pressure_from(|progress| {
            0.3 + 0.9 * math::sin(progress * std::f64::consts::TAU * 3.0).powi(2)
        }),
        2 => base.nib(0.18, 0.7).nib_mode(NibMode::Fixed(0.7)),
        3 => base.nib(0.18, 0.0).nib_mode(NibMode::FollowPath(0.5)),
        4 => base
            .sample_limit(BRISTLE_SAMPLES)
            .bristles(5, 0.8, 0.12, seed)
            .ink(Ink::new(seed).dryness(0.25)),
        5 => base.caps(BrushCap::Tapered(75.0), BrushCap::Flat),
        6 => base.paint(
            LinearGradient::new(Point::new(40.0, 0.0), Point::new(472.0, 0.0))
                .units(GradientUnits::UserSpace)
                .stop(0.0, Color::from_hex(0x3564c4))
                .stop(0.5, Color::from_hex(0xd64b36))
                .stop(1.0, Color::from_hex(0xe4aa37)),
        ),
        7 => base
            .ink_load(1.0, 0.05)
            .ink(Ink::new(seed).dryness(0.3).fibers(1.0)),
        8 => base.ink(Ink::new(seed).dryness(0.75).fibers(1.0)),
        9 => base.ink(Ink::new(seed).granulation(0.8).edge_roughness(3.0)),
        10 => base.ink(Ink::new(0).paper(paper(seed)?)),
        11 => base.ink(Ink::new(seed).granulation(0.35).build_up(0.8)),
        12 => base.ink(Ink::new(0).edge_darkening(0.9).edge_anisotropy(3.0)),
        13 => base.wobble(5.0, 0.06, seed),
        _ => base,
    })
}

fn stroke_path(page: usize) -> motion::Result<Path2D> {
    let mut path = Path2D::new();
    if page == 17 {
        path.try_move_to(Point::new(48.0, 180.0))?;
        path.try_cubic_to(
            Point::new(120.0, 30.0),
            Point::new(160.0, 240.0),
            Point::new(230.0, 140.0),
        )?;
        path.try_move_to(Point::new(285.0, 180.0))?;
        path.try_cubic_to(
            Point::new(330.0, 40.0),
            Point::new(400.0, 235.0),
            Point::new(465.0, 130.0),
        )?;
    } else {
        path.try_move_to(Point::new(48.0, 190.0))?;
        path.try_cubic_to(
            Point::new(130.0, 18.0),
            Point::new(390.0, 285.0),
            Point::new(280.0, 150.0),
        )?;
        path.try_cubic_to(
            Point::new(170.0, 10.0),
            Point::new(350.0, 85.0),
            Point::new(464.0, 190.0),
        )?;
    }
    Ok(path)
}

fn paper(seed: u32) -> motion::Result<Paper> {
    let mut pixels = Vec::with_capacity(16 * 16 * 4);
    for row in 0..16 {
        for column in 0..16 {
            let value = (motion::noise::hash2(column, row, seed) & 255) as u8;
            pixels.extend_from_slice(&[value, value, value, 255]);
        }
    }
    Ok(Paper::Texture {
        texture: Arc::new(ImageAsset::from_rgba8(16, 16, pixels)?),
        grain_scale: 2.0,
        absorbency: 0.8,
    })
}

fn recording() -> motion::Result<BrushStroke> {
    let samples = [
        (48.0, 190.0, 0.2, 0.0),
        (140.0, 100.0, 1.4, 0.8),
        (256.0, 160.0, 0.6, 1.5),
        (256.0, 160.0, 0.6, 2.5),
        (380.0, 105.0, 1.3, 3.3),
        (464.0, 190.0, 0.1, 4.0),
    ]
    .map(|(left, top, pressure, time)| InputSample {
        position: Point::new(left, top),
        pressure,
        tilt: time * 0.5,
        time: time.seconds(),
    });
    Ok(
        BrushStrokeBuilder::from_samples(&samples, 26.0, Color::from_hex(0x3564c4))?
            .nib(0.35, 0.0)
            .nib_mode(NibMode::Tilt)
            .material_work_limit(MATERIAL_WORK)
            .build()?,
    )
}

fn pigment(progress: f64) -> motion::Result<Node> {
    let first = Color::from_hex(0xffda28);
    let second = Color::from_hex(0x2466cc);
    let mut nodes = Vec::new();
    for (row, mode) in [PigmentMix::Linear, PigmentMix::KubelkaMunk]
        .into_iter()
        .enumerate()
    {
        for column in 0..16 {
            let left = 40.0 + f64::from(column) * 27.0;
            let amount = (f64::from(column) / 15.0 * (0.4 + 0.6 * progress)).min(1.0);
            nodes.push(Node::from(
                Rect::new(
                    left,
                    60.0 + row as f64 * 112.0,
                    left + 27.0,
                    132.0 + row as f64 * 112.0,
                )
                .fill(mode.mix(first, second, amount)?),
            ));
        }
        nodes.push(atlas::text(
            ["Linear light", "Pigment approximation"][row],
            14.0,
            atlas::DARK,
            40.0,
            140.0 + row as f64 * 112.0,
        ));
    }
    Ok(Node::group(nodes))
}
