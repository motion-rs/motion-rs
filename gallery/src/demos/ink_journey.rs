//! Nostalgic home-movie study: a brush-and-ink landscape on aged paper, with a
//! typewriter caption, an orange date stamp, drifting dust, scratches, light
//! leaks and flickering film grain over a slow camera drift.

use super::{frame_seed, label};
use crate::{DemoMetadata, DemoParameters};
use motion::prelude::*;
use motion::vector::ink::{BrushStroke, Ink};

const PAPER: u32 = 0xe6d5ae;

const AGED: u32 = 0xb99a62;

const INK: u32 = 0x2b1d14;

const RUST: u32 = 0xb4522c;

const STAMP: u32 = 0xff7a2f;

const TYPEWRITER: &str = "American Typewriter";

const CAPTION: &str = "THE LONG WAY HOME";

pub(super) const METADATA: DemoMetadata = DemoMetadata::new(
    "ink-journey",
    "The Long Way Home",
    "Nostalgia",
    &[
        "brush-strokes",
        "pressure-nibs",
        "ink-material",
        "path-growth",
        "camera",
        "filters",
    ],
);

pub(super) fn timeline(parameters: DemoParameters) -> motion::Result<Timeline> {
    Timeline::new(Size::new(960.0, 540.0), 30)
        .track(|track| {
            track.scene(parameters.seconds.seconds(), move |context| {
                frame(context, parameters)
            })
        })
        .build()
}

fn frame(context: &SceneContext<'_>, parameters: DemoParameters) -> Node {
    frame_mode::<true>(context, parameters)
}

fn color(hex: u32) -> Color {
    Color::from_hex(hex)
}

fn frame_mode<const RETAIN_COMPLETE: bool>(
    context: &SceneContext<'_>,
    parameters: DemoParameters,
) -> Node {
    let progress = context.progress();
    let frame = frame_seed(context);
    let flicker = (context.random(("flicker", frame)) - 0.5) * 0.1;
    let artwork = landscape::<RETAIN_COMPLETE>(context, parameters, progress);
    let layers = vec![
        paper(context),
        artwork,
        Node::group(caption(progress)),
        dust(context, frame),
        light_leak(context),
    ];
    Node::group(layers)
        .clip_with(context.bounds(), ClipMode::Geometric)
        .filter(Filter::ColorAdjust {
            exposure: flicker,
            contrast: 0.94,
            saturation: 0.82,
            hue_degrees: 0.0,
        })
        .filter(Filter::Optics(motion::Optics::LensVignette {
            angle_of_view: 75.0,
            strength: 0.6,
            roundness: 0.0,
        }))
        .filter(Filter::grain(parameters.seed ^ frame, 0.05))
}

/// Aged stock: a warm center that burns to brown at the corners, with a deckle border.
fn paper(context: &SceneContext<'_>) -> Node {
    context.memo("paper-stock", || {
        Node::group([
            Node::from(
                context.bounds().fill(
                    RadialGradient::new(Point::new(480.0, 250.0), 620.0)
                        .stop(0.0, color(PAPER))
                        .stop(0.7, color(0xd2bb88))
                        .stop(1.0, color(AGED))
                        .units(GradientUnits::UserSpace),
                ),
            ),
            Node::from(
                Rect::new(24.0, 24.0, 936.0, 516.0).stroke(1.2, color(INK).with_alpha(0.55)),
            ),
            Node::from(
                Rect::new(30.0, 30.0, 930.0, 510.0).stroke(0.6, color(INK).with_alpha(0.35)),
            ),
        ])
    })
}

fn landscape<const RETAIN_COMPLETE: bool>(
    context: &SceneContext<'_>,
    parameters: DemoParameters,
    progress: f64,
) -> Node {
    let ink = color(INK);
    let material = Ink::new(parameters.seed as u32)
        .dryness(0.1)
        .fibers(0.75)
        .granulation(0.2)
        .paper(71, 0.2, 1.2)
        .edge_pooling(0.22, 0.8);
    let mut strokes = Vec::new();
    for index in 0..6 {
        let path = context.cache(("landscape-ridge", index), || ridge(index));
        let reveal = ((progress - index as f64 * 0.075) / 0.43).clamp(0.0, 1.0);
        let complete = || {
            BrushStroke::new((*path).clone(), 6.0 + index as f64 * 1.1, ink)
                .pressure(0.2, 1.25, 0.15)
                .nib(0.42, -0.35)
                .ink(material)
                .ink_load(1.0, 0.75)
                .material_work_limit(64_000_000)
        };
        strokes.push(if RETAIN_COMPLETE && reveal == 1.0 {
            context.memo(("landscape-complete", parameters.seed, index), complete)
        } else if RETAIN_COMPLETE {
            // The full-stroke material does not depend on reveal, and `write` keeps it, so
            // a retained stroke prepares the material once instead of on every frame.
            let stroke = context.cache(("landscape-stroke", parameters.seed, index), complete);
            Node::from((*stroke).clone().write(reveal))
        } else {
            Node::from(complete().write(reveal))
        });
    }
    for index in 0..7 {
        let path = context.cache(("landscape-detail", index), || {
            let mut path = Path2D::new();
            path.move_to(Point::new(
                300.0 + index as f64 * 25.0,
                395.0 + index as f64 * 6.0,
            ))
            .quadratic_to(
                Point::new(480.0, 385.0 + index as f64 * 5.0),
                Point::new(850.0 - index as f64 * 20.0, 394.0 + index as f64 * 6.0),
            );
            path
        });
        strokes.push(
            path.as_ref()
                .clone()
                .stroke(0.75, ink.with_alpha(0.25))
                .opacity(((progress - 0.45) * 4.0).clamp(0.0, 1.0)),
        );
    }
    // Diluted ink washes give the far ridges body beneath the brush lines.
    let mut washes = Vec::new();
    for index in [0usize, 2, 4] {
        let path = context.cache(("landscape-wash", index), || {
            let mut shape = ridge(index);
            shape
                .line_to(Point::new(945.0, 460.0))
                .line_to(Point::new(35.0, 460.0))
                .close();
            shape
        });
        let top = 120.0 + index as f64 * 24.0;
        washes.push(
            Node::from(
                path.as_ref().clone().fill(
                    LinearGradient::new(Point::new(0.0, top), Point::new(0.0, 420.0))
                        .stop(0.0, color(0x5a3d26).with_alpha(0.42 - index as f64 * 0.06))
                        .stop(1.0, ink.with_alpha(0.0))
                        .units(GradientUnits::UserSpace),
                ),
            )
            .opacity(((progress - 0.12 - index as f64 * 0.05) * 3.0).clamp(0.0, 1.0)),
        );
    }
    strokes.splice(0..0, washes);
    let view = Camera {
        center: Point::new(480.0 + progress * 18.0, 270.0 - progress * 6.0),
        zoom: 1.0 + progress * 0.035,
        rotation: 0.0,
    };
    let sun = Circle::new(Point::new(735.0, 173.0), 34.0)
        .fill(color(RUST))
        .opacity((progress * 5.0).clamp(0.0, 1.0))
        .blend_mode(BlendMode::Multiply);
    let birds = flock(progress);
    Node::group([sun, camera(context, Node::group(strokes), view), birds])
        .clip_with(Rect::new(48.0, 92.0, 912.0, 442.0), ClipMode::Geometric)
}

fn ridge(index: usize) -> Path2D {
    let mut path = Path2D::new();
    for sample in 0..=64 {
        let x = 35.0 + sample as f64 * 14.0;
        let peak = math::exp(-math::pow((x - 515.0 - index as f64 * 8.0) / 125.0, 2.0));
        let shoulder = math::exp(-math::pow((x - 300.0) / 95.0, 2.0));
        let y = 357.0 - index as f64 * 23.0 - peak * (91.0 + index as f64 * 7.0) - shoulder * 35.0
            + math::sin(x * 0.016 + index as f64 * 0.4) * 9.0;
        if sample == 0 {
            path.move_to(Point::new(x, y));
        } else {
            path.line_to(Point::new(x, y));
        }
    }
    path
}

/// Three gulls drifting across the sun, drawn as simple wing strokes.
fn flock(progress: f64) -> Node {
    let mut wings = Path2D::new();
    for (index, (x, y, size)) in [
        (560.0, 150.0, 9.0),
        (592.0, 132.0, 7.0),
        (528.0, 126.0, 6.0),
    ]
    .into_iter()
    .enumerate()
    {
        let flap = math::sin(progress * 60.0 + index as f64 * 1.7) * size * 0.35;
        let x = x + progress * 150.0;
        let y = y - progress * 24.0;
        wings
            .move_to(Point::new(x - size, y + flap))
            .quadratic_to(Point::new(x - size * 0.4, y - size * 0.5), Point::new(x, y))
            .quadratic_to(
                Point::new(x + size * 0.4, y - size * 0.5),
                Point::new(x + size, y + flap),
            );
    }
    Node::from(wings.stroke(1.3, color(INK).with_alpha(0.8)))
        .opacity(math::smoothstep(0.25, 0.4, progress))
}

/// A typed caption with a blinking block cursor, plus a rust stamp and date mark.
fn caption(progress: f64) -> Vec<Node> {
    let typed = ((progress - 0.04) / 0.22 * CAPTION.len() as f64)
        .clamp(0.0, CAPTION.len() as f64)
        .floor() as usize;
    let blink = (progress * 90.0).floor() as i64 % 2 == 0;
    let text = label(CAPTION, TYPEWRITER, 22.0, 3.0, FontWeight::BOLD, color(INK))
        .split(TextSplit::Characters)
        .visible_parts(typed);
    let cursor_x = 52.0 + typed as f64 * 16.8;
    let stamp = math::smoothstep(0.6, 0.72, progress);
    vec![
        text.translate(52.0, 52.0),
        Rect::new(cursor_x, 54.0, cursor_x + 11.0, 76.0)
            .fill(color(INK))
            .opacity(if blink && progress < 0.34 { 0.85 } else { 0.0 }),
        label(
            "A landscape, one breath at a time.",
            TYPEWRITER,
            14.0,
            1.0,
            FontWeight::NORMAL,
            color(INK),
        )
        .translate(52.0, 468.0)
        .opacity(math::smoothstep(0.3, 0.45, progress)),
        Node::group([
            Node::from(Circle::new(Point::new(0.0, 0.0), 30.0).stroke(2.2, color(RUST))),
            Node::from(Circle::new(Point::new(0.0, 0.0), 24.0).stroke(0.8, color(RUST))),
            label("HOME", TYPEWRITER, 12.0, 2.0, FontWeight::BOLD, color(RUST))
                .translate(-17.0, -9.0),
        ])
        .rotate(-0.22)
        .translate(844.0, 62.0)
        .scale(1.5 - 0.5 * stamp)
        .opacity(stamp * 0.8)
        .blend_mode(BlendMode::Multiply),
        label(
            "'87  10  27",
            "Menlo",
            18.0,
            2.0,
            FontWeight::BOLD,
            color(STAMP),
        )
        .translate(770.0, 474.0)
        .opacity(math::smoothstep(0.5, 0.6, progress))
        .filter(Filter::Glow {
            threshold: 0.2,
            sigma: 3.0,
            intensity: 0.9,
        }),
    ]
}

/// Dust, hairs and the occasional vertical scratch, reseeded every frame so the
/// damage jumps like emulsion and still replays exactly after a seek.
fn dust(context: &SceneContext<'_>, frame: u64) -> Node {
    let mut specks = Vec::new();
    for speck in 0..22 {
        let x = context.random(("dust-x", frame, speck)) * 960.0;
        let y = context.random(("dust-y", frame, speck)) * 540.0;
        let size = 0.6 + context.random(("dust-size", frame, speck)) * 1.8;
        let shade = if speck % 4 == 0 {
            Color::WHITE.with_alpha(0.55)
        } else {
            color(INK).with_alpha(0.55)
        };
        specks.push(Node::from(Circle::new(Point::new(x, y), size).fill(shade)));
    }
    if context.random(("scratch", frame / 2)) > 0.62 {
        let x = context.random(("scratch-x", frame / 2)) * 960.0;
        let mut path = Path2D::new();
        path.move_to(Point::new(x, 0.0))
            .line_to(Point::new(x + 1.5, 540.0));
        specks.push(Node::from(path.stroke(0.8, Color::WHITE.with_alpha(0.35))));
    }
    if context.random(("hair", frame / 3)) > 0.8 {
        let x = context.random(("hair-x", frame / 3)) * 860.0 + 40.0;
        let y = context.random(("hair-y", frame / 3)) * 440.0 + 40.0;
        let mut hair = Path2D::new();
        hair.move_to(Point::new(x, y)).quadratic_to(
            Point::new(x + 22.0, y - 14.0),
            Point::new(x + 40.0, y + 6.0),
        );
        specks.push(Node::from(hair.stroke(0.7, color(INK).with_alpha(0.6))));
    }
    Node::group(specks)
}

/// A slow, warm light leak that breathes in from the upper-right corner.
fn light_leak(context: &SceneContext<'_>) -> Node {
    let breath = 0.5 + 0.5 * math::sin(context.seconds() * 0.9);
    Node::from(
        context.bounds().fill(
            RadialGradient::new(Point::new(960.0, 0.0), 420.0 + breath * 80.0)
                .stop(0.0, color(0xff9a4a).with_alpha(0.55))
                .stop(0.5, color(0xff5a2a).with_alpha(0.16))
                .stop(1.0, color(0xff5a2a).with_alpha(0.0))
                .units(GradientUnits::UserSpace),
        ),
    )
    .blend_mode(BlendMode::Screen)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn completed_brush_reuse_matches_original_frames_and_seek() -> motion::Result {
        let font = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../tests/fixtures/fonts/NotoSans-Regular.ttf");
        let options = crate::render_options(Some(&font))?.size(PixelSize::new(192, 108));
        let parameters = DemoParameters::default();
        let optimized = timeline(parameters)?;
        let reference = Timeline::new(Size::new(960.0, 540.0), 30)
            .track(|track| {
                track.push(
                    (move |context: &SceneContext<'_>| frame_mode::<false>(context, parameters))
                        .with_duration(8.seconds()),
                )
            })
            .build()?;
        let mut actual =
            motion::export::composition(&optimized, &options)?.frame_renderer(optimized.size())?;
        let mut expected =
            motion::export::composition(&reference, &options)?.frame_renderer(reference.size())?;
        for index in [220, 180, 40, 90, 220, 40, 180] {
            let actual = actual.render_frame(index, |context| optimized.render(context))?;
            let expected = expected.render_frame(index, |context| reference.render(context))?;
            assert_eq!(actual.pixels(), expected.pixels(), "frame {index}");
        }
        Ok(())
    }
}
