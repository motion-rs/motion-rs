//! Shot three of Quiet Hours: a dusk village perched above a sea of clouds under a
//! warm evening sky.

use super::paint::{
    Grass, color, drift, ellipse, foliage, glow_disc, grass, point, ridge, rnd, vertical,
};
use super::{HEIGHT, Rig, WIDTH};
use motion::prelude::*;

// Shot three: dusk village above the clouds.

pub(super) fn dusk(context: &SceneContext<'_>, rig: Rig, progress: f64, seconds: f64) -> Node {
    let sun = point(650.0, 292.0 + 34.0 * progress);

    let sky = context.memo((rig.seed, "dusk-sky"), || {
        context.bounds().fill(vertical(
            0.0,
            352.0,
            &[
                (0.0, color(0x30306e)),
                (0.32, color(0x7a4f9c)),
                (0.58, color(0xe0708f)),
                (0.82, color(0xffa45e)),
                (1.0, color(0xffd98e)),
            ],
        ))
    });

    let sun_glow = Node::group([
        Node::from(Circle::new(sun, 300.0).fill(glow_disc(
            sun,
            300.0,
            &[
                (0.0, color(0xffe7a8).with_alpha(0.75)),
                (0.35, color(0xffa660).with_alpha(0.3)),
                (1.0, color(0xff8a60).with_alpha(0.0)),
            ],
        ))),
        Circle::new(sun, 42.0).fill(color(0xfff4cf)).into(),
    ])
    .blend_mode(BlendMode::Screen);

    let cloud_bank = |tag: &'static str,
                      count: u64,
                      y: f64,
                      band: f64,
                      light: u32,
                      shade: u32,
                      speed: f64,
                      alpha: f64| {
        let puffs = context.memo((rig.seed, tag), || {
            let mut lit = Path2D::new();
            let mut dark = Path2D::new();
            for index in 0..count {
                let x = rnd(rig, tag, index * 4) * (WIDTH + 400.0);
                let cy = y + (rnd(rig, tag, index * 4 + 1) - 0.5) * band;
                let rx = 50.0 + 90.0 * rnd(rig, tag, index * 4 + 2);
                let ry = rx * (0.16 + 0.12 * rnd(rig, tag, index * 4 + 3));
                ellipse(&mut dark, x, cy + ry * 0.55, rx, ry);
                ellipse(&mut lit, x, cy - ry * 0.1, rx * 0.88, ry * 0.8);
            }
            Node::group([
                Node::from(dark.fill(color(shade))),
                lit.fill(color(light)).into(),
            ])
        });
        let shift = (seconds * speed).rem_euclid(WIDTH + 400.0);
        Node::group([
            puffs.clone().translate(-shift - 200.0, 0.0),
            puffs.translate(WIDTH + 200.0 - shift, 0.0),
        ])
        .opacity(alpha)
    };
    let streaks = cloud_bank(
        "dusk-streaks",
        7,
        150.0,
        120.0,
        0xffc79a,
        0x9a5da0,
        6.0,
        0.9,
    );
    let sea_far = cloud_bank(
        "dusk-sea-far",
        14,
        362.0,
        24.0,
        0xffd6c2,
        0xc98aa8,
        3.5,
        0.9,
    );
    let sea_near = cloud_bank(
        "dusk-sea-near",
        16,
        408.0,
        30.0,
        0xffe0cc,
        0xd495a8,
        5.5,
        0.95,
    );

    let ranges = context.memo((rig.seed, "dusk-ranges"), || {
        let tones = [(0xb9709e, 338.0), (0x93589c, 360.0), (0x6a4b8c, 380.0)];
        Node::group(
            tones
                .iter()
                .enumerate()
                .map(|(index, (tone, base))| {
                    Node::from(
                        ridge(
                            rig,
                            ["range-a", "range-b", "range-c"][index],
                            *base,
                            34.0 - index as f64 * 6.0,
                            0.011 + index as f64 * 0.004,
                            430.0,
                        )
                        .fill(color(*tone)),
                    )
                })
                .collect::<Vec<_>>(),
        )
    });

    let hill = ridge(rig, "village-hill", 440.0, 26.0, 0.006, HEIGHT + 30.0);
    let village_hill = Node::from(hill.fill(vertical(
        400.0,
        HEIGHT,
        &[(0.0, color(0x5b4a78)), (1.0, color(0x2f3a55))],
    )));

    let houses = context.memo((rig.seed, "dusk-houses"), || {
        let mut walls = Path2D::new();
        let mut shade = Path2D::new();
        let mut roofs = Path2D::new();
        let mut windows = Path2D::new();
        for index in 0..8u64 {
            let x = 70.0 + index as f64 * 108.0 + (rnd(rig, "house-x", index) - 0.5) * 40.0;
            let scale = 0.8 + 0.5 * rnd(rig, "house-s", index);
            let y = 442.0 + 14.0 * math::sin(x * 0.006 + 1.0) + 6.0 * rnd(rig, "house-y", index);
            let (w, h) = (34.0 * scale, 24.0 * scale);
            walls
                .move_to(point(x - w, y))
                .line_to(point(x + w, y))
                .line_to(point(x + w, y - h))
                .line_to(point(x - w, y - h))
                .close();
            shade
                .move_to(point(x + w * 0.2, y))
                .line_to(point(x + w, y))
                .line_to(point(x + w, y - h))
                .line_to(point(x + w * 0.2, y - h))
                .close();
            roofs
                .move_to(point(x - w - 6.0, y - h))
                .line_to(point(x, y - h - 26.0 * scale))
                .line_to(point(x + w + 6.0, y - h))
                .close();
            roofs
                .move_to(point(x + w * 0.35, y - h - 8.0 * scale))
                .line_to(point(x + w * 0.35, y - h - 30.0 * scale))
                .line_to(point(x + w * 0.35 + 7.0, y - h - 30.0 * scale))
                .line_to(point(x + w * 0.35 + 7.0, y - h - 12.0 * scale))
                .close();
            windows
                .move_to(point(x - w * 0.55, y - h * 0.8))
                .line_to(point(x - w * 0.2, y - h * 0.8))
                .line_to(point(x - w * 0.2, y - h * 0.35))
                .line_to(point(x - w * 0.55, y - h * 0.35))
                .close();
        }
        Node::group([
            Node::from(walls.fill(color(0xf1d3ae))),
            shade.fill(color(0xc79a86)).into(),
            roofs.fill(color(0xa8473f)).into(),
            Node::from(windows.fill(color(0xffd46a))).blend_mode(BlendMode::Plus),
        ])
    });

    // Smoke rises from the first three chimneys and thins as it drifts.
    let mut smoke = Vec::new();
    for chimney in 0..3u64 {
        let scale = 0.8 + 0.5 * rnd(rig, "house-s", chimney * 3);
        let x =
            70.0 + (chimney * 3) as f64 * 108.0 + (rnd(rig, "house-x", chimney * 3) - 0.5) * 40.0;
        let y = 442.0 + 14.0 * math::sin(x * 0.006 + 1.0) + 6.0 * rnd(rig, "house-y", chimney * 3);
        let origin = point(
            x + 34.0 * scale * 0.35 + 3.5,
            y - 24.0 * scale - 30.0 * scale,
        );
        for puff in 0..6u64 {
            let age = (seconds * 0.16 + puff as f64 / 6.0 + chimney as f64 * 0.31).rem_euclid(1.0);
            let at = point(
                origin.x + age * 46.0 + math::sin(age * 9.0 + chimney as f64) * 5.0,
                origin.y - age * 90.0,
            );
            let alpha = 0.55 * (1.0 - age) * math::smoothstep(0.0, 0.1, age);
            smoke.push(Node::from(
                Circle::new(at, 3.0 + age * 15.0).fill(color(0xf0dde4).with_alpha(alpha)),
            ));
        }
    }

    let mut birds = Vec::new();
    for index in 0..5u64 {
        let phase = rnd(rig, "bird", index);
        let x =
            (phase * 600.0 + seconds * (22.0 + index as f64 * 3.0)).rem_euclid(WIDTH + 80.0) - 40.0;
        let y = 100.0 + index as f64 * 22.0 + math::sin(seconds * 0.6 + index as f64) * 8.0;
        let flap = math::sin(seconds * 6.0 + index as f64 * 1.7) * 5.0;
        let mut path = Path2D::new();
        path.move_to(point(x - 9.0, y - 3.0 + flap))
            .quadratic_to(point(x - 4.0, y - 5.0), point(x, y))
            .quadratic_to(point(x + 4.0, y - 5.0), point(x + 9.0, y - 3.0 + flap));
        birds.push(Node::from(
            path.stroke(1.6, color(0x3a2a56).with_alpha(0.8)),
        ));
    }

    let foreground = context.memo((rig.seed, "dusk-foreground"), || {
        let mut tree = Path2D::new();
        foliage(
            &mut tree,
            rig,
            "dusk-tree",
            point(140.0, 428.0),
            Vector2::new(56.0, 34.0),
            9,
            40.0,
        );
        Node::group([
            Node::from(
                ridge(rig, "dusk-front", 500.0, 22.0, 0.008, HEIGHT + 30.0).fill(color(0x23304a)),
            ),
            Node::from(
                Path2D::polyline([
                    point(136.0, 500.0),
                    point(140.0, 432.0),
                    point(146.0, 432.0),
                    point(152.0, 500.0),
                ])
                .closed()
                .fill(color(0x1c2740)),
            ),
            tree.fill(color(0x1c2740)).into(),
        ])
    });
    let tufts = grass(
        rig,
        seconds,
        &Grass {
            tag: "dusk-grass",
            blades: 80,
            base: 534.0,
            height: (26.0, 62.0),
            width: 10.0,
            sway: 4.5,
            tones: &[color(0x1a2640), color(0x26385a)],
        },
    );

    let scene = Node::group([
        sky,
        sun_glow,
        streaks,
        ranges,
        sea_far,
        sea_near,
        village_hill,
        houses,
        Node::group(smoke),
        Node::group(birds),
        foreground,
        tufts,
    ]);
    drift(scene, progress, 1.06, -22.0).filter(Filter::Glow {
        threshold: 0.6,
        sigma: 14.0,
        intensity: 0.5,
    })
}
