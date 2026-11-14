//! Animation atlas comparing easing, continuous signals and composed motion.

use super::atlas::{self, Page};
use crate::{DemoMetadata, DemoParameters};
use motion::prelude::*;
use std::sync::Arc;

pub(super) const METADATA: DemoMetadata = DemoMetadata::new(
    "animation-atlas",
    "Animation Atlas",
    "Animation",
    &[
        "tween",
        "easing",
        "keyframes",
        "spring",
        "oscillator",
        "seeded-noise",
        "follow-path",
        "time-composition",
        "color-interpolation",
        "transform-animation",
    ],
);

const ACCENTS: [u32; 4] = [atlas::GOLD, atlas::MINT, atlas::BLUE, 0xf08c79];

pub(super) const PAGES: &[Page] = &[
    easing_page("Linear", "A constant rate gives equal steps."),
    easing_page("Sine In", "A gentle trigonometric start."),
    easing_page("Sine Out", "A gentle trigonometric finish."),
    easing_page("Sine In Out", "A smooth start and finish."),
    easing_page("Quadratic In", "A squared progress accelerates."),
    easing_page("Quadratic Out", "A quadratic curve slows down."),
    easing_page("Quadratic In Out", "Two quadratic halves meet."),
    easing_page("Cubic In", "A cubic curve accelerates."),
    easing_page("Cubic Out", "A cubic curve decelerates."),
    easing_page("Cubic In Out", "Cubic acceleration and braking."),
    easing_page("Quartic In", "A stronger power-curve start."),
    easing_page("Quartic Out", "A stronger power-curve finish."),
    easing_page("Quartic In Out", "Quartic motion on both halves."),
    easing_page("Exponential In", "An exponential acceleration."),
    easing_page("Exponential Out", "An exponential deceleration."),
    easing_page("Exponential In Out", "Exponential motion at each end."),
    easing_page("Back In", "Anticipation moves below zero."),
    easing_page("Back Out", "Overshoot moves beyond one."),
    easing_page("Back In Out", "Anticipation and overshoot."),
    easing_page("Elastic Out", "An elastic oscillation settles."),
    easing_page("Bounce Out", "A piecewise bounce comes to rest."),
    easing_page("Steps", "Six discrete steps replace a ramp."),
    easing_page("Cubic Bezier", "Explicit control points set pace."),
    easing_page("Custom", "A supplied smoothstep function."),
    page(
        "Underdamped spring",
        [
            "Low damping retains",
            "the real spring overshoot",
            "before approaching its target.",
        ],
    ),
    page(
        "Critical spring",
        [
            "Critical damping approaches",
            "the target without",
            "an oscillating overshoot.",
        ],
    ),
    page(
        "Overdamped spring",
        [
            "Stronger damping returns",
            "more slowly to the same",
            "spring target.",
        ],
    ),
    page(
        "Sine oscillator",
        [
            "A continuous sine signal",
            "oscillates at 0.85 Hz",
            "around a signed zero.",
        ],
    ),
    page(
        "Triangle oscillator",
        [
            "A continuous triangle signal",
            "uses straight rising and",
            "falling segments.",
        ],
    ),
    page(
        "Square oscillator",
        [
            "A continuous square signal",
            "alternates between",
            "two signed levels.",
        ],
    ),
    page(
        "Sawtooth oscillator",
        [
            "A continuous sawtooth signal",
            "ramps and resets",
            "at each cycle.",
        ],
    ),
    page(
        "Seeded noise",
        [
            "Three octaves of keyed noise",
            "depend only on the supplied",
            "seed and explicit sample time.",
        ],
    ),
    page(
        "FollowPath",
        [
            "A real path sample supplies",
            "position and tangent angle",
            "for the moving arrow.",
        ],
    ),
    page(
        "Color and transforms",
        [
            "Six staggered specimens tween",
            "color, scale, rotation",
            "and opacity together.",
        ],
    ),
    page(
        "Delay",
        [
            "The original tween waits",
            "0.55 seconds before",
            "its local motion begins.",
        ],
    ),
    page(
        "Reverse",
        [
            "The tween samples its",
            "finite duration backward",
            "from the original end.",
        ],
    ),
    page(
        "Repeat",
        [
            "A double-speed tween",
            "restarts four times",
            "on its explicit local clock.",
        ],
    ),
    page(
        "Ping pong",
        [
            "A finite tween alternates",
            "forward and backward",
            "without sampling history.",
        ],
    ),
    page(
        "Then",
        [
            "One tween finishes before",
            "a second bounce tween",
            "continues from its endpoint.",
        ],
    ),
    page(
        "Speed x2",
        [
            "A doubled clock completes",
            "the same finite tween",
            "in half the time.",
        ],
    ),
    page(
        "Map",
        [
            "The tween's sampled value",
            "is squared by a real",
            "animation mapping function.",
        ],
    ),
    page(
        "Zip + map",
        [
            "Two clocks sample together;",
            "their paired values are",
            "mapped to an average.",
        ],
    ),
    page(
        "Keyframes + hold",
        [
            "Timed keys and an explicit",
            "hold preserve a plateau",
            "before the next transition.",
        ],
    ),
];

const fn easing_page(title: &'static str, description: &'static str) -> Page {
    page(
        title,
        [
            description,
            "A real eased tween moves",
            "forward, then backward.",
        ],
    )
}

const fn page(title: &'static str, lines: [&'static str; 3]) -> Page {
    Page { title, lines }
}

pub(super) fn timeline(parameters: DemoParameters) -> Result<Timeline, motion::ErrorReport> {
    let curves = easing_curves()?;
    let signals = signal_studies(parameters.seed)?;
    let compositions = composition_studies();
    let path = flight_path()?;
    let flight = FollowPath::new(path.clone(), 2.5.seconds())
        .easing(Easing::SineInOut)
        .delay(0.2.seconds());
    atlas::timeline(parameters, METADATA, PAGES, move |page, progress, _| {
        Ok(match page {
            0..=23 => {
                let movement = Tween::new(0.0, 1.0, 1.25.seconds())
                    .easing(curves[page].1.clone())
                    .ping_pong()
                    .delay(0.2.seconds());
                animation_plot(&movement, progress, false)?
            }
            24..=31 => {
                let study = &signals[page - 24];
                animation_plot(study.as_ref(), progress, page >= 27)?
            }
            32 => path_board(&path, &flight, progress)?,
            33 => transform_board(progress)?,
            _ => {
                let study = &compositions[page - 34];
                animation_plot(study.as_ref(), progress, false)?
            }
        })
    })
}

fn easing_curves() -> motion::Result<Vec<(&'static str, Easing)>> {
    Ok(vec![
        ("Linear", Easing::Linear),
        ("Sine In", Easing::SineIn),
        ("Sine Out", Easing::SineOut),
        ("Sine In Out", Easing::SineInOut),
        ("Quadratic In", Easing::QuadraticIn),
        ("Quadratic Out", Easing::QuadraticOut),
        ("Quadratic In Out", Easing::QuadraticInOut),
        ("Cubic In", Easing::CubicIn),
        ("Cubic Out", Easing::CubicOut),
        ("Cubic In Out", Easing::CubicInOut),
        ("Quartic In", Easing::QuarticIn),
        ("Quartic Out", Easing::QuarticOut),
        ("Quartic In Out", Easing::QuarticInOut),
        ("Exponential In", Easing::ExponentialIn),
        ("Exponential Out", Easing::ExponentialOut),
        ("Exponential In Out", Easing::ExponentialInOut),
        ("Back In", Easing::back_in(1.7)?),
        ("Back Out", Easing::back_out(1.7)?),
        ("Back In Out", Easing::back_in_out(1.7)?),
        ("Elastic Out", Easing::elastic_out(1.0, 0.3)?),
        ("Bounce Out", Easing::BounceOut),
        ("Steps", Easing::steps(6)?),
        ("Cubic Bezier", Easing::cubic_bezier(0.76, 0.0, 0.24, 1.0)?),
        (
            "Custom",
            Easing::Custom(Arc::new(|progress| {
                progress * progress * (3.0 - 2.0 * progress)
            })),
        ),
    ])
}

fn signal_studies(seed: u64) -> motion::Result<Vec<Arc<dyn Signal<f64>>>> {
    let mut studies = Vec::new();
    for damping in [9.0, 22.0, 38.0] {
        let spring = Spring::builder(0.0, 1.0)
            .stiffness(121.0)
            .damping(damping)
            .build()?;
        studies.push((move |time: Time| spring.sample(time - 0.2.seconds())).boxed());
    }
    for waveform in [
        Waveform::Sine,
        Waveform::Triangle,
        Waveform::Square,
        Waveform::Sawtooth,
    ] {
        studies.push(Oscillator::try_new(waveform, 0.85)?.boxed());
    }
    studies.push(
        Noise::builder(seed)
            .frequency(1.3)
            .octaves(3)
            .build()?
            .boxed(),
    );
    Ok(studies)
}

fn animation_plot(
    animation: &dyn Signal<f64>,
    progress: f64,
    signed: bool,
) -> motion::Result<Node> {
    let time = Time::from_seconds(progress * 3.2);
    let (lower, upper) = if signed { (-1.2, 1.2) } else { (-0.3, 1.4) };
    let vertical = |value: f64| 190.0 - (value - lower) / (upper - lower) * 142.0;
    let mut trace = Path2D::new();
    for index in 0..=160 {
        let fraction = index as f64 / 160.0;
        let value = animation.sample(Time::from_seconds(fraction * 3.2));
        let position = Point::new(56.0 + 400.0 * fraction, vertical(value));
        if index == 0 {
            trace.try_move_to(position)?;
        } else {
            trace.try_line_to(position)?;
        }
    }
    let value = animation.sample(time);
    let position = if signed {
        256.0 + value * 160.0
    } else {
        80.0 + value * 300.0
    };
    Ok(Node::group([
        Rect::new(56.0, vertical(0.0), 456.0, vertical(0.0) + 0.8)
            .fill(Color::from_hex(0x648377))
            .into(),
        trace.stroke(2.3, Color::from_hex(atlas::MINT)).into(),
        Rect::new(
            55.5 + progress * 400.0,
            38.0,
            56.5 + progress * 400.0,
            202.0,
        )
        .fill(Color::from_hex(0x648377))
        .into(),
        Circle::new(Point::new(56.0 + progress * 400.0, vertical(value)), 5.0)?
            .fill(Color::from_hex(atlas::GOLD))
            .into(),
        Rect::new(64.0, 234.0, 464.0, 235.0)
            .fill(Color::from_hex(0x648377))
            .into(),
        Circle::new(Point::new(position, 234.0), 10.0)?
            .fill(Color::from_hex(atlas::BLUE))
            .into(),
        atlas::text("0", 12.0, 0xb9cec5, 28.0, vertical(0.0) - 6.0),
        atlas::text(&format!("value {value:.3}"), 14.0, 0xb9cec5, 40.0, 271.0),
        atlas::text(
            &format!("t {:.2} s", time.as_seconds()),
            14.0,
            0xb9cec5,
            363.0,
            271.0,
        ),
    ]))
}

fn composition_studies() -> Vec<Arc<dyn Signal<f64>>> {
    let movement = Tween::new(0.0, 1.0, 1.2.seconds()).easing(Easing::CubicInOut);
    vec![
        movement.clone().delay(0.55.seconds()).boxed(),
        movement.clone().reverse().boxed(),
        movement.clone().speed(2.0).repeat(4).boxed(),
        movement.clone().ping_pong().boxed(),
        movement
            .clone()
            .then(Tween::new(1.0, 0.35, 1.seconds()).easing(Easing::BounceOut))
            .boxed(),
        movement.clone().speed(2.0).boxed(),
        movement.clone().map(|value| value * value).boxed(),
        movement
            .zip(Tween::new(0.0, 1.0, 2.4.seconds()))
            .map(|(first, second)| (first + second) * 0.5)
            .boxed(),
        Keyframes::new(0.0)
            .to(0.4.seconds(), 0.75, Easing::QuadraticOut)
            .hold(0.5.seconds())
            .to(1.5.seconds(), 0.25, Easing::CubicInOut)
            .to(2.5.seconds(), 1.0, Easing::BounceOut)
            .boxed(),
    ]
}

fn flight_path() -> Result<Path2D, ValidationError> {
    Path2D::builder()
        .move_to(Point::new(70.0, 285.0))
        .cubic_to(
            Point::new(80.0, 150.0),
            Point::new(250.0, 160.0),
            Point::new(245.0, 258.0),
        )
        .cubic_to(
            Point::new(235.0, 370.0),
            Point::new(400.0, 365.0),
            Point::new(425.0, 210.0),
        )
        .build()
}

fn path_board(
    path: &Path2D,
    flight: &impl Signal<PathSample>,
    progress: f64,
) -> motion::Result<Node> {
    let time = Time::from_seconds(progress * 3.2);
    let sample = flight.sample(time);
    let arrow = Path2D::builder()
        .move_to(Point::new(14.0, 0.0))
        .line_to(Point::new(-10.0, -8.0))
        .line_to(Point::new(-5.0, 0.0))
        .line_to(Point::new(-10.0, 8.0))
        .close()
        .build()?
        .fill(Color::from_hex(ACCENTS[1]))
        .pivot(Pivot::Point(Point::ORIGIN))
        .rotate(sample.angle)
        .translate(sample.position.x, sample.position.y);
    Ok(Node::group([
        Node::group((path.clone().stroke(2.5, Color::from_hex(0x648377)), arrow))
            .translate(0.0, -110.0),
        atlas::text("position + tangent", 14.0, 0xb9cec5, 28.0, 272.0),
    ]))
}

fn transform_board(progress: f64) -> motion::Result<Node> {
    let time = Time::from_seconds(progress * 3.2);
    let mut nodes = Vec::new();
    let color = Tween::new(
        Color::from_hex(ACCENTS[0]),
        Color::from_hex(ACCENTS[2]),
        1.1.seconds(),
    )
    .easing(Easing::SineInOut)
    .ping_pong();
    let transform = Tween::new(0.0, 1.0, 1.1.seconds())
        .easing(Easing::back_out(1.4)?)
        .ping_pong();
    for index in 0..6 {
        let local = time - (0.15 + index as f64 * 0.12).seconds();
        let amount = transform.sample(local);
        nodes.push(
            Rect::new(-24.0, -24.0, 24.0, 24.0)
                .fill(color.sample(local))
                .pivot(Pivot::Point(Point::ORIGIN))
                .scale(0.5 + amount * 0.6)
                .rotate(amount * std::f64::consts::PI)
                .translate(
                    112.0 + (index % 3) as f64 * 144.0,
                    94.0 + (index / 3) as f64 * 130.0 - amount * 13.0,
                )
                .opacity((0.3 + amount * 0.7).clamp(0.0, 1.0)),
        );
    }
    Ok(Node::group(nodes))
}

#[cfg(test)]
mod tests {
    use super::*;

    const PAGE_COUNT: usize = PAGES.len();

    #[test]
    #[ignore = "requires explicit MOTION_TEST_FONT; CPU stills only, no window or encoder"]
    fn each_chapter_animates_and_replays_exactly() -> motion::Result {
        let font = std::env::var_os("MOTION_TEST_FONT")
            .ok_or_else(|| std::io::Error::other("Set MOTION_TEST_FONT"))?;
        let options = crate::render_options(Some(std::path::Path::new(&font)))?
            .backend(BackendPreference::Cpu);
        let timeline = timeline(DemoParameters::default())?;
        for size in [PixelSize::new(288, 162), PixelSize::new(1920, 1080)] {
            let options = options.clone().size(size);
            let width = size.width as usize;
            let height = size.height as usize;
            for page in 0..PAGE_COUNT {
                let seconds = DemoParameters::default().seconds as f64;
                let start = (page as f64 + 0.18) / PAGE_COUNT as f64 * seconds;
                let end = (page as f64 + 0.62) / PAGE_COUNT as f64 * seconds;
                let first = render_still(&timeline, start.seconds().after_origin(), &options)?;
                let later = render_still(&timeline, end.seconds().after_origin(), &options)?;
                let replay = render_still(&timeline, start.seconds().after_origin(), &options)?;
                assert_eq!(first.bytes().len(), width * height * 4);
                assert_eq!(first.bytes(), replay.bytes());
                let changed = (height * 152 / 540..height * 456 / 540)
                    .flat_map(|row| {
                        (width * 388 / 960..width * 900 / 960)
                            .map(move |column| (row * width + column) * 4)
                    })
                    .filter(|offset| {
                        first.bytes()[*offset..*offset + 4] != later.bytes()[*offset..*offset + 4]
                    })
                    .count();
                assert!(
                    changed > 30,
                    "page {page} at {start}s did not animate at {size:?}"
                );
            }
        }
        Ok(())
    }
}
