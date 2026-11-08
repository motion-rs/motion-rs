//! Sci-fi instrument study: audio-reactive radial spectrum, Plexus particle
//! constellations, a motion-blurred radar sweep and live HUD readouts, lit with
//! bloom, depth-of-field bokeh and a hairline grid.

use crate::{DemoMetadata, DemoParameters};
use motion::encode::{AudioAnalysis, AudioSignal, PcmBuffer};
use motion::particles::{Orbit, ParticleStyle, ParticleTrajectory};
use motion::prelude::*;
use std::{
    f64::consts::{FRAC_PI_2, TAU},
    sync::Arc,
};

const RATE: u32 = 48_000;

const CENTER: Point = Point::new(480.0, 270.0);

const VOID: u32 = 0x03060f;

const DEEP: u32 = 0x0a1530;

const CYAN: u32 = 0x3fd8ff;

const BLUE: u32 = 0x2f6bff;

const ICE: u32 = 0xd4f1ff;

const AMBER: u32 = 0xffb347;

const MONO: &str = "Menlo";

const BANDS: [(f64, f64); 5] = [
    (60.0, 180.0),
    (180.0, 360.0),
    (360.0, 720.0),
    (720.0, 1200.0),
    (1200.0, 2400.0),
];

pub(super) const METADATA: DemoMetadata = DemoMetadata::new(
    "signal-garden",
    "Signal Garden",
    "Sci-Fi",
    &[
        "particle-trajectories",
        "plexus-particles",
        "pcm-audio",
        "fft-analysis",
        "motion-blur",
        "filters",
    ],
);

struct Rig {
    bands: Vec<AudioSignal>,
    systems: Vec<ParticleTrajectory>,
}

pub(super) fn timeline(parameters: DemoParameters) -> motion::Result<Timeline> {
    let duration = i64::from(parameters.seconds).seconds();
    let notes = [220.0, 277.2, 329.6, 415.3, 440.0, 554.4];
    let samples = (0..parameters.seconds * RATE)
        .map(|index| {
            let time = f64::from(index) / f64::from(RATE);
            let beat = math::exp(-(time * 2.0).fract() * 9.0);
            let step = (time * 4.0).floor();
            let pluck = math::exp(-(time * 4.0).fract() * 7.0);
            let note = notes[(step as usize) % notes.len()];
            let bass = math::sin(TAU * 110.0 * time) * (0.1 + beat * 0.18);
            let lead = math::sin(TAU * note * time) * pluck * 0.07;
            let air = math::sin(TAU * 1320.0 * time) * (0.015 + 0.015 * math::sin(time * 1.3));
            let fade =
                (time * 12.0).min(1.0) * ((duration.as_seconds() - time) * 10.0).clamp(0.0, 1.0);
            ((bass + lead + air) * fade) as f32
        })
        .collect();
    let pcm = Arc::new(PcmBuffer::new(RATE, 1, samples)?);
    let analysis = AudioAnalysis::from_pcm(pcm.clone());
    let bands = BANDS
        .iter()
        .map(|(low, high)| analysis.band(*low, *high))
        .collect::<motion::Result<Vec<_>>>()?;
    let systems = (0..3)
        .map(|index| {
            ParticleTrajectory::new(
                Orbit::new(
                    CENTER,
                    150.0 + index as f64 * 52.0,
                    u64::from(parameters.count / 3 + 1),
                )
                .seed(parameters.seed + index)
                .angular_velocity([0.5, -0.36, 0.22][index as usize])
                .lifetime(duration),
            )
            .style(ParticleStyle::Plexus {
                radius: 2.0,
                max_distance: 62.0,
            })
        })
        .collect::<Vec<_>>();
    let rig = Arc::new(Rig { bands, systems });
    Ok(Timeline::new(Size::new(960.0, 540.0), 30)
        .audio(
            AudioClip::source(pcm)
                .duration(duration)
                .volume(0.65)
                .pan(-0.15),
        )
        .track(|track| track.scene(duration, move |context| frame(context, &rig))))
}

fn color(hex: u32) -> Color {
    Color::from_hex(hex)
}

fn frame(context: &SceneContext<'_>, rig: &Rig) -> Node {
    let levels: Vec<f64> = rig
        .bands
        .iter()
        .map(|band| (context.sample(band) * 3.2).clamp(0.0, 1.5))
        .collect();
    let bass = levels[0];
    let seconds = context.seconds();
    let intro = math::smoothstep(0.0, 0.12, context.progress());

    let mut layers = vec![
        backdrop(context),
        bokeh(context),
        grid(context),
        rings(context, bass),
        spectrum(&levels, seconds),
        radar(context),
    ];
    for (index, system) in rig.systems.iter().enumerate() {
        layers.push(
            system
                .render(context)
                .pivot(Pivot::Point(CENTER))
                .scale(1.0 + bass * (0.14 + index as f64 * 0.08))
                .opacity(intro),
        );
    }
    layers.push(core(context, bass));
    layers.push(foreground(context));
    layers.extend(hud(context, &levels, intro));
    Node::group(layers)
        .clip_with(context.bounds(), ClipMode::Geometric)
        .filter(Filter::Glow {
            threshold: 0.35,
            sigma: 8.0,
            intensity: 0.9,
        })
        .filter(Filter::Optics(motion::Optics::Lens {
            distortion: 0.0,
            chromatic: 0.0022,
            auto_fit: false,
            center: None,
        }))
        .filter(Filter::Optics(motion::Optics::LensVignette {
            angle_of_view: 75.0,
            strength: 0.55,
            roundness: 0.0,
        }))
        .filter(Filter::grain(7, 0.008))
}

fn backdrop(context: &SceneContext<'_>) -> Node {
    context.memo("signal-backdrop", || {
        context.bounds().fill(
            RadialGradient::new(CENTER, 520.0)
                .stop(0.0, color(DEEP))
                .stop(1.0, color(VOID))
                .units(GradientUnits::UserSpace),
        )
    })
}

/// Hairline square grid; split into chunks to stay inside the stroke verb limit.
fn grid(context: &SceneContext<'_>) -> Node {
    context.memo("signal-grid", || {
        let mut vertical = Path2D::new();
        for column in 0..=24 {
            let x = column as f64 * 40.0;
            vertical
                .move_to(Point::new(x, 0.0))
                .line_to(Point::new(x, 540.0));
        }
        let mut horizontal = Path2D::new();
        for row in 0..=13 {
            let y = row as f64 * 40.0;
            horizontal
                .move_to(Point::new(0.0, y))
                .line_to(Point::new(960.0, y));
        }
        let paint = color(CYAN).with_alpha(0.07);
        Node::group([
            Node::from(vertical.stroke(0.8, paint)),
            Node::from(horizontal.stroke(0.8, paint)),
        ])
    })
}

/// Far-field light discs, defocused as one blurred layer for cheap depth of field.
fn bokeh(context: &SceneContext<'_>) -> Node {
    let seconds = context.seconds();
    let discs = (0..16).map(|index| {
        let drift = context.random(("bokeh-speed", index)) * 14.0 + 6.0;
        let x = (context.random(("bokeh-x", index)) * 1100.0 + seconds * drift).rem_euclid(1100.0)
            - 70.0;
        let y = context.random(("bokeh-y", index)) * 540.0;
        let radius = 10.0 + context.random(("bokeh-r", index)) * 26.0;
        let hue = if index % 3 == 0 { BLUE } else { CYAN };
        Node::from(Circle::new(Point::new(x, y), radius).fill(color(hue).with_alpha(0.16)))
            .blend_mode(BlendMode::Plus)
    });
    Node::group(discs.collect::<Vec<_>>()).filter(Filter::Blur { sigma: 5.0 })
}

fn rings(context: &SceneContext<'_>, bass: f64) -> Node {
    let spin = context.seconds() * 0.12;
    let ticks = |radius: f64, count: usize, length: f64, offset: f64| {
        let mut path = Path2D::new();
        for tick in 0..count {
            let angle = offset + tick as f64 * TAU / count as f64;
            let long = if tick % 5 == 0 { length * 2.0 } else { length };
            path.move_to(Point::new(
                CENTER.x + math::cos(angle) * radius,
                CENTER.y + math::sin(angle) * radius,
            ))
            .line_to(Point::new(
                CENTER.x + math::cos(angle) * (radius + long),
                CENTER.y + math::sin(angle) * (radius + long),
            ));
        }
        path
    };
    Node::group([
        Node::from(Circle::new(CENTER, 222.0).stroke(0.8, color(CYAN).with_alpha(0.35))),
        Node::from(ticks(226.0, 90, 4.0, spin).stroke(1.0, color(CYAN).with_alpha(0.6))),
        Node::from(ticks(112.0, 60, 3.0, -spin * 2.0).stroke(1.0, color(ICE).with_alpha(0.55))),
        Node::from(
            Ring::new(CENTER, 126.0, 128.5 + bass * 5.0)
                .sweep(spin * 4.0, TAU * 0.62)
                .fill(color(CYAN)),
        ),
        Node::from(
            Ring::new(CENTER, 150.0, 151.0)
                .sweep(-spin * 3.0 + 1.2, TAU * 0.3)
                .fill(color(AMBER)),
        ),
    ])
}

/// One bar per angle; heights blend the five analysis bands around the circle.
fn spectrum(levels: &[f64], seconds: f64) -> Node {
    let count = 60;
    let mut path = Path2D::new();
    for bar in 0..count {
        let fraction = bar as f64 / count as f64;
        let angle = fraction * TAU - FRAC_PI_2;
        let position = (0.5 - 0.5 * math::cos(fraction * TAU * 2.0)) * (levels.len() - 1) as f64;
        let low = position.floor() as usize;
        let high = (low + 1).min(levels.len() - 1);
        let level = 0.12 + 0.9 * (levels[low] + (levels[high] - levels[low]) * position.fract());
        let wobble = 0.5 + 0.5 * math::sin(angle * 5.0 + seconds * 3.0);
        let length = 5.0 + math::pow(level, 0.7) * (56.0 + wobble * 34.0);
        path.move_to(Point::new(
            CENTER.x + math::cos(angle) * 134.0,
            CENTER.y + math::sin(angle) * 134.0,
        ))
        .line_to(Point::new(
            CENTER.x + math::cos(angle) * (134.0 + length),
            CENTER.y + math::sin(angle) * (134.0 + length),
        ));
    }
    Node::from(
        path.stroke(
            3.6,
            LinearGradient::new(Point::new(CENTER.x, 100.0), Point::new(CENTER.x, 440.0))
                .stop(0.0, color(ICE))
                .stop(0.5, color(CYAN))
                .stop(1.0, color(BLUE))
                .units(GradientUnits::UserSpace),
        ),
    )
}

/// A fading wedge that trails the sweep line; the shutter smears it into a comet.
fn radar(context: &SceneContext<'_>) -> Node {
    context.motion_blur(30.milliseconds(), 5, |sample| {
        let head = sample.seconds() * 2.4;
        let mut wedge = vec![];
        for slice in 0..8 {
            let alpha = 0.34 * (1.0 - slice as f64 / 8.0);
            wedge.push(Node::from(
                Ring::new(CENTER, 8.0, 218.0)
                    .sweep(head - (slice as f64 + 1.0) * 0.07, 0.07)
                    .fill(color(CYAN).with_alpha(alpha)),
            ));
        }
        wedge.push(Node::from(
            Ring::new(CENTER, 8.0, 222.0)
                .sweep(head - 0.012, 0.012)
                .fill(color(ICE)),
        ));
        Node::group(wedge).blend_mode(BlendMode::Plus)
    })
}

fn core(context: &SceneContext<'_>, bass: f64) -> Node {
    let breath = 0.5 + 0.5 * math::sin(context.seconds() * 2.0);
    let glow = Circle::new(CENTER, 46.0 + bass * 26.0).fill(
        RadialGradient::new(CENTER, 60.0 + bass * 26.0)
            .stop(0.0, Color::WHITE)
            .stop(0.35, color(CYAN))
            .stop(1.0, color(BLUE).with_alpha(0.0))
            .units(GradientUnits::UserSpace),
    );
    let halo = Circle::new(CENTER, 56.0 + breath * 4.0).stroke(1.2, color(ICE).with_alpha(0.8));
    Node::group([Node::from(glow), Node::from(halo)]).blend_mode(BlendMode::Plus)
}

/// Near-field discs drifting past in front, out of focus for parallax.
fn foreground(context: &SceneContext<'_>) -> Node {
    let seconds = context.seconds();
    let discs = (0..4).map(|index| {
        let x = (context.random(("near-x", index)) * 1200.0
            - seconds * (30.0 + index as f64 * 11.0))
            .rem_euclid(1200.0)
            - 120.0;
        let y = 60.0 + context.random(("near-y", index)) * 420.0;
        Node::from(
            Circle::new(Point::new(x, y), 34.0 + index as f64 * 10.0)
                .fill(color(ICE).with_alpha(0.07)),
        )
        .blend_mode(BlendMode::Plus)
    });
    Node::group(discs.collect::<Vec<_>>()).filter(Filter::Blur { sigma: 14.0 })
}

fn readout(value: &str, size: f64, fill: Color) -> Text {
    Text::new(value.to_owned())
        .family(MONO)
        .size(size)
        .weight(FontWeight::BOLD)
        .letter_spacing(1.5)
        .fill(fill)
}

fn bracket(left: f64, top: f64, dx: f64, dy: f64) -> Node {
    let mut path = Path2D::new();
    path.move_to(Point::new(left, top + dy * 18.0))
        .line_to(Point::new(left, top))
        .line_to(Point::new(left + dx * 18.0, top));
    path.stroke(1.4, color(CYAN).with_alpha(0.85)).into()
}

fn hud(context: &SceneContext<'_>, levels: &[f64], intro: f64) -> Vec<Node> {
    let seconds = context.seconds();
    let dim = color(CYAN).with_alpha(0.75);
    let mut nodes = vec![
        bracket(28.0, 28.0, 1.0, 1.0),
        bracket(932.0, 28.0, -1.0, 1.0),
        bracket(28.0, 512.0, 1.0, -1.0),
        bracket(932.0, 512.0, -1.0, -1.0),
        readout("SIGNAL GARDEN", 15.0, color(ICE)).translate(48.0, 40.0),
        readout("LIVE SPECTRUM // CH-01", 10.5, dim).translate(48.0, 62.0),
        readout(&format!("T+{seconds:05.2}s"), 12.0, color(ICE)).translate(790.0, 40.0),
        readout("MODE  RADIAL", 10.5, dim).translate(790.0, 62.0),
    ];
    for (index, (low, high)) in BANDS.iter().enumerate() {
        let top = 392.0 + index as f64 * 22.0;
        let level = levels[index].clamp(0.0, 1.0);
        nodes.push(readout(&format!("{low:>4.0}-{high:<4.0}"), 10.0, dim).translate(48.0, top));
        nodes.push(
            Rect::new(138.0, top + 2.0, 258.0, top + 10.0)
                .fill(color(CYAN).with_alpha(0.14))
                .into(),
        );
        nodes.push(
            Rect::new(138.0, top + 2.0, 138.0 + 120.0 * level, top + 10.0)
                .fill(if level > 0.85 {
                    color(AMBER)
                } else {
                    color(CYAN)
                })
                .into(),
        );
    }
    nodes.push(
        readout(&format!("LEVEL {:.2}", levels[0]), 12.0, color(ICE)).translate(790.0, 490.0),
    );
    nodes.into_iter().map(|node| node.opacity(intro)).collect()
}
