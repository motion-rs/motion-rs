//! Layer compositing comparisons using actual blend, mask and clip operations.

use super::atlas::{self, Page};
use crate::{DemoMetadata, DemoParameters};
use motion::prelude::*;

pub(super) const METADATA: DemoMetadata = DemoMetadata::new(
    "compositing-atlas",
    "Compositing Atlas",
    "Rendering",
    &[
        "blend-modes",
        "masks",
        "clips",
        "layer-opacity",
        "layer-order",
    ],
);

const MODES: [BlendMode; 17] = [
    BlendMode::SourceOver,
    BlendMode::Plus,
    BlendMode::Multiply,
    BlendMode::Screen,
    BlendMode::Darken,
    BlendMode::Lighten,
    BlendMode::Difference,
    BlendMode::Exclusion,
    BlendMode::Overlay,
    BlendMode::HardLight,
    BlendMode::SoftLight,
    BlendMode::ColorDodge,
    BlendMode::ColorBurn,
    BlendMode::Hue,
    BlendMode::Saturation,
    BlendMode::Color,
    BlendMode::Luminosity,
];

pub(super) const PAGES: &[Page] = &[
    Page {
        title: "Source over",
        lines: [
            "The foreground covers its",
            "backdrop according to alpha",
            "in linear working color.",
        ],
    },
    Page {
        title: "Plus",
        lines: [
            "Premultiplied colors add",
            "light energy without clipping",
            "the working RGB channels.",
        ],
    },
    Page {
        title: "Multiply",
        lines: [
            "Source and backdrop colors",
            "multiply in their overlap;",
            "white leaves color unchanged.",
        ],
    },
    Page {
        title: "Screen",
        lines: [
            "The inverse-product formula",
            "brightens the overlapping",
            "source and backdrop colors.",
        ],
    },
    Page {
        title: "Darken",
        lines: [
            "Each channel retains the",
            "darker straight-color value",
            "inside the overlap.",
        ],
    },
    Page {
        title: "Lighten",
        lines: [
            "Each channel retains the",
            "lighter straight-color value",
            "inside the overlap.",
        ],
    },
    Page {
        title: "Difference",
        lines: [
            "Absolute channel differences",
            "highlight where the source",
            "and backdrop disagree.",
        ],
    },
    Page {
        title: "Exclusion",
        lines: [
            "A softer difference formula",
            "combines both source colors",
            "without absolute-value edges.",
        ],
    },
    Page {
        title: "Overlay",
        lines: [
            "Backdrop channels choose",
            "between multiply and screen",
            "around their midpoint.",
        ],
    },
    Page {
        title: "Hard light",
        lines: [
            "Source channels choose",
            "between multiply and screen",
            "around their midpoint.",
        ],
    },
    Page {
        title: "Soft light",
        lines: [
            "A piecewise lighting curve",
            "gently adjusts contrast",
            "using the foreground color.",
        ],
    },
    Page {
        title: "Color dodge",
        lines: [
            "Bright source channels",
            "lift the backdrop through",
            "bounded division endpoints.",
        ],
    },
    Page {
        title: "Color burn",
        lines: [
            "Dark source channels",
            "deepen the backdrop through",
            "the inverse-ratio formula.",
        ],
    },
    Page {
        title: "Hue",
        lines: [
            "Source hue combines with",
            "the backdrop saturation",
            "and backdrop luminosity.",
        ],
    },
    Page {
        title: "Saturation",
        lines: [
            "Source saturation changes",
            "the backdrop while keeping",
            "its hue and luminosity.",
        ],
    },
    Page {
        title: "Color",
        lines: [
            "Source hue and saturation",
            "replace backdrop chroma",
            "while retaining luminosity.",
        ],
    },
    Page {
        title: "Luminosity",
        lines: [
            "Source luminosity changes",
            "the backdrop while keeping",
            "its hue and saturation.",
        ],
    },
    Page {
        title: "Alpha mask",
        lines: [
            "Only mask opacity controls",
            "the retained source; black",
            "can still be fully opaque.",
        ],
    },
    Page {
        title: "Luminance mask",
        lines: [
            "Linear mask luminance",
            "controls source coverage;",
            "white and black differ.",
        ],
    },
    Page {
        title: "Inverted alpha",
        lines: [
            "Opaque mask regions erase",
            "the source while transparent",
            "regions preserve it.",
        ],
    },
    Page {
        title: "Inverted luminance",
        lines: [
            "Dark mask regions retain",
            "the source while bright",
            "regions attenuate it.",
        ],
    },
    Page {
        title: "Coverage clip",
        lines: [
            "Source and clip coverages",
            "are rasterized separately",
            "before multiplication.",
        ],
    },
    Page {
        title: "Geometric clip",
        lines: [
            "Geometry is intersected",
            "before quantization rather",
            "than multiplying coverages.",
        ],
    },
    Page {
        title: "Group opacity",
        lines: [
            "Per-child alpha overlaps",
            "differ from fading the",
            "completed group just once.",
        ],
    },
    Page {
        title: "Sibling order",
        lines: [
            "Sibling-local z-index",
            "changes which overlapping",
            "shape is painted last.",
        ],
    },
    Page {
        title: "Isolation and filters",
        lines: [
            "Blur each child separately",
            "or blur their assembled",
            "group: overlap differs.",
        ],
    },
];

pub(super) fn timeline(parameters: DemoParameters) -> motion::Result<Timeline> {
    Ok(atlas::timeline(
        parameters,
        METADATA,
        PAGES,
        |page, progress, _| {
            if page < MODES.len() {
                return Ok(layers(progress, MODES[page], 1.0));
            }
            let source = atlas::specimen(progress);
            let processed = match page {
                17..=20 => {
                    let mode = [
                        MaskMode::Alpha,
                        MaskMode::Luminance,
                        MaskMode::InvertedAlpha,
                        MaskMode::InvertedLuminance,
                    ][page - 17];
                    let mask = Circle::new(Point::new(150.0 + 212.0 * progress, 152.0), 100.0)
                        .fill(
                            LinearGradient::new(Point::new(110.0, 0.0), Point::new(400.0, 0.0))
                                .units(GradientUnits::UserSpace)
                                .stop(0.0, Color::BLACK)
                                .stop(1.0, Color::WHITE),
                        );
                    source.clone().mask(mask, mode)
                }
                21 | 22 => source.clone().clip_with(
                    Circle::new(Point::new(180.0 + 150.0 * progress, 152.0), 105.0),
                    if page == 21 {
                        ClipMode::Coverage
                    } else {
                        ClipMode::Geometric
                    },
                ),
                23 => {
                    let background = Node::from(
                        Rect::new(48.0, 48.0, 464.0, 256.0).fill(Color::from_hex(0xc9d9cf)),
                    );
                    return Ok(atlas::pair(
                        Node::group([background.clone(), Node::group(shapes(progress, 0.5))]),
                        Node::group([background, Node::group(shapes(progress, 1.0)).opacity(0.5)]),
                        "Child alpha",
                        "Group opacity",
                    ));
                }
                24 => {
                    let back = Rect::new(90.0, 70.0, 300.0, 235.0)
                        .fill(Color::from_hex(atlas::MINT))
                        .z_index(1);
                    let front = Circle::new(
                        Point::new(290.0 + math::sin(progress * 6.0) * 40.0, 152.0),
                        84.0,
                    )
                    .fill(Color::from_hex(atlas::GOLD))
                    .z_index(if progress < 0.5 { 0 } else { 2 });
                    return Ok(Node::group([back, front]));
                }
                _ => {
                    let children = shapes(progress, 1.0);
                    let blur = Filter::Blur { sigma: 12.0 };
                    return Ok(atlas::pair(
                        Node::group(children.clone()).filter(blur.clone()),
                        Node::group(children.map(|child| child.filter(blur.clone()))),
                        "Group blur",
                        "Child blur",
                    ));
                }
            };
            Ok(atlas::comparison(source, processed))
        },
    ))
}

fn layers(progress: f64, mode: BlendMode, alpha: f64) -> Node {
    let [back, front] = shapes(progress, alpha);
    Node::group([
        Node::from(Rect::new(48.0, 48.0, 464.0, 256.0).fill(Color::from_hex(0xc9d9cf))),
        back,
        front.blend_mode(mode),
    ])
}

fn shapes(progress: f64, alpha: f64) -> [Node; 2] {
    [
        Circle::new(Point::new(210.0, 152.0), 86.0)
            .fill(
                LinearGradient::new(Point::new(124.0, 0.0), Point::new(296.0, 0.0))
                    .units(GradientUnits::UserSpace)
                    .stop(0.0, Color::from_hex(0x3564c4))
                    .stop(1.0, Color::from_hex(atlas::MINT)),
            )
            .opacity(alpha),
        Circle::new(
            Point::new(290.0 + math::sin(progress * 6.0) * 36.0, 152.0),
            86.0,
        )
        .fill(Color::from_hex(0xe37659))
        .opacity(alpha),
    ]
}
