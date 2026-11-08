//! Shared painting helpers for Quiet Hours: keyed randomness, gradients, ellipse and
//! ridge silhouettes, foliage masses, wind-swayed grass rows and the slow camera drift.

use super::{HEIGHT, KAPPA, Rig, WIDTH};
use motion::prelude::*;
use std::f64::consts::TAU;

pub(super) fn color(hex: u32) -> Color {
    Color::from_hex(hex)
}

pub(super) fn point(x: f64, y: f64) -> Point {
    Point::new(x, y)
}

/// Key-derived value in [0, 1); one stream per (seed, tag, index).
pub(super) fn rnd(rig: Rig, tag: &str, index: u64) -> f64 {
    Random::from_key((rig.seed, tag, index)).next_float()
}

pub(super) fn mix(from: f64, to: f64, amount: f64) -> f64 {
    from + (to - from) * amount
}

pub(super) fn vertical(top: f64, bottom: f64, stops: &[(f64, Color)]) -> LinearGradient {
    stops
        .iter()
        .fold(
            LinearGradient::new(point(0.0, top), point(0.0, bottom)),
            |gradient, (position, color)| gradient.stop(*position, *color),
        )
        .units(GradientUnits::UserSpace)
}

pub(super) fn glow_disc(center: Point, radius: f64, stops: &[(f64, Color)]) -> RadialGradient {
    stops
        .iter()
        .fold(
            RadialGradient::new(center, radius),
            |gradient, (position, color)| gradient.stop(*position, *color),
        )
        .units(GradientUnits::UserSpace)
}

pub(super) fn ellipse(path: &mut Path2D, cx: f64, cy: f64, rx: f64, ry: f64) {
    let (kx, ky) = (rx * KAPPA, ry * KAPPA);
    path.move_to(point(cx + rx, cy))
        .cubic_to(
            point(cx + rx, cy + ky),
            point(cx + kx, cy + ry),
            point(cx, cy + ry),
        )
        .cubic_to(
            point(cx - kx, cy + ry),
            point(cx - rx, cy + ky),
            point(cx - rx, cy),
        )
        .cubic_to(
            point(cx - rx, cy - ky),
            point(cx - kx, cy - ry),
            point(cx, cy - ry),
        )
        .cubic_to(
            point(cx + kx, cy - ry),
            point(cx + rx, cy - ky),
            point(cx + rx, cy),
        )
        .close();
}

/// A filled ridge line that closes along `bottom`; three sines give a painted feel.
pub(super) fn ridge(
    rig: Rig,
    tag: &str,
    base: f64,
    amplitude: f64,
    scale: f64,
    bottom: f64,
) -> Path2D {
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

/// A lumpy foliage mass built from overlapping ellipses.
pub(super) fn foliage(
    path: &mut Path2D,
    rig: Rig,
    tag: &str,
    center: Point,
    spread: Vector2,
    lumps: u64,
    radius: f64,
) {
    for lump in 0..lumps {
        let angle = rnd(rig, tag, lump * 3) * TAU;
        let distance = math::sqrt(rnd(rig, tag, lump * 3 + 1));
        let size = radius * (0.65 + 0.7 * rnd(rig, tag, lump * 3 + 2));
        ellipse(
            path,
            center.x + math::cos(angle) * distance * spread.x,
            center.y + math::sin(angle) * distance * spread.y,
            size,
            size * 0.72,
        );
    }
}

fn blade(path: &mut Path2D, x: f64, base: f64, height: f64, width: f64, bend: f64) {
    path.move_to(point(x - width * 0.5, base))
        .quadratic_to(
            point(x - width * 0.3 + bend * 0.35, base - height * 0.55),
            point(x + bend, base - height),
        )
        .quadratic_to(
            point(x + width * 0.3 + bend * 0.35, base - height * 0.55),
            point(x + width * 0.5, base),
        )
        .close();
}

pub(super) struct Grass<'a> {
    pub(super) tag: &'a str,
    pub(super) blades: u64,
    pub(super) base: f64,
    pub(super) height: (f64, f64),
    pub(super) width: f64,
    pub(super) sway: f64,
    pub(super) tones: &'a [Color],
}

/// A row of wind-swayed blades; one path per tone keeps it to a few fills.
pub(super) fn grass(rig: Rig, seconds: f64, row: &Grass) -> Node {
    let mut paths = vec![Path2D::new(); row.tones.len()];
    for index in 0..row.blades {
        let x = (index as f64 + rnd(rig, row.tag, index * 5)) / row.blades as f64 * (WIDTH + 60.0)
            - 30.0;
        let height = mix(row.height.0, row.height.1, rnd(rig, row.tag, index * 5 + 1));
        let lean = (rnd(rig, row.tag, index * 5 + 2) - 0.5) * height * 0.3;
        let phase = rnd(rig, row.tag, index * 5 + 3) * TAU;
        let wind = row.sway
            * (math::sin(seconds * 1.7 + x * 0.013 + phase * 0.2)
                + 0.6 * math::sin(seconds * 0.7 - x * 0.005))
            * height
            / 70.0;
        blade(
            &mut paths[index as usize % row.tones.len()],
            x,
            row.base + rnd(rig, row.tag, index * 5 + 4) * 10.0,
            height,
            row.width * (0.8 + 0.5 * rnd(rig, row.tag, index * 5 + 3)),
            lean + wind,
        );
    }
    Node::group(
        paths
            .into_iter()
            .zip(row.tones)
            .map(|(path, tone)| Node::from(path.fill(*tone)))
            .collect::<Vec<_>>(),
    )
}

pub(super) fn ground_below(top: f64, tone: Color) -> Node {
    Rect::new(-30.0, top, WIDTH + 30.0, HEIGHT + 30.0)
        .fill(tone)
        .into()
}

/// Slow push-in with a little lateral drift, always zoomed enough to hide the edges.
pub(super) fn drift(shot: Node, progress: f64, zoom: f64, pan: f64) -> Node {
    shot.pivot(Pivot::Point(point(WIDTH * 0.5, HEIGHT * 0.5)))
        .scale(zoom - 0.03 * progress)
        .translate((progress - 0.5) * pan, 0.0)
}
