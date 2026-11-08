//! Cyberpunk night study: a synthwave horizon, interfering neon ribbons and rain
//! over a city skyline, finished with bloom, lens fringing, glitch slices and
//! scanlines across three CrossFade-linked acts.

use super::{frame_seed, label};
use crate::{DemoMetadata, DemoParameters};
use motion::prelude::*;
use std::f64::consts::TAU;

const NIGHT: u32 = 0x07040f;

const DUSK: u32 = 0x1d0636;

const MAGENTA: u32 = 0xff2a9d;

const CYAN: u32 = 0x18f0ff;

const VIOLET: u32 = 0x7a2cff;

const SUN: u32 = 0xffe94d;

const HORIZON: f64 = 318.0;

const MONO: &str = "Menlo";

pub(super) const METADATA: DemoMetadata = DemoMetadata::new(
    "chromatic-tide",
    "Chromatic Tide",
    "Cyberpunk",
    &[
        "gradients",
        "masks",
        "blend-modes",
        "filters",
        "scene-windows",
    ],
);

pub(super) fn timeline(parameters: DemoParameters) -> motion::Result<Timeline> {
    let duration = parameters.seconds.seconds();
    Ok(Timeline::new(Size::new(960.0, 540.0), 30)
        .track(|track| track.scene(duration, move |context| film(context, parameters.seed))))
}

fn color(hex: u32) -> Color {
    Color::from_hex(hex)
}

fn rect_path(path: &mut Path2D, left: f64, top: f64, right: f64, bottom: f64) {
    path.move_to(Point::new(left, top))
        .line_to(Point::new(right, top))
        .line_to(Point::new(right, bottom))
        .line_to(Point::new(left, bottom))
        .close();
}

/// Three acts overlap by a tenth of the film and dissolve by opacity. They share
/// one set of post filters, so the dissolve never nests filtered subtrees.
fn film(context: &SceneContext<'_>, seed: u64) -> Node {
    let total = context.clip_duration().unwrap_or(8.seconds());
    let progress = context.progress();
    let frame = frame_seed(context);
    let shot = Time::from_ticks(total.ticks() * 2 / 5);
    let step = Time::from_ticks(total.ticks() * 3 / 10);
    let mut acts = Vec::new();
    for act in 0..3usize {
        let start = Time::from_ticks(step.ticks() * act as i64);
        let local = (progress - 0.3 * act as f64) / 0.4;
        let fade_in = if act == 0 {
            1.0
        } else {
            math::smoothstep(0.0, 0.25, local)
        };
        let fade_out = if act == 2 {
            1.0
        } else {
            1.0 - math::smoothstep(0.75, 1.0, local)
        };
        acts.push(
            context
                .during(start, shot, |inner| act_content(inner, act, seed))
                .opacity(fade_in * fade_out),
        );
    }
    let glitch = glitch_amount(context, frame, 0);
    acts.insert(0, context.bounds().fill(color(NIGHT)).into());
    let picture = Node::group(acts).clip_with(context.bounds(), ClipMode::Geometric);
    let picture = if glitch > 0.0 {
        glitch_slices(context, picture, frame, glitch)
    } else {
        picture
    };
    picture
        .filter(Filter::Glow {
            threshold: 0.42,
            sigma: 9.0,
            intensity: 0.85,
        })
        .filter(Filter::Optics(motion::Optics::Lens {
            distortion: 0.0,
            chromatic: (1.1 + glitch * 7.0) * 0.0025,
            auto_fit: false,
            center: None,
        }))
        .filter(Filter::Optics(motion::Optics::LensVignette {
            angle_of_view: 75.0,
            strength: 0.6,
            roundness: 0.0,
        }))
        .filter(Filter::grain(seed ^ frame, 0.012))
}

fn act_content(context: &SceneContext<'_>, act: usize, seed: u64) -> Node {
    let progress = context.progress();
    let mut nodes = vec![context.memo("neon-sky", || {
        context.bounds().fill(
            LinearGradient::new(Point::new(0.0, 0.0), Point::new(0.0, HORIZON))
                .stop(0.0, color(NIGHT))
                .stop(1.0, color(DUSK))
                .units(GradientUnits::UserSpace),
        )
    })];
    match act {
        0 => nodes.extend(horizon(context, progress)),
        1 => nodes.extend(interference(context, progress)),
        _ => nodes.extend(rain(context, seed, progress)),
    }
    nodes.push(skyline(context, seed));
    nodes.extend(titles(act, progress));
    nodes.push(scanlines(context));
    Node::group(nodes)
}

/// Short bursts of horizontal tearing; deterministic per frame so seeking replays.
fn glitch_amount(context: &SceneContext<'_>, frame: u64, act: usize) -> f64 {
    let burst = context.random(("glitch-burst", act, frame / 4));
    if burst > 0.94 {
        0.4 + 0.6 * context.random(("glitch-power", act, frame))
    } else {
        0.0
    }
}

fn glitch_slices(context: &SceneContext<'_>, picture: Node, frame: u64, amount: f64) -> Node {
    let mut layers = vec![picture.clone()];
    for slice in 0..3 {
        let top = 40.0 + context.random(("slice-top", frame, slice)) * 420.0;
        let height = 10.0 + context.random(("slice-height", frame, slice)) * 46.0;
        let shift = (context.random(("slice-shift", frame, slice)) - 0.5) * 90.0 * amount;
        layers.push(
            picture
                .clone()
                .clip_with(
                    Rect::new(0.0, top, 960.0, top + height),
                    ClipMode::Geometric,
                )
                .translate(shift, 0.0),
        );
    }
    Node::group(layers)
}

fn horizon(context: &SceneContext<'_>, progress: f64) -> Vec<Node> {
    let center = Point::new(480.0, 236.0);
    let drift = context.seconds() * 0.35;

    // The sun is cut by slits that thicken toward the horizon and creep upward.
    let sun = Circle::new(center, 112.0).fill(
        LinearGradient::new(Point::new(0.0, 124.0), Point::new(0.0, 348.0))
            .stop(0.0, color(SUN))
            .stop(0.55, color(MAGENTA))
            .stop(1.0, color(VIOLET))
            .units(GradientUnits::UserSpace),
    );
    let mut bands = Path2D::new();
    let mut top = 110.0;
    for slit in 0..9 {
        let thickness = 1.5 + slit as f64 * 1.9;
        let cut = 232.0 + slit as f64 * 17.0 - (drift * 14.0).rem_euclid(17.0);
        if cut > top {
            rect_path(&mut bands, 300.0, top, 660.0, cut.min(HORIZON));
        }
        top = cut + thickness;
    }
    rect_path(&mut bands, 300.0, top.min(HORIZON), 660.0, HORIZON);
    let sun = sun
        .mask(bands.fill(Color::WHITE), MaskMode::Alpha)
        .scale(0.9 + 0.1 * math::smoothstep(0.0, 0.5, progress));

    // Floor grid in perspective: horizontals bunch toward the vanishing line.
    let mut grid = Path2D::new();
    for line in 0..14 {
        let depth = ((line as f64 + drift * 2.0).rem_euclid(14.0)) / 14.0;
        let y = HORIZON + (540.0 - HORIZON) * math::pow(depth, 2.2);
        grid.move_to(Point::new(0.0, y))
            .line_to(Point::new(960.0, y));
    }
    for column in -13..=13 {
        let spread = column as f64;
        grid.move_to(Point::new(480.0 + spread * 9.0, HORIZON))
            .line_to(Point::new(480.0 + spread * 150.0, 560.0));
    }
    let floor = grid.stroke(
        1.3,
        LinearGradient::new(Point::new(0.0, HORIZON), Point::new(0.0, 540.0))
            .stop(0.0, color(MAGENTA).with_alpha(0.0))
            .stop(0.3, color(MAGENTA).with_alpha(0.7))
            .stop(1.0, color(CYAN).with_alpha(0.9))
            .units(GradientUnits::UserSpace),
    );
    let haze = Rect::new(0.0, HORIZON - 70.0, 960.0, HORIZON + 6.0).fill(
        LinearGradient::new(Point::new(0.0, HORIZON - 70.0), Point::new(0.0, HORIZON))
            .stop(0.0, color(MAGENTA).with_alpha(0.0))
            .stop(1.0, color(MAGENTA).with_alpha(0.45))
            .units(GradientUnits::UserSpace),
    );
    vec![
        sun,
        haze.blend_mode(BlendMode::Screen),
        Rect::new(0.0, HORIZON, 960.0, 540.0)
            .fill(color(NIGHT))
            .into(),
        floor.into(),
    ]
}

fn interference(context: &SceneContext<'_>, progress: f64) -> Vec<Node> {
    let phase = progress * TAU;
    let mut nodes = Vec::new();
    for index in 0..9 {
        let path = ribbon(index, phase);
        let hue = [CYAN, MAGENTA, VIOLET][index % 3];
        let paint = context.cache(("neon-ribbon", index), || {
            Paint::from(
                LinearGradient::new(Point::new(0.0, 0.0), Point::new(960.0, 0.0))
                    .stop(0.0, color(hue).with_alpha(0.0))
                    .stop(0.25, color(hue))
                    .stop(0.75, color([MAGENTA, CYAN, SUN][index % 3]))
                    .stop(1.0, color(hue).with_alpha(0.0))
                    .units(GradientUnits::UserSpace),
            )
        });
        nodes.push(
            path.fill(paint.as_ref().clone())
                .opacity(0.55)
                .blend_mode(BlendMode::Plus),
        );
    }
    let pulse = 0.5 + 0.5 * math::sin(phase * 2.0);
    let center = Point::new(480.0, 236.0);
    nodes.push(
        Ring::new(center, 70.0, 73.0 + pulse * 3.0)
            .fill(color(CYAN))
            .blend_mode(BlendMode::Plus),
    );
    nodes.push(
        Circle::new(center, 36.0 + pulse * 6.0)
            .fill(
                RadialGradient::new(center, 42.0 + pulse * 6.0)
                    .stop(0.0, Color::WHITE)
                    .stop(0.45, color(MAGENTA))
                    .stop(1.0, color(MAGENTA).with_alpha(0.0))
                    .units(GradientUnits::UserSpace),
            )
            .blend_mode(BlendMode::Plus),
    );
    nodes
}

fn rain(context: &SceneContext<'_>, seed: u64, progress: f64) -> Vec<Node> {
    let fall = context.seconds();
    let streaks = |range: std::ops::Range<u64>| {
        let mut drops = Path2D::new();
        for drop in range {
            let x = context.random(("rain-x", seed, drop)) * 1040.0 - 40.0;
            let speed = 380.0 + context.random(("rain-speed", seed, drop)) * 360.0;
            let length = 14.0 + context.random(("rain-length", seed, drop)) * 26.0;
            let offset = context.random(("rain-offset", seed, drop)) * 640.0;
            let y = (fall * speed + offset).rem_euclid(640.0) - 50.0;
            drops
                .move_to(Point::new(x, y))
                .line_to(Point::new(x - length * 0.18, y + length));
        }
        Node::from(drops.stroke(1.1, color(CYAN).with_alpha(0.55))).blend_mode(BlendMode::Plus)
    };
    let sign_flicker =
        0.75 + 0.25 * math::sin(context.seconds() * 31.0) * math::sin(context.seconds() * 7.3);
    let center = Point::new(724.0, 168.0);
    vec![
        Rect::new(0.0, 0.0, 960.0, HORIZON)
            .fill(
                RadialGradient::new(Point::new(724.0, 190.0), 380.0)
                    .stop(0.0, color(MAGENTA).with_alpha(0.38))
                    .stop(1.0, color(VIOLET).with_alpha(0.0))
                    .units(GradientUnits::UserSpace),
            )
            .into(),
        Ring::new(center, 56.0, 62.0)
            .fill(color(CYAN))
            .opacity(sign_flicker)
            .blend_mode(BlendMode::Plus),
        Ring::new(center, 36.0, 39.0)
            .sweep(-1.2, TAU * 0.78 * math::smoothstep(0.0, 0.5, progress))
            .fill(color(MAGENTA))
            .blend_mode(BlendMode::Plus),
        streaks(0..65),
        streaks(65..130),
        Rect::new(0.0, HORIZON, 960.0, 540.0)
            .fill(
                LinearGradient::new(Point::new(0.0, HORIZON), Point::new(0.0, 540.0))
                    .stop(0.0, color(DUSK))
                    .stop(1.0, color(NIGHT))
                    .units(GradientUnits::UserSpace),
            )
            .into(),
        // Wet street: the sign's reflection smeared vertically.
        Ring::new(Point::new(724.0, HORIZON + 96.0), 56.0, 62.0)
            .fill(color(CYAN).with_alpha(0.35))
            .scale_xy(1.0, 0.2)
            .filter(Filter::Blur { sigma: 3.0 })
            .opacity(sign_flicker)
            .blend_mode(BlendMode::Plus),
    ]
}

fn ribbon(index: usize, phase: f64) -> Path2D {
    let mut path = Path2D::new();
    for (reverse, side) in [(false, -1.0), (true, 1.0)] {
        for sample in 0..=32 {
            let fraction = if reverse { 32 - sample } else { sample } as f64 / 32.0;
            let x = -30.0 + fraction * 1020.0;
            let wave = math::sin(fraction * TAU + index as f64 * 0.31 + phase * 0.4);
            let envelope = math::sin(fraction * std::f64::consts::PI).max(0.0);
            let y = 236.0
                + (index as f64 - 4.0) * 20.0
                + wave * envelope * 96.0
                + side * (3.0 + index as f64 * 0.5);
            let point = Point::new(x, y);
            if !reverse && sample == 0 {
                path.move_to(point);
            } else {
                path.line_to(point);
            }
        }
    }
    path.close();
    path
}

/// A seeded skyline of silhouettes with lit windows, built once per scene seed.
fn skyline(context: &SceneContext<'_>, seed: u64) -> Node {
    context.memo(("skyline", seed), || {
        let mut towers = Path2D::new();
        let mut cyan = Path2D::new();
        let mut magenta = Path2D::new();
        let mut yellow = Path2D::new();
        let mut generator = context.random_generator(("skyline", seed));
        let mut x = -10.0;
        while x < 970.0 {
            let width = 18.0 + generator.next_float() * 34.0;
            let height = 24.0 + {
                let tall = generator.next_float();
                tall * tall
            } * 150.0;
            let top = HORIZON - height;
            rect_path(&mut towers, x, top, x + width, HORIZON + 2.0);
            let mut row = top + 6.0;
            while row < HORIZON - 4.0 {
                let mut column = x + 4.0;
                while column < x + width - 4.0 {
                    let lit = generator.next_float();
                    if lit > 0.72 {
                        let target = if lit > 0.95 {
                            &mut yellow
                        } else if lit > 0.84 {
                            &mut magenta
                        } else {
                            &mut cyan
                        };
                        rect_path(target, column, row, column + 2.5, row + 3.5);
                    }
                    column += 6.0;
                }
                row += 8.0;
            }
            x += width + 1.5 + generator.next_float() * 5.0;
        }
        Node::group([
            towers.fill(color(0x0a0518)),
            cyan.fill(color(CYAN)),
            magenta.fill(color(MAGENTA)),
            yellow.fill(color(SUN)),
        ])
    })
}

fn scanlines(context: &SceneContext<'_>) -> Node {
    // Stroke inputs are limited to 256 verbs, so the lines are split in chunks.
    context.memo("scanlines", || {
        let chunks = (0..180)
            .step_by(100)
            .map(|first| {
                let mut lines = Path2D::new();
                for row in first..(first + 100).min(180) {
                    let y = row as f64 * 3.0 + 1.5;
                    lines
                        .move_to(Point::new(0.0, y))
                        .line_to(Point::new(960.0, y));
                }
                lines.stroke(1.0, Color::BLACK.with_alpha(0.22))
            })
            .map(Node::from)
            .collect::<Vec<_>>();
        Node::group(chunks)
    })
}

fn titles(act: usize, progress: f64) -> Vec<Node> {
    let reveal = math::smoothstep(0.0, 0.18, progress);
    let neon = |text: Text, hue: u32| {
        Node::group([
            Node::from(text.clone().fill(color(MAGENTA)))
                .translate(-3.0, 0.0)
                .opacity(0.7)
                .blend_mode(BlendMode::Plus),
            Node::from(text.clone().fill(color(CYAN)))
                .translate(3.0, 0.0)
                .opacity(0.7)
                .blend_mode(BlendMode::Plus),
            Node::from(text.fill(color(hue))),
        ])
    };
    vec![
        label("CHROMATIC", MONO, 15.0, 7.0, FontWeight::BOLD, color(CYAN))
            .translate(44.0, 36.0)
            .opacity(reveal),
        label(
            &format!(
                "// 0{} {}",
                act + 1,
                ["NEON", "INTERFERENCE", "RELEASE"][act]
            ),
            MONO,
            13.0,
            3.0,
            FontWeight::BOLD,
            color(MAGENTA).interpolate(&Color::WHITE, 0.35),
        )
        .translate(716.0, 40.0)
        .opacity(reveal),
        neon(
            label(
                "TIDE",
                "Helvetica Neue",
                96.0,
                10.0,
                FontWeight::BLACK,
                Color::WHITE,
            ),
            0xf2f0ff,
        )
        .translate(40.0, 398.0 + (1.0 - reveal) * 30.0)
        .opacity(reveal),
        Rect::new(44.0, 508.0, 916.0, 509.0)
            .fill(color(CYAN).with_alpha(0.5))
            .scale_xy(reveal, 1.0)
            .pivot(Pivot::Point(Point::new(44.0, 508.0))),
    ]
}
