//! A lightning orb gathers in a moonlit valley: sparks and thin arcs
//! swell into a blinding burst, then a full-power orb throws branching bolts,
//! cracks the ground and flings debris before the charge dies away. Bolts are
//! procedural midpoint-displacement polylines that re-roll at 15 Hz, lit with plus
//! blending, bloom and chromatic fringing. The story is paced by progress, so any
//! duration plays the same beats.

use crate::{DemoMetadata, DemoParameters};
use motion::prelude::*;
use std::f64::consts::TAU;

const WIDTH: f64 = 960.0;

const HEIGHT: f64 = 540.0;

const CORE: Point = Point::new(480.0, 290.0);

const GROUND: f64 = 478.0;

const BLUE: u32 = 0x2f7bff;

const SKY: u32 = 0x7cc8ff;

const WHITE: u32 = 0xf2fbff;

/// The beat at which the orb detonates, as a fraction of the film.
const BURST: f64 = 0.5;

/// Virtual seconds of the default eight second cut; burst physics run on this clock.
const DEFAULT_SECONDS: f64 = 8.0;

pub(super) const METADATA: DemoMetadata = DemoMetadata::new(
    "chidori",
    "Chidori",
    "Lightning",
    &[
        "procedural-lightning",
        "blend-modes",
        "filters",
        "gradients",
        "camera-shake",
    ],
);

#[derive(Clone, Copy)]
struct Rig {
    seed: u64,
    count: u64,
}

pub(super) fn timeline(parameters: DemoParameters) -> motion::Result<Timeline> {
    let duration = i64::from(parameters.seconds).seconds();
    let rig = Rig {
        seed: parameters.seed,
        count: u64::from(parameters.count),
    };
    Ok(Timeline::new(Size::new(WIDTH, HEIGHT), 30)
        .track(|track| track.scene(duration, move |context| frame(context, rig))))
}

fn color(hex: u32) -> Color {
    Color::from_hex(hex)
}

fn point(x: f64, y: f64) -> Point {
    Point::new(x, y)
}

fn rnd(rig: Rig, tag: &str, index: u64) -> f64 {
    Random::from_key((rig.seed, tag, index)).next_float()
}

fn mix(from: f64, to: f64, amount: f64) -> f64 {
    from + (to - from) * amount
}

fn vertical(top: f64, bottom: f64, stops: &[(f64, Color)]) -> LinearGradient {
    stops
        .iter()
        .fold(
            LinearGradient::new(point(0.0, top), point(0.0, bottom)),
            |gradient, (position, color)| gradient.stop(*position, *color),
        )
        .units(GradientUnits::UserSpace)
}

fn glow(center: Point, radius: f64, stops: &[(f64, Color)]) -> RadialGradient {
    stops
        .iter()
        .fold(
            RadialGradient::new(center, radius),
            |gradient, (position, color)| gradient.stop(*position, *color),
        )
        .units(GradientUnits::UserSpace)
}

fn ridge(rig: Rig, tag: &str, base: f64, amplitude: f64, scale: f64, bottom: f64) -> Path2D {
    let phases = [
        rnd(rig, tag, 0) * TAU,
        rnd(rig, tag, 1) * TAU,
        rnd(rig, tag, 2) * TAU,
    ];
    let mut path = Path2D::new();
    path.move_to(point(-24.0, bottom));
    let mut x = -24.0;
    while x <= WIDTH + 24.0 {
        let wave = 0.55 * math::sin(x * scale + phases[0])
            + 0.3 * math::sin(x * scale * 2.3 + phases[1])
            + 0.15 * math::sin(x * scale * 5.1 + phases[2]);
        path.line_to(point(x, base + amplitude * wave));
        x += 8.0;
    }
    path.line_to(point(WIDTH + 24.0, bottom)).close();
    path
}

fn round_line(path: Path2D, width: f64, paint: impl Into<Paint>) -> Node {
    path.stroke(width, paint)
        .stroke_style(StrokeStyle {
            width,
            cap: LineCap::Round,
            join: LineJoin::Round,
        })
        .into()
}

/// Midpoint displacement: every level halves the segments and nudges the new
/// vertices sideways, so a straight chord becomes a crackling arc.
fn jagged(rig: Rig, key: (u64, u64, u64), from: Point, to: Point, levels: u32) -> Vec<Point> {
    let mut points = vec![from, to];
    let mut scale = 0.34;
    for level in 0..levels {
        let mut next = Vec::with_capacity(points.len() * 2);
        for (index, pair) in points.windows(2).enumerate() {
            let (start, end) = (pair[0], pair[1]);
            let chord = end - start;
            let length = chord.length();
            let normal = if length > 1e-6 {
                Vector2::new(-chord.y, chord.x) * (1.0 / length)
            } else {
                Vector2::ZERO
            };
            let roll = Random::from_key((rig.seed, "jag", key, level, index as u64)).next_float();
            let middle = point((start.x + end.x) * 0.5, (start.y + end.y) * 0.5);
            next.push(start);
            next.push(middle + normal * ((roll - 0.5) * 2.0 * length * scale));
        }
        next.push(*points.last().unwrap_or(&to));
        points = next;
        scale *= 0.8;
    }
    points
}

/// Three stacked strokes: wide blue bloom, bright ice body, white-hot core.
fn strike(points: &[Point], width: f64, energy: f64) -> Node {
    let path = Path2D::polyline(points.iter().copied());
    Node::group([
        round_line(
            path.clone(),
            width * 5.5,
            color(BLUE).with_alpha(0.14 * energy),
        ),
        round_line(
            path.clone(),
            width * 2.2,
            color(SKY).with_alpha(0.5 * energy),
        ),
        round_line(path, width * 0.9, color(WHITE).with_alpha(0.95 * energy)),
    ])
}

fn frame(context: &SceneContext<'_>, rig: Rig) -> Node {
    let progress = context.progress();
    let frame_index = context.global_time().to_frame(context.frame_rate()).max(0) as u64;
    // Bolts re-roll at 15 Hz so they flicker rather than strobe.
    let step = frame_index / 2;
    let clock = progress * DEFAULT_SECONDS;
    let beat = (progress - BURST).max(0.0) * DEFAULT_SECONDS;

    let charge = math::smoothstep(0.08, BURST, progress);
    let ramp = math::smoothstep(BURST - 0.005, BURST + 0.02, progress);
    let fade = math::smoothstep(0.86, 1.0, progress);
    let flicker = 0.82 + 0.18 * rnd(rig, "flicker", frame_index);
    let base_energy = mix(0.12 + 0.5 * charge, 1.0, ramp) * (1.0 - 0.95 * fade);
    let energy = (base_energy * flicker).clamp(0.0, 1.0);
    let flash = math::smoothstep(BURST - 0.005, BURST + 0.005, progress)
        * (1.0 - math::smoothstep(BURST + 0.005, BURST + 0.06, progress));
    let radius = mix(7.0 + 12.0 * charge, 46.0, ramp) * (1.0 - 0.9 * fade);

    let mut layers = vec![
        environment(context, rig, energy),
        ground_light(energy),
        cracks(rig, beat, energy),
        shockwave(beat, energy),
        debris(rig, beat, energy),
        core(radius, energy),
        arcs(rig, step, energy, radius),
        sparks(rig, clock, energy),
    ];
    layers.push(
        context
            .bounds()
            .fill(color(WHITE).with_alpha(0.9 * flash))
            .into(),
    );

    let shake = 1.0 + 9.0 * energy * energy + 14.0 * flash;
    let jitter = point(
        (rnd(rig, "shake-x", frame_index) - 0.5) * shake,
        (rnd(rig, "shake-y", frame_index) - 0.5) * shake,
    );
    Node::group(layers)
        .pivot(Pivot::Point(CORE))
        .scale(1.04 + 0.08 * progress)
        .translate(jitter.x, jitter.y)
        .clip_with(context.bounds(), ClipMode::Geometric)
        .filter(Filter::Glow {
            threshold: 0.5,
            sigma: 16.0,
            intensity: 0.7 + 0.5 * energy,
        })
        .filter(Filter::Optics(motion::Optics::Lens {
            distortion: 0.0,
            chromatic: (0.3 + 1.0 * energy + 2.5 * flash) * 0.0025,
            auto_fit: false,
            center: None,
        }))
        .filter(Filter::Optics(motion::Optics::LensVignette {
            angle_of_view: 75.0,
            strength: 0.5,
            roundness: 0.0,
        }))
}

fn environment(context: &SceneContext<'_>, rig: Rig, energy: f64) -> Node {
    let backdrop = context.memo((rig.seed, "chidori-backdrop"), || {
        let mut stars = Path2D::new();
        for index in 0..60u64 {
            let at = point(
                rnd(rig, "star-x", index) * WIDTH,
                rnd(rig, "star-y", index) * 280.0,
            );
            let size = 0.9 + 1.1 * rnd(rig, "star-r", index);
            stars
                .move_to(point(at.x + size, at.y))
                .line_to(point(at.x, at.y + size))
                .line_to(point(at.x - size, at.y))
                .line_to(point(at.x, at.y - size))
                .close();
        }
        let moon = point(170.0, 110.0);
        let mut pines = Path2D::new();
        for index in 0..26u64 {
            let x = index as f64 * 38.0 + rnd(rig, "pine-x", index) * 24.0 - 10.0;
            let base = 404.0 + 14.0 * math::sin(x * 0.011) + rnd(rig, "pine-y", index) * 8.0;
            let height = 44.0 + 40.0 * rnd(rig, "pine-h", index);
            for tier in 0..3 {
                let top = base - height + tier as f64 * height * 0.27;
                let half = height * (0.22 + 0.1 * tier as f64);
                pines
                    .move_to(point(x, top))
                    .line_to(point(x + half, top + height * 0.4))
                    .line_to(point(x - half, top + height * 0.4))
                    .close();
            }
            pines
                .move_to(point(x - 2.0, base))
                .line_to(point(x - 2.0, base + 10.0))
                .line_to(point(x + 2.0, base + 10.0))
                .line_to(point(x + 2.0, base))
                .close();
        }
        Node::group([
            Node::from(context.bounds().fill(vertical(
                0.0,
                400.0,
                &[
                    (0.0, color(0x02050f)),
                    (0.6, color(0x0a1530)),
                    (1.0, color(0x182a4c)),
                ],
            ))),
            stars.fill(color(0xcfe0ff).with_alpha(0.8)).into(),
            Circle::new(moon, 150.0)
                .fill(glow(
                    moon,
                    150.0,
                    &[
                        (0.0, color(0xbcd0ff).with_alpha(0.3)),
                        (1.0, color(0xbcd0ff).with_alpha(0.0)),
                    ],
                ))
                .into(),
            Circle::new(moon, 26.0).fill(color(0xe8efff)).into(),
            ridge(rig, "far-cliff", 350.0, 46.0, 0.006, 470.0)
                .fill(color(0x101c38))
                .into(),
            ridge(rig, "mid-cliff", 392.0, 28.0, 0.009, 470.0)
                .fill(color(0x0a1226))
                .into(),
            pines.fill(color(0x060b17)).into(),
            Rect::new(-30.0, 400.0, WIDTH + 30.0, HEIGHT + 30.0)
                .fill(vertical(
                    400.0,
                    HEIGHT,
                    &[(0.0, color(0x0b1424)), (1.0, color(0x03060d))],
                ))
                .into(),
        ])
    });
    // The technique lights the whole valley, and it flickers with the charge.
    let wash = Circle::new(CORE, 620.0)
        .fill(glow(
            CORE,
            620.0,
            &[
                (0.0, color(0x4a98ff).with_alpha(0.6 * energy)),
                (0.45, color(BLUE).with_alpha(0.2 * energy)),
                (1.0, color(BLUE).with_alpha(0.0)),
            ],
        ))
        .blend_mode(BlendMode::Screen);
    Node::group([backdrop, wash])
}

fn ground_light(energy: f64) -> Node {
    let center = point(CORE.x, GROUND);
    Node::from(Circle::new(center, 300.0).fill(glow(
        center,
        300.0,
        &[
            (0.0, color(SKY).with_alpha(0.55 * energy)),
            (1.0, color(SKY).with_alpha(0.0)),
        ],
    )))
    .scale_xy(1.0, 0.22)
    .blend_mode(BlendMode::Plus)
}

/// Fractures spread from the point under the orb once it detonates.
fn cracks(rig: Rig, beat: f64, energy: f64) -> Node {
    let grow = math::smoothstep(0.0, 0.7, beat);
    if grow <= 0.01 {
        return Node::empty();
    }
    let origin = point(CORE.x, GROUND + 4.0);
    let mut nodes = Vec::new();
    for index in 0..10u64 {
        let direction = if index % 2 == 0 { -1.0 } else { 1.0 };
        let reach = (120.0 + 340.0 * rnd(rig, "crack-len", index)) * direction;
        let drop = 10.0 + 56.0 * rnd(rig, "crack-dy", index);
        let mut path = Path2D::new();
        path.move_to(origin);
        for step in 1..=14u64 {
            let along = step as f64 / 14.0;
            path.line_to(point(
                origin.x + reach * along,
                origin.y
                    + drop * math::pow(along, 0.8)
                    + (rnd(rig, "crack-j", index * 20 + step) - 0.5) * 14.0 * along,
            ));
        }
        let width = 1.0 + 2.2 * (1.0 - index as f64 / 12.0);
        nodes.push(Node::group([
            round_line(
                path.trim(0.0, grow),
                width * 4.0,
                color(BLUE).with_alpha(0.18 * energy),
            ),
            round_line(
                path.trim(0.0, grow),
                width,
                color(WHITE).with_alpha(0.85 * energy),
            ),
        ]));
    }
    Node::group(nodes).blend_mode(BlendMode::Plus)
}

fn shockwave(beat: f64, energy: f64) -> Node {
    let mut rings = Vec::new();
    for (delay, strength) in [(0.0, 1.0), (0.25, 0.6)] {
        let age = beat - delay;
        if age <= 0.0 || age > 1.8 {
            continue;
        }
        let radius = age * 420.0;
        let alpha = (1.0 - age / 1.8) * strength * energy.max(0.3);
        let mut ring = Path2D::new();
        let center = point(CORE.x, GROUND + 6.0);
        let (kx, ky) = (radius * 0.5523, radius * 0.18 * 0.5523);
        let ry = radius * 0.18;
        ring.move_to(point(center.x + radius, center.y))
            .cubic_to(
                point(center.x + radius, center.y + ky),
                point(center.x + kx, center.y + ry),
                point(center.x, center.y + ry),
            )
            .cubic_to(
                point(center.x - kx, center.y + ry),
                point(center.x - radius, center.y + ky),
                point(center.x - radius, center.y),
            )
            .cubic_to(
                point(center.x - radius, center.y - ky),
                point(center.x - kx, center.y - ry),
                point(center.x, center.y - ry),
            )
            .cubic_to(
                point(center.x + kx, center.y - ry),
                point(center.x + radius, center.y - ky),
                point(center.x + radius, center.y),
            )
            .close();
        rings.push(Node::from(
            ring.stroke(3.0, color(SKY).with_alpha(0.7 * alpha)),
        ));
    }
    Node::group(rings).blend_mode(BlendMode::Plus)
}

/// Rock shards thrown up in repeating waves while the orb burns.
fn debris(rig: Rig, beat: f64, energy: f64) -> Node {
    if beat <= 0.0 {
        return Node::empty();
    }
    let mut shards = Vec::new();
    for index in 0..30u64 {
        let wave = 2.2;
        let age = (beat + rnd(rig, "rock-phase", index) * wave).rem_euclid(wave);
        let velocity = Vector2::new(
            (rnd(rig, "rock-vx", index) - 0.5) * 520.0,
            -(160.0 + 300.0 * rnd(rig, "rock-vy", index)),
        );
        let start = point(CORE.x + (rnd(rig, "rock-x", index) - 0.5) * 90.0, GROUND);
        let at = point(
            start.x + velocity.x * age,
            start.y + velocity.y * age + 0.5 * 620.0 * age * age,
        );
        if at.y > GROUND + 8.0 {
            continue;
        }
        let size = 3.0 + 6.0 * rnd(rig, "rock-s", index);
        let spin = age * (2.0 + 6.0 * rnd(rig, "rock-w", index));
        let mut shard = Path2D::new();
        for corner in 0..4u64 {
            let angle = spin + corner as f64 * TAU / 4.0 + rnd(rig, "rock-c", index * 4 + corner);
            let reach = size * (0.6 + 0.6 * rnd(rig, "rock-r", index * 4 + corner));
            let vertex = point(
                at.x + math::cos(angle) * reach,
                at.y + math::sin(angle) * reach,
            );
            if corner == 0 {
                shard.move_to(vertex);
            } else {
                shard.line_to(vertex);
            }
        }
        shard.close();
        shards.push(Node::group([
            Node::from(shard.clone().fill(color(0x141c30))),
            Node::from(shard.stroke(1.0, color(SKY).with_alpha(0.7 * energy))),
        ]));
    }
    Node::group(shards)
}

fn core(radius: f64, energy: f64) -> Node {
    Node::group([
        Node::from(Circle::new(CORE, radius * 5.0).fill(glow(
            CORE,
            radius * 5.0,
            &[
                (0.0, color(0x8fd0ff).with_alpha(0.7 * energy)),
                (0.4, color(BLUE).with_alpha(0.25 * energy)),
                (1.0, color(BLUE).with_alpha(0.0)),
            ],
        ))),
        Circle::new(CORE, radius * 1.5)
            .fill(glow(
                CORE,
                radius * 1.5,
                &[
                    (0.0, color(WHITE)),
                    (0.55, color(SKY).with_alpha(0.9 * energy)),
                    (1.0, color(BLUE).with_alpha(0.0)),
                ],
            ))
            .into(),
        Circle::new(CORE, radius * 0.7)
            .fill(color(WHITE).with_alpha(0.6 + 0.4 * energy))
            .into(),
    ])
    .blend_mode(BlendMode::Plus)
}

/// Bolts radiate from the orb and a few of them strike the ground.
fn arcs(rig: Rig, step: u64, energy: f64, radius: f64) -> Node {
    let mut nodes = Vec::new();
    let mains = 3 + (energy * rig.count as f64 * 0.12) as u64;
    for index in 0..mains {
        let roll = |tag: &str| Random::from_key((rig.seed, tag, step, index)).next_float();
        let angle = roll("angle") * TAU;
        let length = radius * 1.4 + (30.0 + 250.0 * energy) * (0.35 + 0.65 * roll("length"));
        let mut end = point(
            CORE.x + math::cos(angle) * length,
            CORE.y + math::sin(angle) * length * 0.8,
        );
        end.y = end.y.min(GROUND + 20.0);
        let start = point(
            CORE.x + math::cos(angle) * radius * 0.5,
            CORE.y + math::sin(angle) * radius * 0.5,
        );
        let points = jagged(rig, (step, index, 0), start, end, 4);
        let width = 0.9 + 1.5 * energy * roll("width");
        nodes.push(strike(&points, width, energy));
        // Forks leave the main bolt at a shallow angle.
        for fork in 0..2u64 {
            if roll(["fork-a", "fork-b"][fork as usize]) > 0.55 + 0.3 * energy {
                continue;
            }
            let pick = 4 + (roll("pick") * 8.0) as usize + fork as usize * 2;
            let from = points[pick.min(points.len() - 2)];
            let side = if fork == 0 { 1.0 } else { -1.0 };
            let turn = angle + side * (0.4 + 0.7 * roll("turn"));
            let reach = length * (0.25 + 0.25 * roll("reach"));
            let to = point(
                from.x + math::cos(turn) * reach,
                from.y + math::sin(turn) * reach,
            );
            let branch = jagged(rig, (step, index, 1 + fork), from, to, 3);
            nodes.push(strike(&branch, width * 0.6, energy));
        }
    }
    // Grounding strikes drop straight to the floor under the orb.
    for index in 0..(energy * 4.0) as u64 {
        let roll = |tag: &str| Random::from_key((rig.seed, tag, step, index)).next_float();
        let target = point(
            CORE.x + (roll("ground-x") - 0.5) * 220.0,
            GROUND + 6.0 * roll("ground-y"),
        );
        let points = jagged(rig, (step, 500 + index, 0), CORE, target, 4);
        nodes.push(strike(&points, 1.0 + energy, energy));
    }
    Node::group(nodes).blend_mode(BlendMode::Plus)
}

/// Embers thrown off the orb, recycled on a short loop.
fn sparks(rig: Rig, clock: f64, energy: f64) -> Node {
    let mut dots = Vec::new();
    for index in 0..rig.count / 2 {
        let age = (clock * 0.9 + rnd(rig, "spark-phase", index)).rem_euclid(1.0);
        let angle = rnd(rig, "spark-angle", index) * TAU;
        let speed = 0.4 + 0.9 * rnd(rig, "spark-speed", index);
        let distance = 24.0 + 280.0 * age * speed * (0.3 + energy);
        let at = point(
            CORE.x + math::cos(angle) * distance,
            CORE.y + math::sin(angle) * distance + 90.0 * age * age,
        );
        let alpha = (1.0 - age) * energy;
        dots.push(Node::from(
            Circle::new(at, 1.2 + 1.6 * (1.0 - age) * rnd(rig, "spark-size", index))
                .fill(color(0xd8f2ff).with_alpha(alpha * 0.8)),
        ));
    }
    Node::group(dots).blend_mode(BlendMode::Plus)
}
