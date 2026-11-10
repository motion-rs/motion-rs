//! Vector geometry, paint, SVG preparation and morphing as isolated animated studies.

use super::atlas::{self, Page};
use crate::{DemoMetadata, DemoParameters};
use motion::eager::{Morph, WriteOptions};
use motion::prelude::*;
use motion::svg::SvgAsset;

pub(super) const METADATA: DemoMetadata = DemoMetadata::new(
    "vector-atlas",
    "Vector Atlas",
    "Vector",
    &[
        "paths",
        "stroke-styles",
        "gradients",
        "svg",
        "vector-morph",
        "vector-write",
    ],
);

const SOURCE: &[u8] = br##"<svg xmlns="http://www.w3.org/2000/svg" width="512" height="304"><circle id="mark" cx="256" cy="152" r="92" fill="#66e0c2"/><path id="line" d="M190 150 L240 195 L322 104" fill="none" stroke="#17231f" stroke-width="12"/></svg>"##;

const TARGET: &[u8] = br##"<svg xmlns="http://www.w3.org/2000/svg" width="512" height="304"><path id="mark" d="M256 44 L376 152 L256 260 L136 152 Z" fill="#ffc777"/><path id="line" d="M190 190 L256 112 L322 190" fill="none" stroke="#17231f" stroke-width="12"/></svg>"##;

pub(super) const PAGES: &[Page] = &[
    Page {
        title: "Arc-length trim",
        lines: [
            "A cubic path is revealed",
            "by normalized arc distance",
            "rather than control-point time.",
        ],
    },
    Page {
        title: "Dash and phase",
        lines: [
            "Explicit dash intervals",
            "move along the same curve",
            "with a changing phase.",
        ],
    },
    Page {
        title: "Stroke caps",
        lines: [
            "Butt, round and square",
            "caps produce different ends",
            "for the same centerline.",
        ],
    },
    Page {
        title: "Stroke joins",
        lines: [
            "Miter, round and bevel",
            "joins resolve corners with",
            "different outline geometry.",
        ],
    },
    Page {
        title: "Fill rules",
        lines: [
            "Same-winding nested paths",
            "compare NonZero with",
            "EvenOdd hole formation.",
        ],
    },
    Page {
        title: "Linear gradient",
        lines: [
            "Color stops interpolate",
            "along an explicit moving",
            "line in local coordinates.",
        ],
    },
    Page {
        title: "Focal radial",
        lines: [
            "A displaced inner focus",
            "moves the highlight inside",
            "a bounded radial gradient.",
        ],
    },
    Page {
        title: "Sweep gradient",
        lines: [
            "Angular color intervals",
            "rotate around an explicit",
            "center through a full turn.",
        ],
    },
    Page {
        title: "Gradient extension",
        lines: [
            "Pad, repeat and reflect",
            "extend the same short",
            "gradient interval differently.",
        ],
    },
    Page {
        title: "Interpolation space",
        lines: [
            "The same encoded stops",
            "compare linear-light and",
            "sRGB color interpolation.",
        ],
    },
    Page {
        title: "SVG vector tree",
        lines: [
            "Original SVG geometry",
            "stays vector as the mark",
            "rotates and scales.",
        ],
    },
    Page {
        title: "Vector morph",
        lines: [
            "Matching vector identities",
            "interpolate geometry and",
            "paint between two marks.",
        ],
    },
    Page {
        title: "Vector writing",
        lines: [
            "Cumulative contour length",
            "draws temporary outlines",
            "then hands off to the fill.",
        ],
    },
];

pub(super) fn timeline(parameters: DemoParameters) -> motion::Result<Timeline> {
    let source = SvgAsset::from_bytes(SOURCE)?.tree;
    let target = SvgAsset::from_bytes(TARGET)?.tree;
    let morph = Morph::new(&source, &target)?;
    atlas::timeline(parameters, METADATA, PAGES, move |page, progress, _| {
        let color = Color::from_hex(atlas::MINT);
        let bounds = Rect::new(40.0, 52.0, 472.0, 252.0);
        Ok(match page {
            0 => curve()
                .stroke(7.0, color)
                .trim(0.0, 0.1 + 0.9 * progress)
                .into(),
            1 => curve()
                .stroke(7.0, color)
                .dash(&[18.0, 10.0, 3.0, 10.0], progress * 100.0)
                .into(),
            2 | 3 => stroke_styles(page, progress),
            4 => {
                let path = nested_path();
                Node::group([
                    path.clone()
                        .fill(color)
                        .fill_rule(FillRule::NonZero)
                        .translate(-120.0, 0.0),
                    path.fill(Color::from_hex(atlas::GOLD))
                        .fill_rule(FillRule::EvenOdd)
                        .translate(120.0, 0.0),
                ])
                .rotate((progress - 0.5) * 0.25)
            }
            5 => bounds
                .fill(
                    LinearGradient::new(
                        Point::new(40.0, 52.0),
                        Point::new(260.0 + progress * 210.0, 252.0),
                    )
                    .units(GradientUnits::UserSpace)
                    .stop(0.0, color)
                    .stop(0.5, Color::from_hex(atlas::BLUE))
                    .stop(1.0, Color::from_hex(0xd64b36)),
                )
                .into(),
            6 => bounds
                .fill(
                    RadialGradient::new(Point::new(256.0, 152.0), 210.0)
                        .units(GradientUnits::UserSpace)
                        .focal(Point::new(170.0 + progress * 150.0, 120.0))
                        .stop(0.0, Color::from_hex(atlas::GOLD))
                        .stop(1.0, Color::from_hex(0x3564c4)),
                )
                .into(),
            7 => bounds
                .fill(
                    SweepGradient::new(
                        Point::new(256.0, 152.0),
                        progress * 6.0,
                        progress * 6.0 + std::f64::consts::TAU,
                    )
                    .units(GradientUnits::UserSpace)
                    .stop(0.0, color)
                    .stop(0.5, Color::from_hex(atlas::GOLD))
                    .stop(1.0, color),
                )
                .into(),
            8 => extensions(progress),
            9 => interpolation(progress),
            10 => source
                .node()
                .rotate((progress - 0.5) * 0.9)
                .scale(0.8 + 0.2 * progress),
            11 => morph.sample(Easing::CubicInOut.apply(progress))?,
            _ => source.write(
                progress,
                WriteOptions {
                    width: 3.0,
                    outline_fraction: 0.8,
                },
            )?,
        })
    })
}

fn curve() -> Path2D {
    let mut path = Path2D::new();
    path.move_to(Point::new(48.0, 212.0)).cubic_to(
        Point::new(140.0, -20.0),
        Point::new(350.0, 330.0),
        Point::new(464.0, 80.0),
    );
    path
}

fn stroke_styles(page: usize, progress: f64) -> Node {
    let mut nodes = Vec::new();
    for index in 0..3 {
        let left = 40.0 + index as f64 * 160.0;
        let mut path = Path2D::new();
        path.move_to(Point::new(left, 198.0))
            .line_to(Point::new(left + 60.0, 96.0 + progress * 30.0))
            .line_to(Point::new(left + 120.0, 198.0));
        let cap = [LineCap::Butt, LineCap::Round, LineCap::Square][index];
        let join = [
            LineJoin::Miter { limit: 6.0 },
            LineJoin::Round,
            LineJoin::Bevel,
        ][index];
        nodes.push(Node::from(
            path.stroke(22.0, Color::from_hex(atlas::BLUE))
                .stroke_style(StrokeStyle {
                    width: 22.0,
                    cap: if page == 2 { cap } else { LineCap::Butt },
                    join: if page == 3 { join } else { LineJoin::Round },
                }),
        ));
        nodes.push(atlas::text(
            if page == 2 {
                ["Butt", "Round", "Square"][index]
            } else {
                ["Miter", "Round", "Bevel"][index]
            },
            15.0,
            0xb9cec5,
            left + 35.0,
            244.0,
        ));
    }
    Node::group(nodes)
}

fn nested_path() -> Path2D {
    let mut path = Path2D::new();
    for (left, top, right, bottom) in [(176.0, 72.0, 336.0, 232.0), (216.0, 112.0, 296.0, 192.0)] {
        path.move_to(Point::new(left, top))
            .line_to(Point::new(right, top))
            .line_to(Point::new(right, bottom))
            .line_to(Point::new(left, bottom))
            .close();
    }
    path
}

fn extensions(progress: f64) -> Node {
    let mut nodes = Vec::new();
    for (index, extend) in [
        GradientExtend::Pad,
        GradientExtend::Repeat,
        GradientExtend::Reflect,
    ]
    .into_iter()
    .enumerate()
    {
        let top = 40.0 + index as f64 * 80.0;
        let paint = LinearGradient::new(
            Point::new(160.0 + progress * 40.0, 0.0),
            Point::new(280.0 + progress * 40.0, 0.0),
        )
        .units(GradientUnits::UserSpace)
        .stop(0.0, Color::from_hex(atlas::BLUE))
        .stop(1.0, Color::from_hex(atlas::GOLD))
        .extend(extend);
        nodes.push(Node::from(
            Rect::new(36.0, top, 476.0, top + 44.0).fill(paint),
        ));
        nodes.push(atlas::text(
            ["Pad", "Repeat", "Reflect"][index],
            13.0,
            0xb9cec5,
            36.0,
            top + 48.0,
        ));
    }
    Node::group(nodes)
}

fn interpolation(progress: f64) -> Node {
    let mut nodes = Vec::new();
    for (index, space) in [GradientSpace::LinearRgb, GradientSpace::Srgb]
        .into_iter()
        .enumerate()
    {
        let top = 65.0 + index as f64 * 115.0;
        let paint = LinearGradient::new(
            Point::new(40.0, 0.0),
            Point::new(400.0 + progress * 60.0, 0.0),
        )
        .units(GradientUnits::UserSpace)
        .stop(0.0, Color::from_hex(0xd64b36))
        .stop(1.0, Color::from_hex(0x3564c4))
        .space(space);
        nodes.push(Node::from(
            Rect::new(40.0, top, 472.0, top + 65.0).fill(paint),
        ));
        nodes.push(atlas::text(
            ["Linear RGB", "sRGB"][index],
            14.0,
            0xb9cec5,
            40.0,
            top + 70.0,
        ));
    }
    Node::group(nodes)
}
