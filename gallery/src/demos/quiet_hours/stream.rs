//! Shot two of Quiet Hours: a clear stream winding between two Bezier banks, with
//! rippling water highlights and grassy shores.

use super::paint::{
    Grass, color, drift, ellipse, foliage, glow_disc, grass, ground_below, mix, point, rnd,
    vertical,
};
use super::{HEIGHT, Rig, WIDTH};
use motion::prelude::*;
use std::f64::consts::TAU;

// Shot two: clear stream.

const LEFT_BANK: [Point; 4] = [
    Point::new(442.0, 214.0),
    Point::new(415.0, 300.0),
    Point::new(250.0, 360.0),
    Point::new(70.0, 566.0),
];

const RIGHT_BANK: [Point; 4] = [
    Point::new(520.0, 214.0),
    Point::new(548.0, 290.0),
    Point::new(720.0, 360.0),
    Point::new(880.0, 566.0),
];

fn bezier(curve: &[Point; 4], t: f64) -> Point {
    let s = 1.0 - t;
    let weights = [s * s * s, 3.0 * s * s * t, 3.0 * s * t * t, t * t * t];
    point(
        curve.iter().zip(weights).map(|(p, w)| p.x * w).sum(),
        curve.iter().zip(weights).map(|(p, w)| p.y * w).sum(),
    )
}

/// A point on the stream: `t` runs from the far bend to the camera, `across` spans bank to bank.
fn water_point(t: f64, across: f64) -> Point {
    let left = bezier(&LEFT_BANK, t);
    let right = bezier(&RIGHT_BANK, t);
    point(mix(left.x, right.x, across), mix(left.y, right.y, across))
}

pub(super) fn stream(context: &SceneContext<'_>, rig: Rig, progress: f64, seconds: f64) -> Node {
    let stream_path = {
        let mut path = Path2D::new();
        path.move_to(LEFT_BANK[0])
            .cubic_to(LEFT_BANK[1], LEFT_BANK[2], LEFT_BANK[3])
            .line_to(RIGHT_BANK[3])
            .cubic_to(RIGHT_BANK[2], RIGHT_BANK[1], RIGHT_BANK[0])
            .close();
        path
    };

    let backdrop = context.memo((rig.seed, "stream-backdrop"), || {
        let mut trees = Vec::new();
        for (index, (tone, base, spread)) in [
            (0x9ccfa4, 150.0, 20.0),
            (0x62ab7e, 176.0, 26.0),
            (0x3d8c5c, 204.0, 20.0),
        ]
        .into_iter()
        .enumerate()
        {
            let mut path = Path2D::new();
            for column in 0..10u64 {
                foliage(
                    &mut path,
                    rig,
                    "stream-trees",
                    point(
                        column as f64 * 110.0 - 10.0
                            + rnd(rig, "stream-x", column + 40 * index as u64) * 50.0,
                        base - rnd(rig, "stream-y", column + 40 * index as u64) * spread,
                    ),
                    Vector2::new(60.0, 22.0),
                    5,
                    50.0 - index as f64 * 6.0,
                );
            }
            trees.push(Node::from(path.fill(color(tone))));
        }
        Node::group([
            Node::from(context.bounds().fill(vertical(
                0.0,
                230.0,
                &[
                    (0.0, color(0x9fd8ee)),
                    (0.7, color(0xdcf1e0)),
                    (1.0, color(0xf6f8d8)),
                ],
            ))),
            Node::group(trees),
            ground_below(205.0, color(0x6fb85a)),
            Rect::new(0.0, 205.0, WIDTH, HEIGHT + 30.0)
                .fill(vertical(
                    205.0,
                    HEIGHT,
                    &[
                        (0.0, color(0xb5e07a)),
                        (0.4, color(0x6cb552)),
                        (1.0, color(0x3b8a41)),
                    ],
                ))
                .into(),
        ])
    });

    let water = Node::from(stream_path.clone().fill(vertical(
        214.0,
        HEIGHT,
        &[
            (0.0, color(0xcdf2ea)),
            (0.35, color(0x74d0cc)),
            (1.0, color(0x2d94b4)),
        ],
    )));
    let edge = Node::from(stream_path.stroke(2.5, color(0xf5ffdf).with_alpha(0.45)));

    // Ripple lines ride the current toward the camera.
    let mut ripples = Vec::new();
    for index in 0..22u64 {
        let t = (index as f64 / 22.0 + seconds * 0.025 + rnd(rig, "ripple", index) * 0.03)
            .rem_euclid(1.0);
        let depth = 0.05 + 0.95 * t;
        let mut path = Path2D::new();
        for step in 0..=10 {
            let across = 0.04 + 0.92 * step as f64 / 10.0;
            let mut at = water_point(depth, across);
            at.y += math::sin(across * 14.0 + seconds * 2.0 + index as f64) * 2.5 * depth;
            if step == 0 {
                path.move_to(at);
            } else {
                path.line_to(at);
            }
        }
        let fade = math::smoothstep(0.0, 0.12, t) * (1.0 - math::smoothstep(0.88, 1.0, t));
        ripples.push(Node::from(
            path.stroke(0.8 + 2.4 * depth, Color::WHITE.with_alpha(0.4 * fade)),
        ));
    }

    // Smooth stones with a foam ring that breathes with the current.
    let mut stones = Vec::new();
    for index in 0..9u64 {
        let t = 0.18 + 0.78 * rnd(rig, "stone-t", index);
        let across = 0.12 + 0.76 * rnd(rig, "stone-a", index);
        let at = water_point(t, across);
        let scale = 0.35 + 1.1 * t;
        let (rx, ry) = (26.0 * scale, 10.0 * scale);
        let foam = 0.5 + 0.5 * math::sin(seconds * 1.6 + index as f64 * 2.0);
        let mut shape = Path2D::new();
        ellipse(&mut shape, at.x, at.y, rx, ry);
        let mut top = Path2D::new();
        ellipse(
            &mut top,
            at.x - rx * 0.15,
            at.y - ry * 0.25,
            rx * 0.7,
            ry * 0.55,
        );
        let mut ring = Path2D::new();
        ellipse(
            &mut ring,
            at.x,
            at.y + ry * 0.35,
            rx * (1.25 + 0.1 * foam),
            ry * 0.95,
        );
        stones.push(Node::group([
            Node::from(ring.stroke(1.2 + scale, Color::WHITE.with_alpha(0.25 + 0.3 * foam))),
            shape.fill(color(0x7b8084)).into(),
            top.fill(color(0xb8bcb4)).into(),
        ]));
    }

    let mut sparkles = Vec::new();
    for index in 0..rig.count {
        let t = (rnd(rig, "spark-t", index) + seconds * 0.02).rem_euclid(1.0);
        let at = water_point(0.06 + 0.94 * t, 0.04 + 0.92 * rnd(rig, "spark-a", index));
        let twinkle = math::pow(
            0.5 + 0.5
                * math::sin(
                    seconds * (1.5 + 2.0 * rnd(rig, "spark-f", index)) + index as f64 * 1.3,
                ),
            4.0,
        );
        let size = (2.0 + 6.0 * t) * twinkle;
        if size > 0.4 {
            sparkles.push(Node::from(
                Star::new(at, 4, size * 2.0, size * 0.35).fill(Color::WHITE.with_alpha(0.9)),
            ));
        }
    }
    let sparkles = Node::group(sparkles).blend_mode(BlendMode::Plus);

    let mut blossoms = Vec::new();
    for index in 0..(rig.count / 2).max(6) {
        let bank_left = rnd(rig, "flower-side", index) < 0.5;
        let t = 0.22 + 0.78 * rnd(rig, "flower-t", index);
        let edge = if bank_left {
            bezier(&LEFT_BANK, t)
        } else {
            bezier(&RIGHT_BANK, t)
        };
        let scale = 0.3 + 0.9 * t;
        let outward = (14.0 + 190.0 * rnd(rig, "flower-d", index)) * scale;
        let base = point(
            edge.x + if bank_left { -outward } else { outward },
            edge.y + 14.0 * scale,
        );
        if base.x < -20.0 || base.x > WIDTH + 20.0 || base.y > HEIGHT + 20.0 {
            continue;
        }
        let height = (38.0 + 40.0 * rnd(rig, "flower-h", index)) * scale;
        let sway = math::sin(seconds * 1.4 + index as f64 * 0.9) * 7.0 * scale;
        let head = point(base.x + sway, base.y - height);
        let mut stem = Path2D::new();
        stem.move_to(base)
            .quadratic_to(point(base.x + sway * 0.2, base.y - height * 0.55), head);
        let tone = [0xff9fb8, 0xfffaf0, 0xffd95a, 0xc9a2ff][(index % 4) as usize];
        let mut petals = Path2D::new();
        for petal in 0..5 {
            let angle = petal as f64 / 5.0 * TAU + index as f64;
            ellipse(
                &mut petals,
                head.x + math::cos(angle) * 5.5 * scale,
                head.y + math::sin(angle) * 5.5 * scale,
                4.6 * scale,
                4.6 * scale,
            );
        }
        blossoms.push((
            base.y,
            Node::group([
                Node::from(stem.stroke(1.8 * scale, color(0x2f8a40))),
                petals.fill(color(tone)).into(),
                Circle::new(head, 2.8 * scale).fill(color(0xf2a62a)).into(),
            ]),
        ));
    }
    blossoms.sort_by(|a, b| a.0.total_cmp(&b.0));

    let wind = |tag: &'static str, blades, base, height, width, sway, tones: &[Color]| {
        grass(
            rig,
            seconds,
            &Grass {
                tag,
                blades,
                base,
                height,
                width,
                sway,
                tones,
            },
        )
    };
    let corner_left = wind(
        "bank-left",
        26,
        548.0,
        (50.0, 120.0),
        14.0,
        6.0,
        &[color(0x57b44d), color(0x7fcb58)],
    )
    .clip_with(Rect::new(0.0, 380.0, 250.0, HEIGHT), ClipMode::Geometric);
    let corner_right = wind(
        "bank-right",
        26,
        548.0,
        (50.0, 120.0),
        14.0,
        6.0,
        &[color(0x57b44d), color(0x86d05a)],
    )
    .clip_with(Rect::new(700.0, 400.0, WIDTH, HEIGHT), ClipMode::Geometric);

    let dapple = Node::from(Circle::new(point(500.0, 300.0), 220.0).fill(glow_disc(
        point(500.0, 300.0),
        220.0,
        &[
            (0.0, color(0xfffbd0).with_alpha(0.28)),
            (1.0, color(0xfffbd0).with_alpha(0.0)),
        ],
    )))
    .blend_mode(BlendMode::Screen);

    let scene = Node::group([
        backdrop,
        water,
        Node::group(ripples),
        Node::group(stones),
        edge,
        dapple,
        sparkles,
        Node::group(
            blossoms
                .into_iter()
                .map(|(_, node)| node)
                .collect::<Vec<_>>(),
        ),
        corner_left,
        corner_right,
    ]);
    drift(scene, progress, 1.05, -16.0).filter(Filter::Glow {
        threshold: 0.75,
        sigma: 9.0,
        intensity: 0.55,
    })
}
