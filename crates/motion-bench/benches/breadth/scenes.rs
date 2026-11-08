//! Deterministic breadth benchmark scenes: shared palette and layout helpers plus the
//! `artwork` builder that renders every named case.

use super::DESIGN;
use motion::particles::{
    Ballistic, BurstEmitter, Drag, Gravity, ParticleSimulation, ParticleStyle, ParticleTrajectory,
};
use motion::prelude::*;
use motion::render::{PathClip, RectClip};

pub(super) fn teal() -> Color {
    Color::from_hex(0x178b79)
}

fn blue() -> Color {
    Color::from_hex(0x327fc1)
}

fn coral() -> Color {
    Color::from_hex(0xf47762)
}

pub(super) fn grid(columns: u32, rows: u32, build: impl Fn(Rect, u32) -> Node) -> Node {
    let width = DESIGN.width / f64::from(columns);
    let height = DESIGN.height / f64::from(rows);
    Node::group((0..columns * rows).map(|index| {
        let left = f64::from(index % columns) * width;
        let top = f64::from(index / columns) * height;
        build(
            Rect::new(
                left + 1.0,
                top + 1.0,
                left + width - 1.0,
                top + height - 1.0,
            ),
            index,
        )
    }))
}

pub(super) fn gradient(kind: &str, index: u32) -> Paint {
    let first = [teal(), blue(), coral()][index as usize % 3];
    let second = [coral(), teal(), blue()][index as usize % 3];
    match kind {
        "gradient-linear" => LinearGradient::new(Point::new(0.0, 0.0), Point::new(1.0, 1.0))
            .stop(0.0, first)
            .stop(0.5, Color::WHITE)
            .stop(1.0, second)
            .into(),
        "gradient-radial" => RadialGradient::new(Point::new(0.5, 0.5), 0.6)
            .stop(0.0, first)
            .stop(1.0, second)
            .into(),
        _ => SweepGradient::new(Point::new(0.5, 0.5), 0.0, std::f64::consts::TAU)
            .stop(0.0, first)
            .stop(0.5, second)
            .stop(1.0, first)
            .into(),
    }
}

fn star(points: u32, center: Point, outer: f64, inner: f64) -> Path2D {
    let mut path = Path2D::new();
    for index in 0..points * 2 {
        let radius = if index % 2 == 0 { outer } else { inner };
        let angle = f64::from(index) * std::f64::consts::PI / f64::from(points);
        let point = Point::new(
            center.x + radius * math::cos(angle),
            center.y + radius * math::sin(angle),
        );
        if index == 0 {
            path.try_move_to(point).unwrap();
        } else {
            path.try_line_to(point).unwrap();
        }
    }
    path.try_close().unwrap();
    path
}

pub(super) fn wave(row: u32) -> Path2D {
    let y = 20.0 + f64::from(row) * 8.0;
    let mut path = Path2D::new();
    path.try_move_to(Point::new(10.0, y)).unwrap();
    for segment in 0..6 {
        let x = 10.0 + f64::from(segment) * 100.0;
        path.try_cubic_to(
            Point::new(x + 30.0, y - 14.0),
            Point::new(x + 70.0, y + 14.0),
            Point::new(x + 100.0, y),
        )
        .unwrap();
    }
    path
}

pub(super) fn labels(count: u32) -> Node {
    grid(20, count / 20, |bounds, index| {
        Node::from(
            Text::new(format!("L{index:03}"))
                .size(9.0)
                .fill(Color::BLACK),
        )
        .translate(bounds.left, bounds.top)
    })
}

pub(super) fn artwork(kind: &str, context: &SceneContext<'_>) -> Node {
    let node = match kind {
        "gradient-linear" | "gradient-radial" | "gradient-sweep" => grid(8, 4, |bounds, index| {
            Node::from(bounds.fill(gradient(kind, index)))
        }),
        "blend-modes" => {
            let modes = [
                BlendMode::Multiply,
                BlendMode::Screen,
                BlendMode::Overlay,
                BlendMode::Darken,
                BlendMode::Lighten,
                BlendMode::Difference,
                BlendMode::Exclusion,
                BlendMode::HardLight,
                BlendMode::SoftLight,
                BlendMode::ColorDodge,
                BlendMode::Plus,
                BlendMode::SourceOver,
            ];
            Node::group([
                Node::from(context.bounds().fill(Color::from_hex(0xe8dfc8))),
                grid(4, 3, |bounds, index| {
                    Node::group([
                        Node::from(RoundedRect::new(bounds, 6.0).fill(teal())),
                        Node::from(
                            Circle::new(bounds.center(), bounds.height() * 0.42).fill(coral()),
                        )
                        .blend_mode(modes[index as usize % modes.len()]),
                    ])
                }),
            ])
        }
        "clip-path" => Node::group((0..24).map(|row| {
            Node::from(
                Rect::new(
                    0.0,
                    f64::from(row) * 15.0,
                    640.0,
                    f64::from(row) * 15.0 + 9.0,
                )
                .fill([teal(), blue()][row as usize % 2]),
            )
        }))
        .clip(PathClip::new(star(
            7,
            Point::new(320.0, 180.0),
            170.0,
            70.0,
        ))),
        "clip-nested" => Node::group((0..16).map(|index| {
            Node::from(
                Circle::new(
                    Point::new(
                        f64::from(index % 4) * 150.0 + 80.0,
                        f64::from(index / 4) * 80.0 + 50.0,
                    ),
                    60.0,
                )
                .fill([teal(), coral(), blue()][index as usize % 3]),
            )
        }))
        .clip(RectClip::new(Rect::new(40.0, 20.0, 600.0, 340.0)))
        .clip(Circle::new(Point::new(320.0, 180.0), 190.0)),
        "stroke-dash" => Node::group((0..40).map(|row| {
            Node::from(
                wave(row)
                    .stroke(2.0, Color::BLACK)
                    .dash(&[8.0, 4.0, 2.0, 4.0], f64::from(row)),
            )
        })),
        "stroke-wide" => Node::group((0..12).map(|row| {
            Node::from(wave(row * 3).stroke(
                24.0,
                [teal(), blue(), coral()][row as usize % 3].with_alpha(0.5),
            ))
        })),
        // A stroke input is capped at 256 verbs, so the strands are split across paths.
        "hairlines" => context.memo("breadth-hairlines", || {
            Node::group((0..15).map(|strand| {
                let mut path = Path2D::new();
                for index in 0..100 {
                    let x = f64::from(index) * 6.4;
                    let y = f64::from(strand) * 23.0;
                    path.try_move_to(Point::new(x, y)).unwrap();
                    path.try_line_to(Point::new(x + 20.0, y + 30.0)).unwrap();
                }
                Node::from(path.stroke(0.5, Color::BLACK))
            }))
        }),
        "path-complex" => context.memo("breadth-path-complex", || {
            let mut path = Path2D::new();
            for index in 0..240 {
                let angle = f64::from(index) * 2.399_963;
                let radius = 8.0 + f64::from(index) * 0.7;
                let point = Point::new(
                    320.0 + radius * math::cos(angle),
                    180.0 + radius * math::sin(angle) * 0.9,
                );
                if index == 0 {
                    path.try_move_to(point).unwrap();
                } else {
                    path.try_line_to(point).unwrap();
                }
            }
            path.try_close().unwrap();
            Node::from(path.fill(blue()).fill_rule(FillRule::EvenOdd))
        }),
        "layers-nested" => (0..8).fold(
            Node::from(Rect::new(40.0, 40.0, 600.0, 320.0).fill(teal())),
            |inner, depth| {
                Node::group([
                    Node::from(
                        Circle::new(Point::new(80.0 + f64::from(depth) * 70.0, 180.0), 50.0)
                            .fill(coral()),
                    ),
                    inner,
                ])
                .opacity(0.9)
            },
        ),
        "layers-wide" => grid(20, 10, |bounds, index| {
            Node::group([
                Node::from(RoundedRect::new(bounds, 3.0).fill(teal())),
                Node::from(Circle::new(bounds.center(), 5.0).fill(coral())),
            ])
            .opacity(0.4 + f64::from(index % 5) * 0.1)
        }),
        "nodes-1k" | "nodes-10k" | "nodes-50k" => {
            let count: u32 = match kind {
                "nodes-1k" => 1_000,
                "nodes-10k" => 10_000,
                _ => 50_000,
            };
            context.memo(("breadth-nodes", count), || {
                let columns = (f64::from(count) * 16.0 / 9.0).sqrt().ceil() as u32;
                let rows = count.div_ceil(columns);
                let (width, height) = (
                    DESIGN.width / f64::from(columns),
                    DESIGN.height / f64::from(rows),
                );
                Node::group((0..count).map(|index| {
                    let left = f64::from(index % columns) * width;
                    let top = f64::from(index / columns) * height;
                    Node::from(
                        Rect::new(left, top, left + width * 0.7, top + height * 0.7)
                            .fill([teal(), blue(), coral()][index as usize % 3]),
                    )
                }))
            })
        }
        "layout-grid" => context.memo("breadth-layout-grid", || {
            Column::new().gap(2.0).children((0..20).map(|row| {
                Row::new().gap(2.0).children((0..40).map(move |column| {
                    Node::from(
                        Rect::new(0.0, 0.0, 12.0, 12.0)
                            .fill([teal(), blue()][(row + column) as usize % 2]),
                    )
                }))
            }))
        }),
        "filter-siblings" => {
            Node::group([(24.0, 5.0), (224.0, 9.0), (424.0, 14.0)].into_iter().map(
                |(left, sigma)| {
                    Rect::new(left, 50.0, left + 160.0, 310.0)
                        .fill(Color::rgb(20, 150, 210))
                        .filter(Filter::Blur { sigma })
                },
            ))
        }
        "blur-large" => Node::group([
            Node::from(Circle::new(Point::new(220.0, 180.0), 110.0).fill(coral())),
            Node::from(RoundedRect::new(Rect::new(260.0, 90.0, 560.0, 270.0), 24.0).fill(blue())),
        ])
        .filter(Filter::Blur { sigma: 24.0 }),
        "filter-chromatic" => grid(8, 4, |bounds, index| {
            Node::from(bounds.fill([teal(), coral(), blue()][index as usize % 3]))
        })
        .filter(Filter::Optics(motion::Optics::Lens {
            distortion: 0.0,
            chromatic: 0.01,
            auto_fit: false,
            center: None,
        })),
        "filter-chain" => grid(8, 4, |bounds, index| {
            Node::from(
                RoundedRect::new(bounds, 8.0).fill([teal(), coral(), blue()][index as usize % 3]),
            )
        })
        .filter(Filter::ColorAdjust {
            exposure: 0.2,
            contrast: 1.2,
            saturation: 1.3,
            hue_degrees: 12.0,
        })
        .filter(Filter::Optics(motion::Optics::LensVignette {
            angle_of_view: 75.0,
            strength: 0.6,
            roundness: 0.0,
        }))
        .filter(Filter::grain(9, 0.05)),
        "filter-bloom" => grid(8, 4, |bounds, index| {
            Node::from(bounds.fill([teal(), coral(), blue()][index as usize % 3]))
        })
        .filter(Filter::ColorAdjust {
            exposure: 2.0,
            contrast: 1.0,
            saturation: 1.0,
            hue_degrees: 0.0,
        })
        .filter(Filter::Bloom {
            threshold: 0.9,
            knee: 0.2,
            radius: 3.0,
            levels: 4,
            falloff: 0.5,
            intensity: 0.2,
            tint: Color::WHITE,
        }),
        "filter-lens" => grid(8, 4, |bounds, index| {
            Node::from(bounds.fill([teal(), coral(), blue()][index as usize % 3]))
        })
        .filter(Filter::Optics(Optics::Lens {
            distortion: 0.08,
            chromatic: 0.002,
            auto_fit: true,
            center: None,
        })),
        "filter-lut" => context.memo("breadth-original-lut", || {
            let lut = ColorWheels {
                shadows: [-0.01, 0.01, 0.02],
                midtones: [0.0; 3],
                highlights: [0.02, 0.01, -0.01],
            }
            .bake(17, 1.0, Cdl::default())
            .unwrap();
            grid(8, 4, |bounds, index| {
                Node::from(bounds.fill([teal(), coral(), blue()][index as usize % 3]))
            })
            .filter(Filter::Lut {
                lut: std::sync::Arc::new(lut),
                input: LutInput::Linear,
            })
        }),
        "display-transform" => grid(8, 4, |bounds, index| {
            Node::from(bounds.fill([teal(), coral(), blue()][index as usize % 3]))
        })
        .filter(Filter::ColorAdjust {
            exposure: 2.0,
            contrast: 1.0,
            saturation: 1.0,
            hue_degrees: 0.0,
        }),
        "text-paragraph" => Node::from(
            Text::new("Motion keeps geometry, time and typography in agreement. ".repeat(10))
                .size(16.0)
                .max_width(580.0)
                .fill(Color::BLACK),
        )
        .translate(30.0, 20.0),
        "text-labels" => context.memo("breadth-labels", || labels(200)),
        "particles-ballistic" => {
            let system = context.cache("breadth-ballistic", || {
                ParticleTrajectory::new(
                    Ballistic::burst(Point::new(320.0, 100.0), 20_000)
                        .speed(40.0, 220.0)
                        .gravity(Vector2::new(0.0, 160.0))
                        .drag(0.4),
                )
                .style(ParticleStyle::Dots { radius: 1.2 })
            });
            Node::group([
                Node::from(context.bounds().fill(Color::BLACK)),
                system.render(context),
            ])
        }
        "particles-simulation" => {
            let system = context.cache("breadth-simulation", || {
                ParticleSimulation::new(7)
                    .duration(3.seconds())
                    .emitter(BurstEmitter::new(
                        Time::ZERO,
                        6_000,
                        Point::new(320.0, 120.0),
                    ))
                    .modifier(Gravity::new(Vector2::new(0.0, 90.0)))
                    .modifier(Drag::new(0.3))
                    .style(ParticleStyle::GlowDots { radius: 1.8 })
            });
            Node::group([
                Node::from(context.bounds().fill(Color::BLACK)),
                system.try_render(context).unwrap(),
            ])
        }
        "particles-sub-emitters" | "particles-collision" | "particles-parallel" => {
            let system = context.cache(("breadth-particle-extension", kind.to_owned()), || {
                let preset = if kind == "particles-parallel" {
                    motion::particles::presets::Preset::Confetti
                } else {
                    motion::particles::presets::Preset::Rain
                };
                let mut options = motion::particles::presets::PresetOptions::new(
                    7,
                    3.seconds(),
                    context.bounds(),
                );
                options.density = 0.2;
                let system = preset.simulation(options).unwrap();
                if kind == "particles-parallel" {
                    system.workers(4)
                } else {
                    system
                }
            });
            Node::group([
                context.bounds().fill(Color::BLACK).into(),
                system.render(context),
            ])
        }
        other => unreachable!("unknown breadth case {other}"),
    };
    node.translate(math::sin(context.seconds() * 2.0) * 6.0, 0.0)
}
