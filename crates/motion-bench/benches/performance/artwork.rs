//! Frozen performance benchmark artwork: the deterministic scene for every named case,
//! including the particle-count variant used for profiling.

use motion::extend::Transition;
use motion::particles::{Orbit, ParticleStyle, ParticleTrajectory};
use motion::prelude::*;
use motion::render::{MorphPath, SceneCache};
use motion::svg::SvgAsset;
use motion::vector::ink::{BrushStroke, Ink, InkFill};
use std::sync::{Arc, OnceLock};

pub(super) fn image(ring: bool) -> Node {
    static PHOTO: OnceLock<Arc<ImageAsset>> = OnceLock::new();

    static RING: OnceLock<Arc<ImageAsset>> = OnceLock::new();
    let asset = if ring {
        RING.get_or_init(|| {
            Arc::new(
                ImageAsset::from_bytes(include_bytes!("../../../../gallery/assets/ring.png"))
                    .unwrap(),
            )
        })
    } else {
        PHOTO.get_or_init(|| {
            Arc::new(
                ImageAsset::from_bytes(include_bytes!("../../../../gallery/assets/landscape.jpg"))
                    .unwrap(),
            )
        })
    };
    Node::from(
        Image::from_asset(asset.clone())
            .size(125.0, 80.0)
            .fit(Fit::Cover),
    )
}

pub(super) fn rectangle(bounds: Rect, color: Color) -> Node {
    Node::from(RoundedRect::new(bounds, 8.0).fill(color))
}

pub(super) fn artwork(kind: &str, context: &SceneContext<'_>) -> Node {
    artwork_with_particles(kind, context, 72)
}

pub(super) fn artwork_with_particles(
    kind: &str,
    context: &SceneContext<'_>,
    particle_count: usize,
) -> Node {
    if kind == "empty" {
        return Node::empty();
    }
    if matches!(kind, "path-sampling" | "path-trim") {
        let path = Path2D::parse_svg("M30 240Q130 20 250 150C340 330 440 20 610 170").unwrap();
        if kind == "path-trim" {
            let start = 0.05 + context.progress() * 0.2;
            return path
                .try_trim(start, 0.85)
                .unwrap()
                .stroke(3.0, Color::BLACK)
                .into();
        }
        let mut nodes = vec![Node::from(path.clone().stroke(2.0, Color::BLACK))];
        for index in 0..=32 {
            let (point, tangent) = path.try_point_at(index as f64 / 32.0).unwrap();
            nodes.push(Node::from(Circle::new(point, 2.5).fill(Color::BLACK)));
            let mut direction = Path2D::new();
            direction.try_move_to(point).unwrap();
            direction.try_line_to(point + tangent * 8.0).unwrap();
            nodes.push(Node::from(direction.stroke(1.0, Color::BLACK)));
        }
        return Node::group(nodes);
    }
    if kind == "morph-curves" {
        let source = Path2D::parse_svg("M40 180Q150 20 300 160C410 30 560 100 580 260L60 300Z M220 190C220 150 280 150 280 190C280 230 220 230 220 190Z").unwrap();
        let target = Path2D::parse_svg("M60 80C220 20 400 20 560 90Q610 190 500 290L100 280Z M240 170Q300 150 310 210Q270 250 240 170Z").unwrap();
        let (source, target) = MorphPath::pair(source, target).unwrap();
        return source
            .try_interpolate(&target, context.progress())
            .unwrap()
            .fill(Color::BLACK)
            .into();
    }
    if kind == "motion-blur" {
        return context
            .try_motion_blur(30.milliseconds(), 4, |sample| {
                artwork("shapes", sample).translate(libm::sin(sample.seconds() * 8.0) * 20.0, 0.0)
            })
            .unwrap();
    }
    if matches!(kind, "particles-orbit" | "particles-plexus") {
        let system = context.cache(
            ("benchmark-particles", kind.to_owned(), particle_count),
            || {
                ParticleTrajectory::new(
                    Orbit::new(Point::new(320.0, 180.0), 120.0, particle_count as u64)
                        .seed(17)
                        .lifetime(4.seconds()),
                )
                .style(if kind == "particles-plexus" {
                    ParticleStyle::Plexus {
                        radius: 2.0,
                        max_distance: 32.0,
                    }
                } else {
                    ParticleStyle::GlowDots { radius: 2.4 }
                })
            },
        );
        return Node::group([
            Node::from(context.bounds().fill(Color::BLACK)),
            system.render(context),
        ]);
    }
    if kind == "ink-long-stroke" {
        let path = Path2D::parse_svg("M20 240C150 230 200 30 330 90S490 310 620 120").unwrap();
        return BrushStroke::new(path, 8.0, Color::BLACK)
            .ink(
                Ink::new(17)
                    .granulation(0.3)
                    .dryness(0.2)
                    .paper(71, 0.2, 1.2),
            )
            .material_work_limit(100_000_000)
            .pressure(0.08, 1.1, 0.06)
            .ink_load(1.0, 0.3)
            .into();
    }
    if kind == "text-parts" {
        return Text::new("ROOM TO MOVE")
            .size(64.0)
            .weight(FontWeight::BOLD)
            .fill(Color::BLACK)
            .try_animate_parts_at(context, 60.milliseconds(), |part| {
                PartStyle::new().offset(0.0, libm::sin(part.time().as_seconds() * 3.0) * 20.0)
            })
            .unwrap()
            .translate(35.0, 140.0);
    }
    if kind == "svg" {
        let asset = context.cache("benchmark-svg", || {
            Arc::new(SvgAsset::from_bytes(br##"<svg xmlns="http://www.w3.org/2000/svg" width="300" height="160"><defs><linearGradient id="g"><stop stop-color="#188b75"/><stop offset="1" stop-color="#f47762"/></linearGradient></defs><path d="M10 80C60-20 240 180 290 80L240 150H60Z" fill="url(#g)"/><circle cx="150" cy="80" r="45" fill="none" stroke="black" stroke-width="3"/></svg>"##).unwrap())
        });
        return Svg::from_asset((*asset).clone())
            .fit(Size::new(560.0, 300.0))
            .try_node()
            .unwrap()
            .translate(40.0, 30.0);
    }
    let mut nodes = Vec::new();
    let progress = context.seconds() / 3.0;
    for item in 0..12 {
        let bounds = Rect::new(0.0, 0.0, 125.0, 80.0);
        let teal = Color::from_hex(0x178b79);
        let blue = Color::from_hex(0x327fc1);
        let group = || {
            Node::group([
                rectangle(Rect::new(8.0, 10.0, 78.0, 60.0), teal),
                rectangle(Rect::new(48.0, 24.0, 113.0, 69.0), blue),
            ])
            .opacity(0.65)
        };
        let path = || Path2D::parse_svg("M8 12C38 -8 70 85 115 30L105 75L12 65Z").unwrap();
        let ink = || {
            Ink::new(17)
                .granulation(0.4)
                .paper(211, 0.35, 1.5)
                .bleed(3.0)
                .edge_roughness(0.7)
                .edge_pooling(0.5, 2.0)
        };
        let node = match kind {
            "shapes" => rectangle(bounds, teal.with_alpha(190.0 / 255.0)),
            "text" => Node::from(
                Text::new("Motion 012345\nDeterministic ink")
                    .size(15.0)
                    .max_width(140.0)
                    .fill(Color::BLACK),
            ),
            "fill" => Node::from(path().fill(Color::BLACK)),
            "stroke" => {
                Node::from(BrushStroke::new(path(), 9.0, Color::BLACK).pressure(0.2, 1.2, 0.05))
            }
            "ink-fill" => Node::from(InkFill::new(path(), Color::BLACK, ink())),
            "ink-stroke" => Node::from(
                BrushStroke::new(path(), 9.0, Color::BLACK)
                    .pressure(0.2, 1.2, 0.05)
                    .ink_load(0.95, 0.2)
                    .ink(ink().dryness(0.5).fibers(1.0)),
            ),
            "image" => image(false),
            "image-alpha" => image(true).opacity(0.65),
            "mask-alpha" | "mask-luminance" => image(false)
                .mask(
                    if kind == "mask-alpha" {
                        rectangle(
                            Rect::new(10.0, 10.0, 110.0, 70.0),
                            Color::BLACK.with_alpha(0.8),
                        )
                    } else {
                        image(true)
                    },
                    if kind == "mask-alpha" {
                        MaskMode::Alpha
                    } else {
                        MaskMode::Luminance
                    },
                )
                .opacity(0.75),
            "mask-shapes" => group().mask(
                rectangle(
                    Rect::new(8.0, 5.0, 118.0, 75.0),
                    Color::rgb(100, 230, 40).with_alpha(0.75),
                ),
                MaskMode::Luminance,
            ),
            "cross-fade" | "wipe" | "push" => {
                let cache = SceneCache::new(0);
                let local = SceneContext::new(
                    context.time(),
                    context.frame_rate(),
                    Size::new(125.0, 80.0),
                    &cache,
                );
                let incoming = Node::group([rectangle(bounds, blue), image(true)]);
                match kind {
                    "cross-fade" => {
                        Transition::compose(&CrossFade, progress, image(false), incoming, &local)
                    }
                    "wipe" => Transition::compose(
                        &Wipe {
                            angle: 0.0,
                            feather: 0.0,
                        },
                        progress,
                        image(false),
                        incoming,
                        &local,
                    ),
                    _ => Transition::compose(
                        &Push {
                            direction: Direction::Up,
                        },
                        progress,
                        image(false),
                        incoming,
                        &local,
                    ),
                }
            }
            "isolation" => group(),
            "blur" => group().filter(Filter::Blur {
                sigma: 1.0 + progress,
            }),
            "shadow" => group().filter(Filter::DropShadow {
                offset: Vector2::new(4.5, 6.0),
                sigma: 2.0,
                color: Color::BLACK.with_alpha(150.0 / 255.0),
            }),
            "bloom" => group().filter(Filter::Glow {
                threshold: 0.2,
                sigma: 8.0,
                intensity: 0.8,
            }),
            "grade" => group().filter(Filter::ColorAdjust {
                exposure: 0.5,
                contrast: 1.0,
                saturation: 0.7,
                hue_degrees: 15.0,
            }),
            _ => unreachable!("validated benchmark case"),
        };
        nodes.push(
            node.rotate(libm::sin(context.seconds() * 0.09) * 0.1)
                .translate(
                    24.0 + (item % 4) as f64 * 150.0,
                    24.0 + (item / 4) as f64 * 108.0,
                ),
        );
    }
    Node::group(nodes)
}
