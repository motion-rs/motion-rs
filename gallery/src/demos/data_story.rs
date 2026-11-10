//! A flat data-journalism explainer about a fictional harbour ferry: a
//! scale-mapped bar chart grows year by year, an axis-labelled line chart traces
//! falling commute times, keyframed counters tick up inside a measured row of
//! stat cards, and a timed caption track narrates the story underneath.

use crate::{DemoMetadata, DemoParameters};
use motion::captions::{CaptionCue, CaptionLayout, CaptionStyle, CaptionTrack};
use motion::diagram::{LineChart, Scale};
use motion::prelude::*;

pub(super) const METADATA: DemoMetadata = DemoMetadata::new(
    "data-story",
    "The Harbor Line",
    "Infographic",
    &[
        "scale-bar-chart",
        "line-chart",
        "keyframed-counters",
        "row-column-layout",
        "caption-track",
    ],
);

const PAPER: u32 = 0xf7f5f0;

const INK: u32 = 0x1f2a37;

const MUTED: u32 = 0x4b5563;

const GRID: u32 = 0xe2ded5;

const TEAL: u32 = 0x0f9d8a;

const CORAL: u32 = 0xff6b4a;

const AMBER: u32 = 0xe9a23b;

const YEARS: [f64; 10] = [
    2016.0, 2017.0, 2018.0, 2019.0, 2020.0, 2021.0, 2022.0, 2023.0, 2024.0, 2025.0,
];

const RIDERS: [f64; 10] = [3.1, 3.6, 4.2, 4.0, 1.9, 2.8, 4.6, 5.9, 7.2, 8.4];

const COMMUTE: [f64; 10] = [52.0, 51.0, 49.0, 48.0, 47.0, 45.0, 42.0, 39.0, 36.0, 33.0];

const BAR_PLOT: Rect = Rect::new(84.0, 136.0, 500.0, 380.0);

const LINE_PLOT: Rect = Rect::new(600.0, 150.0, 900.0, 292.0);

const NARRATION: [(f64, f64, &str); 4] = [
    (
        0.02,
        0.30,
        "In 2016 the Harbor Line carried 3.1 million riders.",
    ),
    (0.30, 0.55, "After the 2020 dip, ridership nearly tripled."),
    (
        0.55,
        0.80,
        "Meanwhile the average commute fell by 19 minutes.",
    ),
    (0.80, 1.00, "Port Avelin now moves by water."),
];

fn color(hex: u32) -> Color {
    Color::from_hex(hex)
}

fn smooth(value: f64) -> f64 {
    let value = value.clamp(0.0, 1.0);
    value * value * (3.0 - 2.0 * value)
}

fn window(progress: f64, start: f64, length: f64) -> f64 {
    smooth((progress - start) / length)
}

fn label(text: impl Into<String>, size: f64, hex: u32) -> Text {
    Text::new(text.into())
        .size(size)
        .weight(FontWeight::MEDIUM)
        .fill(color(hex))
}

pub(super) fn timeline(parameters: DemoParameters) -> motion::Result<Timeline> {
    let seconds = parameters.seconds as f64;
    let at = move |fraction: f64| (seconds * fraction).seconds();
    let track = CaptionTrack::new(
        NARRATION
            .iter()
            .map(|(start, end, text)| {
                CaptionCue::new(
                    std::time::Duration::from_secs_f64(seconds * start),
                    std::time::Duration::from_secs_f64(seconds * end),
                    *text,
                )
            })
            .collect::<Result<Vec<_>, _>>()?,
    )?;
    let style = CaptionStyle::new(Rect::new(48.0, 452.0, 912.0, 512.0))
        .font_size(22.0)
        .color(color(PAPER))
        .background(color(INK))
        .padding(18.0, 10.0)
        .keyword("tripled.", color(AMBER))
        .keyword("19", color(CORAL))
        .keyword("water.", color(TEAL));
    // Each counter eases to its final value, overshooting nothing and holding.
    let riders = Keyframes::new(0.0).to(at(0.12), 0.0, Easing::Linear).to(
        at(0.54),
        8.4,
        Easing::QuadraticInOut,
    );
    let growth = Keyframes::new(0.0).to(at(0.30), 0.0, Easing::Linear).to(
        at(0.60),
        171.0,
        Easing::QuarticOut,
    );
    let saved =
        Keyframes::new(0.0)
            .to(at(0.50), 0.0, Easing::Linear)
            .to(at(0.80), 19.0, Easing::CubicOut);
    let horizontal = Scale::linear((2016.0, 2025.0), (LINE_PLOT.left, LINE_PLOT.right))?;
    let vertical = Scale::linear((30.0, 55.0), (LINE_PLOT.bottom, LINE_PLOT.top))?;
    let bar_scale = Scale::linear((0.0, 9.0), (BAR_PLOT.bottom, BAR_PLOT.top))?;
    let bar_ticks = [0.0, 2.0, 4.0, 6.0, 8.0];

    Ok(
        Timeline::new(Size::new(960.0, 540.0), 30).track(|track_builder| {
            track_builder.scene(
                parameters.seconds.seconds(),
                move |context: &SceneContext<'_>| -> motion::Result<_> {
                    let progress = context.progress();
                    let time = context.time();
                    let mut nodes: Vec<Node> = vec![context.bounds().fill(color(PAPER)).into()];

                    let header = window(progress, 0.0, 0.08);
                    nodes.push(
                        Node::group([
                            Node::from(Rect::new(48.0, 30.0, 56.0, 88.0).fill(color(TEAL))),
                            label("THE HARBOR LINE", 32.0, INK)
                                .weight(FontWeight::SEMIBOLD)
                                .translate(70.0, 28.0),
                            label("How a ferry rewired Port Avelin's commute", 18.0, MUTED)
                                .translate(70.0, 68.0),
                            label("FICTIONAL DATA  |  2016-2025", 14.0, MUTED)
                                .translate(690.0, 40.0),
                        ])
                        .opacity(header)
                        .translate(0.0, (1.0 - header) * -12.0),
                    );

                    nodes.push(bar_chart(progress, bar_scale, &bar_ticks)?);
                    nodes.push(line_chart(context, progress, horizontal, vertical)?);

                    let cards = window(progress, 0.10, 0.10);
                    let riders = riders.sample(time);
                    let growth = growth.sample(time);
                    let saved = saved.sample(time);
                    let stat = |value: String, caption: &str, hex: u32| {
                        // The invisible strut fixes the column width so ticking digits
                        // never shift neighbouring cards inside the measured row.
                        Column::new().gap(4.0).children([
                            Node::from(label(value, 28.0, hex).weight(FontWeight::SEMIBOLD)),
                            Node::from(label(caption, 14.0, MUTED)),
                            Node::from(Rect::new(0.0, 0.0, 96.0, 1.0).fill(color(0xffffff))),
                        ])
                    };
                    nodes.push(
                        Node::group([
                            Node::from(Rect::new(560.0, 338.0, 912.0, 420.0).fill(color(0xffffff))),
                            Node::from(Rect::new(560.0, 338.0, 912.0, 341.0).fill(color(TEAL))),
                            Row::new()
                                .gap(12.0)
                                .alignment(Alignment::Start)
                                .children([
                                    stat(format!("{riders:.1}M"), "riders in 2025", 0x087f70),
                                    stat(format!("+{growth:.0}%"), "since 2016", 0x996400),
                                    stat(
                                        if saved < 0.5 {
                                            "0 min".to_owned()
                                        } else {
                                            format!("-{saved:.0} min")
                                        },
                                        "per commute",
                                        0xc54230,
                                    ),
                                ])
                                .translate(578.0, 356.0),
                        ])
                        .opacity(cards)
                        .translate(0.0, (1.0 - cards) * 16.0),
                    );

                    if let Some(fonts) = context
                        .assets()
                        .fonts()
                        .and_then(|fonts| fonts.downcast_ref::<FontLibrary>())
                    {
                        let captions: CaptionLayout =
                            style.layout(&track, context.size(), fonts)?;
                        nodes.push(captions.node(time));
                    }
                    Ok(Node::group(nodes))
                },
            )
        }),
    )
}

fn bar_chart(progress: f64, scale: Scale, ticks: &[f64]) -> motion::Result<Node> {
    let mut nodes =
        vec![label("Annual riders, millions", 18.0, INK).translate(BAR_PLOT.left, 100.0)];
    let axis = window(progress, 0.02, 0.08);
    for &tick in ticks {
        let y = scale.map(tick)?;
        nodes.push(
            Node::group([
                Node::from(Rect::new(BAR_PLOT.left, y, BAR_PLOT.right, y + 1.0).fill(color(GRID))),
                label(format!("{tick:.0}"), 14.0, MUTED).translate(BAR_PLOT.left - 26.0, y - 10.0),
            ])
            .opacity(axis),
        );
    }
    let slot = BAR_PLOT.width() / RIDERS.len() as f64;
    let highlight = window(progress, 0.80, 0.08);
    for (index, (&year, &value)) in YEARS.iter().zip(RIDERS.iter()).enumerate() {
        let grow = window(progress, 0.06 + index as f64 * 0.036, 0.12);
        let left = BAR_PLOT.left + slot * index as f64 + slot * 0.18;
        let right = left + slot * 0.64;
        let top = scale.map(value * grow)?;
        let latest = index + 1 == RIDERS.len();
        let fill = if index == 4 {
            color(CORAL)
        } else if latest {
            color(TEAL).interpolate(&color(AMBER), highlight)
        } else {
            color(TEAL).interpolate(&color(0xa7d8cf), highlight)
        };
        nodes.push(
            Rect::new(left, top, right, BAR_PLOT.bottom)
                .fill(fill)
                .into(),
        );
        nodes.push(
            label(format!("'{:02}", year as i64 % 100), 14.0, MUTED)
                .translate(left + 2.0, BAR_PLOT.bottom + 6.0)
                .opacity(axis),
        );
        let shown = window(progress, 0.14 + index as f64 * 0.036, 0.06);
        nodes.push(
            label(format!("{value:.1}"), 14.0, INK)
                .translate(left + 1.0, top - 18.0)
                .opacity(shown),
        );
    }
    nodes.push(
        Rect::new(
            BAR_PLOT.left,
            BAR_PLOT.bottom,
            BAR_PLOT.right,
            BAR_PLOT.bottom + 1.5,
        )
        .fill(color(INK))
        .into(),
    );
    // Callout tagging the record year once the story reaches its conclusion.
    let callout = Node::group([
        Node::from(Rect::new(316.0, 104.0, 500.0, 136.0).fill(color(INK))),
        label("Record year: 8.4M", 15.0, PAPER).translate(326.0, 111.0),
    ]);
    nodes.push(
        callout
            .opacity(highlight)
            .translate(0.0, (1.0 - highlight) * 8.0),
    );
    Ok(Node::group(nodes))
}

fn line_chart(
    context: &SceneContext<'_>,
    progress: f64,
    horizontal: Scale,
    vertical: Scale,
) -> motion::Result<Node> {
    let appear = window(progress, 0.40, 0.08);
    let draw = window(progress, 0.46, 0.30);
    let reach = draw * (YEARS.len() - 1) as f64;
    let whole = reach.floor() as usize;
    let last = whole.min(YEARS.len() - 1);
    let mut points: Vec<(f64, f64)> = (0..=last)
        .map(|index| (YEARS[index], COMMUTE[index]))
        .collect();
    let head = if whole + 1 < YEARS.len() {
        let fraction = reach - whole as f64;
        let head = (
            YEARS[whole] + fraction,
            COMMUTE[whole] + (COMMUTE[whole + 1] - COMMUTE[whole]) * fraction,
        );
        points.push(head);
        head
    } else {
        (YEARS[last], COMMUTE[last])
    };
    let stroke = StrokeStyle {
        width: 3.5,
        cap: LineCap::Round,
        join: LineJoin::Round,
    };
    let mut chart = LineChart::new(LINE_PLOT, horizontal, vertical)?.ticks(6)?;
    if draw > 0.0 {
        chart = chart.series(points, color(CORAL), stroke)?;
    }
    let mut nodes = vec![
        label("Average commute, minutes", 18.0, INK).translate(LINE_PLOT.left, 100.0),
        chart.node(context)?,
        Node::from(
            Rect::new(
                LINE_PLOT.left,
                LINE_PLOT.bottom,
                LINE_PLOT.right,
                LINE_PLOT.bottom + 1.5,
            )
            .fill(color(INK)),
        ),
    ];
    if draw > 0.0 {
        let point = Point::new(horizontal.map(head.0)?, vertical.map(head.1)?);
        let label_left = if point.x < LINE_PLOT.left + 60.0 {
            point.x + 12.0
        } else {
            point.x - 24.0
        };
        nodes.push(Circle::new(point, 6.0).fill(color(CORAL)).into());
        nodes.push(
            label(format!("{:.0} min", head.1), 14.0, 0xc54230)
                .translate(label_left, point.y - 26.0),
        );
    }
    Ok(Node::group(nodes).opacity(appear))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    #[ignore = "requires explicit MOTION_TEST_FONT and native GPU when enabled; label bounds and replay"]
    fn larger_labels_fit_and_frames_replay() -> motion::Result {
        let font = std::env::var_os("MOTION_TEST_FONT")
            .ok_or_else(|| std::io::Error::other("Set MOTION_TEST_FONT"))?;
        let mut fonts = FontLibrary::new();
        fonts.load_file(std::path::Path::new(&font))?;
        for value in ["8.4M", "+171%", "-19 min"] {
            let bounds = label(value, 28.0, INK)
                .weight(FontWeight::SEMIBOLD)
                .layout(&fonts)?
                .bounds();
            assert!(bounds.width() <= 104.0, "statistic too wide: {value}");
        }
        let callout = label("Record year: 8.4M", 15.0, PAPER).layout(&fonts)?;
        assert!(callout.bounds().width() <= 164.0);
        let options = crate::render_options(Some(std::path::Path::new(&font)))?
            .backend(BackendPreference::Cpu);
        let film = timeline(DemoParameters {
            seconds: 30,
            ..DemoParameters::default()
        })?;
        assert_eq!(film.frame_count(), 900);
        for size in [PixelSize::new(960, 540), PixelSize::new(1920, 1080)] {
            let options = options.clone().size(size);
            for seconds in [1.5, 15.0, 25.8, 29.0] {
                let first = render_still(&film, seconds.seconds(), &options)?;
                render_still(&film, Time::ZERO, &options)?;
                let replay = render_still(&film, seconds.seconds(), &options)?;
                assert_eq!(first.bytes(), replay.bytes());
                #[cfg(feature = "gpu")]
                {
                    let gpu_options = options.clone().backend(BackendPreference::gpu());
                    let gpu = render_still(&film, seconds.seconds(), &gpu_options)?;
                    let difference = first
                        .bytes()
                        .iter()
                        .zip(gpu.bytes())
                        .map(|(cpu, gpu)| cpu.abs_diff(*gpu))
                        .max()
                        .unwrap_or(0);
                    assert!(difference <= 2, "CPU/GPU difference: {difference}");
                    let replay = render_still(&film, seconds.seconds(), &gpu_options)?;
                    assert_eq!(gpu.bytes(), replay.bytes());
                }
            }
        }
        Ok(())
    }
}
