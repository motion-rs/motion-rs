//! Shot one of Quiet Hours: a sunlit ancient forest with misty trunk layers, light
//! shafts and swaying undergrowth.

use super::paint::{
    Grass, color, drift, foliage, grass, ground_below, point, ridge, rnd, vertical,
};
use super::{HEIGHT, Rig, WIDTH};
use motion::prelude::*;

// Shot one: sunlit ancient forest.

pub(super) fn forest(context: &SceneContext<'_>, rig: Rig, progress: f64, seconds: f64) -> Node {
    let backdrop = context.memo((rig.seed, "forest-backdrop"), || {
        let mut layers: Vec<Node> = vec![
            context
                .bounds()
                .fill(vertical(
                    0.0,
                    HEIGHT,
                    &[
                        (0.0, color(0xe9f0a8)),
                        (0.45, color(0xa5d49a)),
                        (1.0, color(0x4f9a74)),
                    ],
                ))
                .into(),
        ];
        // Pale trunks and canopy in two misty depths.
        for (depth, tone, alpha) in [(0u64, 0xb4dcae, 0.6), (1, 0x7dbd8f, 0.8)] {
            let mut trunks = Path2D::new();
            let mut crowns = Path2D::new();
            for index in 0..9u64 {
                let x = index as f64 * 124.0 - 20.0 + rnd(rig, "far", depth * 20 + index) * 70.0;
                let width = 10.0 + 12.0 * rnd(rig, "far-w", depth * 20 + index);
                trunks
                    .move_to(point(x - width, 440.0))
                    .quadratic_to(point(x - width * 0.4, 220.0), point(x - width * 0.5, 40.0))
                    .line_to(point(x + width * 0.5, 40.0))
                    .quadratic_to(point(x + width * 0.4, 220.0), point(x + width, 440.0))
                    .close();
                foliage(
                    &mut crowns,
                    rig,
                    "far-crown",
                    point(x, 70.0 + depth as f64 * 40.0),
                    Vector2::new(80.0, 36.0),
                    7,
                    42.0,
                );
            }
            layers.push(
                Node::group([
                    Node::from(trunks.fill(color(tone))),
                    crowns.fill(color(tone)).into(),
                ])
                .opacity(alpha),
            );
        }
        Node::group(layers)
    });

    let trunk_x = [118.0, 505.0, 846.0];
    let trunks = context.memo((rig.seed, "forest-trunks"), || {
        let mut bark = Path2D::new();
        let mut lit = Path2D::new();
        for (index, x) in trunk_x.iter().enumerate() {
            let sway = (rnd(rig, "trunk", index as u64) - 0.5) * 30.0;
            let top = 62.0 + sway * 0.4;
            bark.move_to(point(x - 74.0, 470.0))
                .quadratic_to(point(x - 22.0, 450.0), point(x - 30.0 + sway, 250.0))
                .quadratic_to(point(x - 24.0, 120.0), point(x - 26.0 + sway, -10.0))
                .line_to(point(x + 30.0 + sway, -10.0))
                .quadratic_to(point(x + 30.0, 130.0), point(x + 38.0 + sway, 250.0))
                .quadratic_to(point(x + 28.0, 450.0), point(x + 86.0, 470.0))
                .close();
            lit.move_to(point(x - 22.0, 440.0))
                .quadratic_to(point(x - 14.0, 250.0), point(x - 18.0 + sway, top))
                .line_to(point(x - 4.0 + sway, top))
                .quadratic_to(point(x - 2.0, 250.0), point(x - 2.0, 440.0))
                .close();
        }
        Node::group([
            Node::from(bark.fill(color(0x2c4a38))),
            lit.fill(color(0x6f8a4a).with_alpha(0.55)).into(),
        ])
    });

    let crowns = |tag: &'static str, lumps: u64, radius: f64, offset: f64, tone: u32| {
        context.memo((rig.seed, tag), || {
            let mut path = Path2D::new();
            for column in 0..7u64 {
                let x = column as f64 * 170.0 - 40.0;
                foliage(
                    &mut path,
                    rig,
                    tag,
                    point(x, 34.0 + offset + rnd(rig, tag, 900 + column) * 54.0),
                    Vector2::new(110.0, 54.0),
                    lumps,
                    radius,
                );
            }
            path.fill(color(tone))
        })
    };
    let shade = crowns("canopy-shade", 8, 78.0, 18.0, 0x1b5a38);
    let body = crowns("canopy-body", 7, 62.0, 8.0, 0x2d8446);
    let leaf = crowns("canopy-leaf", 6, 46.0, -6.0, 0x66bb58);
    let flare = crowns("canopy-flare", 5, 26.0, -14.0, 0xb8e36e);
    let canopy = Node::group([
        shade.translate(math::sin(seconds * 0.5) * 2.0, 0.0),
        body.translate(math::sin(seconds * 0.7 + 1.0) * 3.0, 0.0),
        leaf.translate(
            math::sin(seconds * 0.9 + 2.0) * 4.0,
            math::sin(seconds * 0.6) * 1.5,
        ),
        flare
            .translate(math::sin(seconds * 1.1 + 3.0) * 5.0, 0.0)
            .opacity(0.8),
    ]);

    let mut beams = Vec::new();
    for (index, x0) in [200.0, 330.0, 610.0, 760.0].into_iter().enumerate() {
        let sway = math::sin(seconds * 0.35 + index as f64 * 1.9) * 14.0;
        let path = Path2D::polyline([
            point(x0 - 24.0, -10.0),
            point(x0 + 40.0, -10.0),
            point(x0 + 210.0 + sway, 480.0),
            point(x0 + 70.0 + sway, 480.0),
        ])
        .closed();
        beams.push(
            Node::from(path.fill(vertical(
                0.0,
                480.0,
                &[
                    (0.0, color(0xfff3b0).with_alpha(0.75)),
                    (1.0, color(0xfff3b0).with_alpha(0.0)),
                ],
            )))
            .opacity(0.45 + 0.35 * math::sin(seconds * 0.6 + index as f64 * 1.7)),
        );
    }
    let beams = Node::group(beams)
        .filter(Filter::Blur { sigma: 7.0 })
        .blend_mode(BlendMode::Screen);

    let meadow = context.memo((rig.seed, "forest-meadow"), || {
        ridge(rig, "meadow", 392.0, 20.0, 0.007, HEIGHT + 30.0).fill(vertical(
            340.0,
            HEIGHT,
            &[(0.0, color(0x74c15a)), (1.0, color(0x2c7a3c))],
        ))
    });

    let mut motes = Vec::new();
    for index in 0..rig.count {
        let drift_x = 6.0 + 10.0 * rnd(rig, "mote-vx", index);
        let x = (rnd(rig, "mote-x", index) * (WIDTH + 40.0) + seconds * drift_x)
            .rem_euclid(WIDTH + 40.0)
            - 20.0;
        let y = (rnd(rig, "mote-y", index) * 420.0
            - seconds * (3.0 + 4.0 * rnd(rig, "mote-vy", index)))
        .rem_euclid(420.0)
            + 30.0
            + math::sin(seconds * 0.8 + index as f64) * 8.0;
        let size = 1.2 + 2.2 * rnd(rig, "mote-r", index);
        let twinkle =
            0.5 + 0.5 * math::sin(seconds * (0.9 + rnd(rig, "mote-f", index)) + index as f64);
        motes.push(Node::from(
            Circle::new(point(x, y), size).fill(color(0xffe08a).with_alpha(0.35 + 0.6 * twinkle)),
        ));
    }
    let motes = Node::group(motes).blend_mode(BlendMode::Plus);

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
    let back = wind(
        "grass-back",
        110,
        420.0,
        (36.0, 78.0),
        9.0,
        5.0,
        &[color(0x2f8a45), color(0x3f9d4c)],
    );
    let middle = wind(
        "grass-mid",
        90,
        468.0,
        (60.0, 118.0),
        12.0,
        6.5,
        &[color(0x4aa84c), color(0x6cc055), color(0x3d9a4a)],
    );
    let front = wind(
        "grass-front",
        64,
        528.0,
        (96.0, 190.0),
        16.0,
        8.0,
        &[color(0x7fcb58), color(0xa4dc66), color(0x58b64e)],
    );

    let haze = Rect::new(0.0, 250.0, WIDTH, 470.0)
        .fill(vertical(
            250.0,
            470.0,
            &[
                (0.0, color(0xfff6c8).with_alpha(0.0)),
                (1.0, color(0xfff6c8).with_alpha(0.35)),
            ],
        ))
        .blend_mode(BlendMode::Screen);

    let scene = Node::group([
        backdrop,
        trunks,
        canopy,
        beams,
        haze,
        meadow,
        ground_below(520.0, color(0x2c7a3c)),
        back,
        motes,
        middle,
        front,
    ]);
    drift(scene, progress, 1.06, 18.0).filter(Filter::Glow {
        threshold: 0.7,
        sigma: 12.0,
        intensity: 0.5,
    })
}
