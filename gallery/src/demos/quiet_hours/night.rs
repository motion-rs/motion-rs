//! Shot four of Quiet Hours: a starry night meadow lit by drifting fireflies.

use super::paint::{Grass, color, drift, ellipse, glow_disc, grass, point, ridge, rnd, vertical};
use super::{HEIGHT, Rig, WIDTH};
use motion::prelude::*;
use std::f64::consts::TAU;

// Shot four: stars and fireflies.

pub(super) fn night(context: &SceneContext<'_>, rig: Rig, progress: f64, seconds: f64) -> Node {
    let sky = context.memo((rig.seed, "night-sky"), || {
        Node::group([
            Node::from(context.bounds().fill(vertical(
                0.0,
                380.0,
                &[
                    (0.0, color(0x040a22)),
                    (0.6, color(0x0e2150)),
                    (1.0, color(0x1f4670)),
                ],
            ))),
            Node::from(Circle::new(point(480.0, 150.0), 260.0).fill(glow_disc(
                point(480.0, 150.0),
                260.0,
                &[
                    (0.0, color(0x6b82d8).with_alpha(0.22)),
                    (1.0, color(0x6b82d8).with_alpha(0.0)),
                ],
            )))
            .scale_xy(2.4, 0.35)
            .rotate(-0.45),
        ])
    });

    let moon = point(770.0, 110.0);
    let moonlight = Node::group([
        Node::from(Circle::new(moon, 170.0).fill(glow_disc(
            moon,
            170.0,
            &[
                (0.0, color(0xdfe8ff).with_alpha(0.45)),
                (1.0, color(0xdfe8ff).with_alpha(0.0)),
            ],
        ))),
        Circle::new(moon, 32.0).fill(color(0xfff4d6)).into(),
        Circle::new(moon + Vector2::new(-9.0, 6.0), 7.0)
            .fill(color(0xe9dcb8).with_alpha(0.7))
            .into(),
        Circle::new(moon + Vector2::new(10.0, -8.0), 5.0)
            .fill(color(0xe9dcb8).with_alpha(0.6))
            .into(),
    ])
    .blend_mode(BlendMode::Screen);

    let mut stars = Vec::new();
    for index in 0..150u64 {
        let at = point(
            rnd(rig, "star-x", index) * WIDTH,
            rnd(rig, "star-y", index) * 320.0,
        );
        if at.distance(moon) < 46.0 {
            continue;
        }
        let blink = 0.55
            + 0.45
                * math::sin(seconds * (0.8 + 2.0 * rnd(rig, "star-f", index)) + index as f64 * 2.1);
        let size = 0.6 + 1.3 * rnd(rig, "star-r", index);
        stars.push(Node::from(
            Circle::new(at, size).fill(color(0xf4f6ff).with_alpha(blink)),
        ));
        if index % 17 == 0 {
            stars.push(Node::from(
                Star::new(at, 4, size * (2.0 + 4.0 * blink), size * 0.7)
                    .fill(color(0xffffff).with_alpha(0.7 * blink)),
            ));
        }
    }
    let stars = Node::group(stars).blend_mode(BlendMode::Plus);

    let hills = context.memo((rig.seed, "night-hills"), || {
        let layers = [
            ("night-far", 0x193a68, 352.0, 26.0, 0.009),
            ("night-mid", 0x10284d, 380.0, 22.0, 0.012),
        ];
        let mut nodes: Vec<Node> = layers
            .iter()
            .map(|(tag, tone, base, amplitude, scale)| {
                Node::from(ridge(rig, tag, *base, *amplitude, *scale, 460.0).fill(color(*tone)))
            })
            .collect();
        // A few distant windows still glow on the far hill.
        let mut lights = Path2D::new();
        for index in 0..7u64 {
            let x = 120.0 + index as f64 * 105.0 + rnd(rig, "night-light", index) * 40.0;
            ellipse(
                &mut lights,
                x,
                366.0 + rnd(rig, "night-light-y", index) * 10.0,
                1.6,
                1.2,
            );
        }
        nodes.push(Node::from(lights.fill(color(0xffc766))));
        Node::group(nodes)
    });
    let field = context.memo((rig.seed, "night-field"), || {
        ridge(rig, "night-field", 410.0, 14.0, 0.008, HEIGHT + 30.0).fill(vertical(
            392.0,
            HEIGHT,
            &[(0.0, color(0x0b2c3c)), (1.0, color(0x050f1a))],
        ))
    });

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
    let reeds = wind(
        "night-back",
        100,
        440.0,
        (30.0, 70.0),
        9.0,
        4.0,
        &[color(0x08202c), color(0x0b2a34)],
    );
    let tall = wind(
        "night-front",
        56,
        534.0,
        (110.0, 230.0),
        15.0,
        6.5,
        &[color(0x040c14), color(0x07161f), color(0x0a2229)],
    );

    let mut flies = Vec::new();
    for index in 0..rig.count * 2 {
        let depth = rnd(rig, "fly-depth", index);
        let center = point(rnd(rig, "fly-x", index) * WIDTH, 300.0 + depth * 200.0);
        let sway = 18.0 + 60.0 * rnd(rig, "fly-ax", index);
        let speed = 0.25 + 0.45 * rnd(rig, "fly-f", index);
        let at = point(
            center.x + sway * math::sin(seconds * speed + rnd(rig, "fly-p", index) * TAU),
            center.y
                + sway * 0.45 * math::sin(seconds * speed * 1.3 + rnd(rig, "fly-q", index) * TAU)
                + 6.0 * math::sin(seconds * 2.1 + index as f64),
        );
        let blink = math::pow(
            0.5 + 0.5
                * math::sin(seconds * (0.7 + 0.9 * rnd(rig, "fly-b", index)) + index as f64 * 1.9),
            3.0,
        );
        if blink < 0.03 {
            continue;
        }
        let size = 1.2 + 1.8 * depth;
        flies.push(Node::group([
            Node::from(Circle::new(at, size * 7.0).fill(color(0xb6ff4a).with_alpha(0.06 * blink))),
            Circle::new(at, size * 3.4)
                .fill(color(0xc8ff5a).with_alpha(0.16 * blink))
                .into(),
            Circle::new(at, size * 1.6)
                .fill(color(0xe4ff88).with_alpha(0.55 * blink))
                .into(),
            Circle::new(at, size * 0.8)
                .fill(color(0xfbffd0).with_alpha(blink))
                .into(),
        ]));
    }
    let flies = Node::group(flies).blend_mode(BlendMode::Plus);

    let scene = Node::group([sky, moonlight, stars, hills, field, reeds, flies, tall]);
    drift(scene, progress, 1.06, 18.0).filter(Filter::Glow {
        threshold: 0.35,
        sigma: 10.0,
        intensity: 1.0,
    })
}
