//! The diner set for Last Order, built in depth planes from the rainy street backdrop to
//! the counter and dishes, so one lens can rack focus between them.

use super::super::label;
use super::{
    AMBER, Atmospheres, CYAN, HEIGHT, HORIZON, MAGENTA, MONO, WIDTH, beat_time, bulb, color,
    exposure, mix, sheen,
};
use motion::prelude::*;

/// The scene is built in planes, far to near, so one lens can focus between them.
pub(super) fn world(context: &SceneContext<'_>, seed: u64, atmosphere: &Atmospheres) -> Node {
    world_with_street(context, seed, atmosphere, street_plane(context))
}

/// The street plane alone: the only plane that still moves once `Look::film` takes over, so
/// it is the only one that needs shutter samples.
pub(super) fn street_plane(context: &SceneContext<'_>) -> Node {
    street(context, beat_time(context))
}

/// Builds the planes around an already evaluated street, such as a shutter-captured one.
pub(super) fn world_with_street(
    context: &SceneContext<'_>,
    seed: u64,
    atmosphere: &Atmospheres,
    street: Node,
) -> Node {
    let t = beat_time(context);
    let lens = lens(t);
    let planes = [
        (4000.0, backdrop(context, seed)),
        (1800.0, street),
        (1500.0, atmosphere.fog.clone()),
        (800.0, window(context, seed)),
        (450.0, counter(context, t)),
    ];
    let mut nodes = vec![Node::from(
        Rect::new(-80.0, -80.0, WIDTH + 80.0, HEIGHT + 80.0).fill(color(0x05060e)),
    )];
    for (depth, plane) in planes {
        let radius = lens.radius(depth).unwrap_or(0.0);
        nodes.push(if radius > 0.4 {
            plane.filter(Filter::Optics(Optics::Bokeh {
                radius,
                shape: ApertureShape::Polygon {
                    sides: 6,
                    rotation: 0.3,
                },
                highlight_boost: 0.8,
            }))
        } else {
            plane
        });
    }
    nodes.push(atmosphere.dust.clone());
    nodes.push(atmosphere.leak.clone());
    Node::group(nodes)
}

/// Aperture opens on the third beat, then focus racks from the street to the counter
/// and drifts back to a middle distance for the grade.
fn lens(t: f64) -> DepthOfField {
    let open = math::smoothstep(3.0, 3.35, t);
    let settle = math::smoothstep(5.0, 6.0, t);
    let wide = mix(22f64.ln(), 1.4f64.ln(), open);
    let f_stop = math::exp(mix(wide, 4.0f64.ln(), settle));
    let rack = math::smoothstep(3.45, 4.0, t);
    let focus = mix(mix(1800.0, 450.0, rack), 1500.0, settle);
    DepthOfField {
        focus_distance: focus,
        f_stop,
        focal_length: 140.0,
        max_radius: 16.0,
        perspective: false,
    }
}

/// Night sky, skyline and the far lights that turn into bokeh once the lens opens.
fn backdrop(context: &SceneContext<'_>, seed: u64) -> Node {
    context.memo(("backdrop", seed), || {
        let sky = Rect::new(-80.0, -80.0, WIDTH + 80.0, HORIZON).fill(
            LinearGradient::new(Point::new(0.0, 0.0), Point::new(0.0, HORIZON))
                .stop(0.0, color(0x050a1c))
                .stop(0.6, color(0x161a40))
                .stop(1.0, color(0x3a2450))
                .units(GradientUnits::UserSpace),
        );
        let mut skyline = Path2D::new();
        let mut windows = Path2D::new();
        let mut x = -40.0;
        let mut index = 0u64;
        while x < WIDTH + 40.0 {
            let width = 26.0 + context.random(("tower-width", seed, index)) * 44.0;
            let height = 40.0 + context.random(("tower-height", seed, index)).powi(2) * 130.0;
            rect(&mut skyline, x, HORIZON - height, x + width, HORIZON + 4.0);
            let mut row = HORIZON - height + 8.0;
            let mut cell = 0u64;
            while row < HORIZON - 8.0 {
                let mut column = x + 4.0;
                while column < x + width - 5.0 {
                    if context.random(("window", seed, index, cell)) > 0.8 {
                        rect(&mut windows, column, row, column + 3.0, row + 4.0);
                    }
                    cell += 1;
                    column += 8.0;
                }
                row += 10.0;
            }
            x += width + 2.0;
            index += 1;
        }
        let palette = [AMBER, CYAN, MAGENTA, 0x7dffb2, 0xff5a4a, 0xffe49a];
        let lights = (0..46)
            .map(|light| {
                let x = context.random(("far-x", seed, light)) * WIDTH;
                let y = HORIZON - 12.0 - context.random(("far-y", seed, light)) * 170.0;
                let radius = 2.2 + context.random(("far-r", seed, light)) * 3.0;
                let hue = palette[(context.random(("far-hue", seed, light)) * 6.0) as usize % 6];
                bulb(x, y, radius, hue)
            })
            .collect();
        Node::group([
            sky.into(),
            skyline.fill(color(0x0a0c1f)).into(),
            windows.fill(color(AMBER).with_alpha(0.45)).into(),
            exposure(lights, 2.4),
        ])
    })
}

fn rect(path: &mut Path2D, left: f64, top: f64, right: f64, bottom: f64) {
    path.move_to(Point::new(left, top))
        .line_to(Point::new(right, top))
        .line_to(Point::new(right, bottom))
        .line_to(Point::new(left, bottom))
        .close();
}

/// The street across the diner: a neon sign, lamps, wet asphalt and passing cars.
fn street(context: &SceneContext<'_>, t: f64) -> Node {
    let flicker = 0.82 + 0.18 * math::sin(t * 41.0) * math::sin(t * 9.0);
    let mut cars = Vec::new();
    for (start, forward) in [(2.0, false), (7.0, true)] {
        if (start - 0.2..start + 1.3).contains(&t) {
            cars.push(car(t, start, forward));
        }
    }
    let fixed = context.memo("street-fixed", || {
        let mut front = Path2D::new();
        let mut lit = Path2D::new();
        for (index, (left, right, top)) in [
            (-20.0, 150.0, 214.0),
            (160.0, 330.0, 236.0),
            (340.0, 520.0, 200.0),
            (530.0, 830.0, 168.0),
            (840.0, 990.0, 226.0),
        ]
        .into_iter()
        .enumerate()
        {
            rect(&mut front, left, top, right, HORIZON + 2.0);
            let mut row = top + 14.0;
            while row < HORIZON - 12.0 {
                let mut column = left + 12.0;
                let mut cell = 0u64;
                while column < right - 14.0 {
                    if context.random(("lit", index, row as u64, cell)) > 0.7 {
                        rect(&mut lit, column, row, column + 10.0, row + 14.0);
                    }
                    cell += 1;
                    column += 20.0;
                }
                row += 28.0;
            }
        }
        let road = Rect::new(-80.0, HORIZON, WIDTH + 80.0, HEIGHT + 80.0).fill(
            LinearGradient::new(Point::new(0.0, HORIZON), Point::new(0.0, HEIGHT))
                .stop(0.0, color(0x1b1c36))
                .stop(0.35, color(0x0d0e1d))
                .stop(1.0, color(0x05060e))
                .units(GradientUnits::UserSpace),
        );
        let mut lamps = Vec::new();
        let mut poles = Path2D::new();
        let mut glow = Vec::new();
        for x in [150.0, 470.0, 820.0] {
            rect(&mut poles, x - 2.0, 150.0, x + 2.0, HORIZON + 8.0);
            rect(&mut poles, x - 2.0, 148.0, x + 24.0, 152.0);
            lamps.push(bulb(x + 22.0, 156.0, 11.0, AMBER));
            glow.push(sheen(x + 22.0, HORIZON + 32.0, 14.0, 90.0, AMBER, 0.55));
            glow.push(sheen(x + 22.0, HORIZON + 26.0, 120.0, 22.0, AMBER, 0.25));
        }
        Node::group([
            road.into(),
            front.fill(color(0x0e1230)).into(),
            lit.fill(color(0xffc27a).with_alpha(0.5)).into(),
            poles.fill(color(0x04050c)).into(),
            exposure(lamps, 2.8),
            Node::group(glow),
            Rect::new(-80.0, HORIZON, WIDTH + 80.0, HORIZON + 14.0)
                .fill(color(0x2a2f55).with_alpha(0.7))
                .into(),
        ])
    });
    // The sign is outside the memo because it flickers.
    let sign = sign(flicker);
    Node::group([
        fixed,
        sign,
        sheen(700.0, HORIZON + 44.0, 120.0, 70.0, MAGENTA, 0.5 * flicker),
        sheen(640.0, HORIZON + 40.0, 22.0, 80.0, CYAN, 0.4),
        Node::group(cars),
    ])
}

fn sign(flicker: f64) -> Node {
    let board = RoundedRect::new(Rect::new(574.0, 178.0, 826.0, 262.0), 8.0).fill(color(0x070815));
    let tube =
        RoundedRect::new(Rect::new(582.0, 186.0, 818.0, 254.0), 6.0).stroke(3.0, color(CYAN));
    let text = label("ROOMS", MONO, 46.0, 8.0, FontWeight::BOLD, color(MAGENTA))
        .translate(606.0, 190.0)
        .opacity(flicker);
    Node::group([board.into(), exposure(vec![tube.into(), text], 2.2)])
}

/// A car crossing the street in one second; the lights are the only bright parts.
fn car(t: f64, start: f64, forward: bool) -> Node {
    let run = ((t - start + 0.2) / 1.5).clamp(0.0, 1.0);
    let heading = if forward { 1.0 } else { -1.0 };
    let x = if forward {
        mix(-160.0, 1120.0, run)
    } else {
        mix(1120.0, -160.0, run)
    };
    let y = HORIZON + 38.0;
    let front = x + heading * 78.0;
    let rear = x - heading * 78.0;
    let beam = Polygon::new([
        Point::new(front, y - 4.0),
        Point::new(front + heading * 210.0, y - 26.0),
        Point::new(front + heading * 210.0, y + 22.0),
        Point::new(front, y + 3.0),
    ])
    .fill(
        LinearGradient::new(Point::new(front, y), Point::new(front + heading * 210.0, y))
            .stop(0.0, color(0xfff0cc).with_alpha(0.55))
            .stop(1.0, color(0xfff0cc).with_alpha(0.0))
            .units(GradientUnits::UserSpace),
    );
    Node::group([
        Node::from(beam).blend_mode(BlendMode::Plus),
        RoundedRect::new(Rect::new(x - 80.0, y - 20.0, x + 80.0, y + 10.0), 10.0)
            .fill(color(0x040409))
            .into(),
        RoundedRect::new(Rect::new(x - 44.0, y - 38.0, x + 40.0, y - 14.0), 10.0)
            .fill(color(0x040409))
            .into(),
        exposure(
            vec![
                bulb(front, y - 3.0, 12.0, 0xfff0cc),
                bulb(rear, y - 6.0, 7.0, 0xff3a2a),
            ],
            3.4,
        ),
        sheen(front, y + 34.0, 18.0, 60.0, 0xfff0cc, 0.6),
        sheen(rear, y + 30.0, 14.0, 40.0, 0xff3a2a, 0.5),
    ])
}

/// The diner window: frame, rain beads and streaks on the glass.
fn window(context: &SceneContext<'_>, seed: u64) -> Node {
    context.memo(("window", seed), || {
        let mut frame = Path2D::new();
        rect(&mut frame, -80.0, -80.0, WIDTH + 80.0, 30.0);
        rect(&mut frame, -80.0, -80.0, 46.0, HEIGHT + 80.0);
        rect(&mut frame, WIDTH - 46.0, -80.0, WIDTH + 80.0, HEIGHT + 80.0);
        rect(&mut frame, -80.0, 392.0, WIDTH + 80.0, HEIGHT + 80.0);
        let palette = [CYAN, MAGENTA, AMBER, 0xffffff];
        let mut beads = Vec::new();
        let mut trails = Path2D::new();
        for drop in 0..64u64 {
            let x = 46.0 + context.random(("drop-x", seed, drop)) * (WIDTH - 92.0);
            let y = 30.0 + context.random(("drop-y", seed, drop)) * 360.0;
            let radius = 1.2 + context.random(("drop-r", seed, drop)) * 2.2;
            let hue = palette[(context.random(("drop-hue", seed, drop)) * 4.0) as usize % 4];
            beads.push(Node::from(
                Circle::new(Point::new(x, y), radius).fill(color(hue).with_alpha(0.4)),
            ));
            if drop % 5 == 0 {
                trails
                    .move_to(Point::new(x, y - 26.0 - radius * 8.0))
                    .line_to(Point::new(x, y - radius));
            }
        }
        Node::group([
            Node::from(trails.stroke(1.1, Color::WHITE.with_alpha(0.06))),
            exposure(beads, 0.6),
            frame.fill(color(0x04050b)).into(),
        ])
    })
}

/// The foreground counter and three dishes. Each one lifts off the formica in turn,
/// and its shadow stack grows with the height.
fn counter(context: &SceneContext<'_>, t: f64) -> Node {
    let top = context.memo("counter-top", || {
        Node::group([
            Rect::new(-80.0, 392.0, WIDTH + 80.0, HEIGHT + 80.0)
                .fill(
                    LinearGradient::new(Point::new(0.0, 392.0), Point::new(0.0, HEIGHT))
                        .stop(0.0, color(0xaeb9c6))
                        .stop(0.2, color(0x8693a2))
                        .stop(1.0, color(0x3c4655))
                        .units(GradientUnits::UserSpace),
                )
                .into(),
            Rect::new(-80.0, 392.0, WIDTH + 80.0, 396.0)
                .fill(Color::WHITE.with_alpha(0.55))
                .into(),
            sheen(700.0, 436.0, 220.0, 26.0, MAGENTA, 0.35),
            sheen(230.0, 440.0, 200.0, 24.0, CYAN, 0.3),
        ])
    });
    let heights = [24.0, 16.0, 32.0];
    let mut items = vec![top];
    for (index, (x, kind)) in [(240.0, Dish::Cup), (490.0, Dish::Pie), (730.0, Dish::Shake)]
        .into_iter()
        .enumerate()
    {
        let begin = 4.0 + 0.06 * index as f64;
        let rise = math::smoothstep(begin, begin + 0.35, t)
            - math::smoothstep(begin + 0.55, begin + 0.9, t);
        let height = 3.0 + heights[index] * rise;
        let elevation = Elevation::shadow(height)
            .and_then(|shadow| shadow.direction(Vector2::new(0.6, 1.0)))
            .map(|shadow| shadow.color(color(0x02030a)))
            .expect("height is clamped to the supported range");
        items.push(
            dish(x, 478.0, kind)
                .translate(0.0, -(height - 3.0) * 1.5)
                .elevated(elevation),
        );
    }
    Node::group(items)
}

#[derive(Clone, Copy)]
enum Dish {
    Cup,
    Pie,
    Shake,
}

fn dish(x: f64, y: f64, kind: Dish) -> Node {
    let saucer =
        Ellipse::new(Point::new(x, y + 8.0), Vector2::new(74.0, 15.0)).fill(color(0xe9e3d6));
    let rim = Ellipse::new(Point::new(x, y + 6.0), Vector2::new(54.0, 9.0)).fill(color(0xcfc7b7));
    let body: Vec<Node> = match kind {
        Dish::Cup => {
            let mut steam = Path2D::new();
            for strand in 0..3 {
                let base = x - 12.0 + strand as f64 * 12.0;
                steam
                    .move_to(Point::new(base, y - 44.0))
                    .quadratic_to(Point::new(base + 9.0, y - 62.0), Point::new(base, y - 80.0));
            }
            vec![
                RoundedRect::new(Rect::new(x - 32.0, y - 40.0, x + 32.0, y + 4.0), 14.0)
                    .fill(color(0xf5f0e6))
                    .into(),
                Ellipse::new(Point::new(x, y - 40.0), Vector2::new(32.0, 7.0))
                    .fill(color(0x3a2216))
                    .into(),
                Ring::new(Point::new(x + 36.0, y - 20.0), 8.0, 12.0)
                    .fill(color(0xf5f0e6))
                    .into(),
                steam.stroke(2.0, Color::WHITE.with_alpha(0.3)).into(),
            ]
        }
        Dish::Pie => vec![
            Polygon::new([
                Point::new(x - 50.0, y + 4.0),
                Point::new(x + 50.0, y + 4.0),
                Point::new(x + 36.0, y - 30.0),
                Point::new(x - 36.0, y - 30.0),
            ])
            .fill(color(0xc98a46))
            .into(),
            Ellipse::new(Point::new(x, y - 30.0), Vector2::new(36.0, 7.0))
                .fill(color(0xf0bd75))
                .into(),
            Circle::new(Point::new(x - 8.0, y - 34.0), 6.0)
                .fill(color(0xd8283c))
                .into(),
        ],
        Dish::Shake => vec![
            Polygon::new([
                Point::new(x - 30.0, y - 62.0),
                Point::new(x + 30.0, y - 62.0),
                Point::new(x + 22.0, y + 4.0),
                Point::new(x - 22.0, y + 4.0),
            ])
            .fill(color(0xc9eef2).with_alpha(0.9))
            .into(),
            Polygon::new([
                Point::new(x - 27.0, y - 44.0),
                Point::new(x + 27.0, y - 44.0),
                Point::new(x + 21.0, y + 2.0),
                Point::new(x - 21.0, y + 2.0),
            ])
            .fill(color(0xff7fb4))
            .into(),
            Rect::new(x + 6.0, y - 92.0, x + 10.0, y - 40.0)
                .fill(color(0xffffff))
                .into(),
        ],
    };
    Node::group([vec![saucer.into(), rim.into()], body].concat())
}
