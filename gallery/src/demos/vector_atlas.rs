//! Vector geometry, paint, SVG preparation and morphing as isolated animated studies.

use super::atlas::{self, Page};
use crate::{DemoMetadata, DemoParameters};
use motion::Perspective;
use motion::eager::{Morph, WriteOptions};
use motion::math;
use motion::prelude::*;
use motion::render::RasterLimits;
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
        "path-boolean",
        "mesh-gradient",
        "perspective",
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
    Page {
        title: "Path booleans",
        lines: [
            "Union, intersection,",
            "difference and xor combine",
            "two moving filled shapes.",
        ],
    },
    Page {
        title: "Mesh gradient",
        lines: [
            "A lattice of colored points",
            "blends smoothly while its",
            "interior points drift.",
        ],
    },
    Page {
        title: "Planar perspective",
        lines: [
            "A flat card turns about its",
            "pivot and projects through",
            "a pinhole camera.",
        ],
    },
];

pub(super) fn timeline(parameters: DemoParameters) -> Result<Timeline, motion::ErrorReport> {
    let source = SvgAsset::from_bytes(SOURCE)?.tree;
    let target = SvgAsset::from_bytes(TARGET)?.tree;
    let morph = Morph::new(&source, &target)?;
    atlas::timeline(parameters, METADATA, PAGES, move |page, progress, _| {
        let color = Color::from_hex(atlas::MINT);
        let bounds = Rect::new(40.0, 52.0, 472.0, 252.0);
        Ok(match page {
            0 => curve()?
                .stroke(7.0, color)
                .trim(0.0, 0.1 + 0.9 * progress)
                .into(),
            1 => curve()?
                .stroke(7.0, color)
                .dash(&[18.0, 10.0, 3.0, 10.0], progress * 100.0)
                .into(),
            2 | 3 => stroke_styles(page, progress)?,
            4 => {
                let path = nested_path()?;
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
            12 => source.write(
                progress,
                WriteOptions {
                    width: 3.0,
                    outline_fraction: 0.8,
                },
            )?,
            13 => booleans(progress)?,
            14 => mesh(bounds, progress)?,
            _ => perspective(progress)?,
        })
    })
}

fn curve() -> Result<Path2D, ValidationError> {
    Path2D::builder()
        .move_to(Point::new(48.0, 212.0))
        .cubic_to(
            Point::new(140.0, -20.0),
            Point::new(350.0, 330.0),
            Point::new(464.0, 80.0),
        )
        .build()
}

fn stroke_styles(page: usize, progress: f64) -> motion::Result<Node> {
    let mut nodes = Vec::new();
    for index in 0..3 {
        let left = 40.0 + index as f64 * 160.0;
        let path = Path2D::builder()
            .move_to(Point::new(left, 198.0))
            .line_to(Point::new(left + 60.0, 96.0 + progress * 30.0))
            .line_to(Point::new(left + 120.0, 198.0))
            .build()?;
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
    Ok(Node::group(nodes))
}

fn nested_path() -> Result<Path2D, ValidationError> {
    let mut path = Path2D::new();
    for (left, top, right, bottom) in [(176.0, 72.0, 336.0, 232.0), (216.0, 112.0, 296.0, 192.0)] {
        path.try_move_to(Point::new(left, top))?;
        path.try_line_to(Point::new(right, top))?;
        path.try_line_to(Point::new(right, bottom))?;
        path.try_line_to(Point::new(left, bottom))?;
        path.try_close()?;
    }
    Ok(path)
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

fn booleans(progress: f64) -> motion::Result<Node> {
    let ops = [
        (PathBooleanOp::Union, "Union"),
        (PathBooleanOp::Intersection, "Intersection"),
        (PathBooleanOp::Difference, "Difference"),
        (PathBooleanOp::Xor, "Xor"),
    ];
    let mut nodes = Vec::new();
    for (index, (op, name)) in ops.into_iter().enumerate() {
        let left = 20.0 + index as f64 * 122.0;
        let center = Point::new(left + 56.0, 140.0);
        let circle = Path2D::builder()
            .arc_to(
                Point::new(center.x - 14.0, center.y),
                Vector2::new(34.0, 34.0),
                0.0,
                std::f64::consts::TAU,
            )
            .close()
            .build()?;
        let square = Path2D::from_rect(Rect::new(-30.0, -30.0, 30.0, 30.0))?.transformed(
            &Transform::rotate(progress * std::f64::consts::TAU).then(Transform::translate(
                center.x + 14.0 + 10.0 * math::sin(progress * std::f64::consts::TAU),
                center.y,
            )),
        )?;
        let result = circle.boolean(&square, op, PathBooleanOptions::default())?;
        nodes.push(Node::from(circle.stroke(1.5, Color::from_hex(atlas::BLUE))));
        nodes.push(Node::from(square.stroke(1.5, Color::from_hex(atlas::GOLD))));
        nodes.push(
            result
                .fill(Color::from_hex(atlas::MINT))
                .translate(0.0, 70.0),
        );
        nodes.push(atlas::text(name, 13.0, 0xb9cec5, left + 8.0, 262.0));
    }
    Ok(Node::group(nodes))
}

fn mesh(bounds: Rect, progress: f64) -> motion::Result<Node> {
    let phase = progress * std::f64::consts::TAU;
    let palette = [
        0x66e0c2, 0x8bd5f3, 0xffc777, 0xd64b36, 0x3564c4, 0x66e0c2, 0xffc777, 0x8bd5f3, 0xd64b36,
    ];
    let vertices = palette
        .iter()
        .enumerate()
        .map(|(index, color)| {
            let next = palette[(index + 1) % palette.len()];
            let (column, row) = ((index % 3) as f64, (index / 3) as f64);
            let drift = if index == 4 { 0.18 } else { 0.0 };
            MeshVertex {
                position: Point::new(
                    column * 0.5 + drift * math::cos(phase),
                    row * 0.5 + drift * math::sin(phase),
                ),
                color: Color::from_hex(*color).interpolate(&Color::from_hex(next), progress),
            }
        })
        .collect();
    let mesh = MeshGradient::new(3, 3, vertices)?;
    let pattern = mesh.pattern(bounds, PixelSize::new(128, 76), 6, RasterLimits::default())?;
    Ok(bounds.fill(pattern).into())
}

fn perspective(progress: f64) -> motion::Result<Node> {
    let camera = Perspective::new(
        Point::new(256.0, 152.0),
        0.25 * math::sin(progress * std::f64::consts::TAU),
        (progress - 0.5) * 2.2,
        0.0,
        520.0,
    )?;
    let mut parts = vec![
        (
            Path2D::from_rect(Rect::new(146.0, 82.0, 366.0, 222.0))?,
            if camera.faces_camera() {
                atlas::GOLD
            } else {
                0x8a6d3b
            },
        ),
        (
            Path2D::from_rect(Rect::new(166.0, 102.0, 266.0, 112.0))?,
            atlas::DARK,
        ),
        (
            Path2D::from_rect(Rect::new(166.0, 122.0, 226.0, 132.0))?,
            atlas::DARK,
        ),
    ];
    parts.push((
        Path2D::builder()
            .arc_to(
                Point::new(316.0, 172.0),
                Vector2::new(32.0, 32.0),
                0.0,
                std::f64::consts::TAU,
            )
            .close()
            .build()?,
        atlas::MINT,
    ));
    let mut nodes = Vec::new();
    for (path, color) in parts {
        nodes.push(Node::from(
            camera.project_path(&path, 8)?.fill(Color::from_hex(color)),
        ));
    }
    Ok(Node::group(nodes))
}
