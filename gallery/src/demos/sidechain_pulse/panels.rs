//! Visual layers of Sidechain Pulse: the radial spectrum, ducking rings, bus meters, the
//! measured duck trace, the beat timeline strip and captions, all driven by the analysed mix.

use super::soundtrack::rms_db;
use super::{
    CENTER, CORAL, CURVE_FLOOR_DB, CURVE_STEP, CYAN, FEATURES, GOLD, INK, KICK_DEPTH_DB, MINT,
    NAVY, PAPER, PROGRAM_SECONDS, Rig, SPECTRUM_BANDS, VIOLET, VOICE_DEPTH_DB, color, meter_level,
    polar, text,
};
use motion::audio::pcm::BusMeter;
use motion::prelude::*;
use std::f64::consts::{FRAC_PI_2, TAU};

pub(super) fn backdrop(context: &SceneContext<'_>) -> Node {
    context.memo("sidechain-backdrop", || {
        context.bounds().fill(
            RadialGradient::new(CENTER, 560.0)
                .stop(0.0, color(NAVY))
                .stop(1.0, color(INK))
                .units(GradientUnits::UserSpace),
        )
    })
}

/// Radial equalizer: the analysis bands mirrored left/right, low bands at the top.
pub(super) fn spectrum(context: &SceneContext<'_>, rig: &Rig) -> Node {
    let count = SPECTRUM_BANDS * 2;
    let spin = context.seconds() * 0.05;
    let mut even = Path2D::new();
    let mut odd = Path2D::new();
    for bar in 0..count {
        let band = if bar < SPECTRUM_BANDS {
            bar
        } else {
            count - 1 - bar
        };
        let level = context.sample(&rig.spectrum[band]);
        let angle = -FRAC_PI_2 + (bar as f64 + 0.5) * TAU / count as f64 + spin;
        let length = 3.0 + math::pow(level, 1.0) * 92.0;
        let path = if bar % 2 == 0 { &mut even } else { &mut odd };
        path.move_to(polar(120.0, angle))
            .line_to(polar(120.0 + length, angle));
    }
    let gradient = LinearGradient::new(
        Point::new(CENTER.x, CENTER.y - 210.0),
        Point::new(CENTER.x, CENTER.y + 210.0),
    )
    .stop(0.0, color(CORAL))
    .stop(0.5, color(VIOLET))
    .stop(1.0, color(CYAN))
    .units(GradientUnits::UserSpace);
    Node::group([
        Node::from(even.stroke(4.0, gradient.clone())),
        Node::from(odd.stroke(4.0, gradient)).opacity(0.6),
    ])
}

/// Live rings shrink by the measured duck gain; faint references mark unity gain.
pub(super) fn rings(
    context: &SceneContext<'_>,
    bass_duck: f64,
    music_duck: f64,
    kick: f64,
) -> Node {
    let pump = (-bass_duck / KICK_DEPTH_DB).clamp(0.0, 1.0);
    let voice = (-music_duck / VOICE_DEPTH_DB).clamp(0.0, 1.0);
    let bass_radius = 38.0 + 42.0 * (1.0 - pump);
    let music_radius = 84.0 + 24.0 * (1.0 - voice);
    let orb = 22.0 + kick * 16.0;
    let spin = context.seconds() * 0.4;
    let mut ticks = Path2D::new();
    for tick in 0..48 {
        let angle = spin + tick as f64 * TAU / 48.0;
        ticks
            .move_to(polar(112.0, angle))
            .line_to(polar(if tick % 4 == 0 { 117.0 } else { 114.5 }, angle));
    }
    Node::group([
        Node::from(Circle::new(CENTER, 80.0).stroke(1.0, color(CYAN).with_alpha(0.22))),
        Node::from(Circle::new(CENTER, 108.0).stroke(1.0, color(GOLD).with_alpha(0.22))),
        Node::from(ticks.stroke(1.0, color(PAPER).with_alpha(0.35))),
        Node::from(Circle::new(CENTER, bass_radius).stroke(2.0 + 5.0 * pump, color(CYAN))),
        Node::from(Circle::new(CENTER, music_radius).stroke(2.0 + 4.0 * voice, color(GOLD)))
            .opacity(if music_duck < -0.01 || context.seconds() > 2.0 {
                1.0
            } else {
                0.0
            }),
        Node::from(
            Circle::new(CENTER, orb).fill(
                RadialGradient::new(CENTER, orb)
                    .stop(0.0, Color::WHITE)
                    .stop(0.45, color(CORAL))
                    .stop(1.0, color(CORAL).with_alpha(0.0))
                    .units(GradientUnits::UserSpace),
            ),
        )
        .blend_mode(BlendMode::Plus),
    ])
}

/// A white arc closes around the equalizer while the master chain is introduced.
pub(super) fn master_sweep(time: f64) -> Node {
    let progress = math::smoothstep(7.0, 7.8, time);
    if progress <= 0.0 {
        return Node::group(Vec::<Node>::new());
    }
    Node::from(
        Ring::new(CENTER, 214.0, 216.0)
            .sweep(-FRAC_PI_2, TAU * progress)
            .fill(color(MINT)),
    )
}

/// The measured duck gain of every sampled moment; the played part is drawn bright.
pub(super) fn duck_trace(rig: &Rig, time: f64) -> Node {
    let (left, right, top, bottom) = (36.0, 236.0, 376.0, 436.0);
    let point = |index: usize, db: f64| {
        Point::new(
            left + (right - left) * index as f64 * CURVE_STEP / PROGRAM_SECONDS,
            top + (bottom - top) * (db / CURVE_FLOOR_DB).clamp(0.0, 1.0),
        )
    };
    let split = ((time / CURVE_STEP) as usize + 1).min(rig.curve.len());
    let trace = |select: fn(&(f64, f64)) -> f64, hue: u32| {
        let mut played = Path2D::new();
        let mut upcoming = Path2D::new();
        for (index, sample) in rig.curve.iter().enumerate() {
            let position = point(index, select(sample));
            if index == 0 {
                played.move_to(position);
            } else if index < split {
                played.line_to(position);
            }
            if index + 1 == split {
                upcoming.move_to(position);
            } else if index >= split {
                upcoming.line_to(position);
            }
        }
        [
            Node::from(upcoming.stroke(1.0, color(hue).with_alpha(0.25))),
            Node::from(played.stroke(1.6, color(hue))),
        ]
    };
    let mut nodes = vec![
        text(
            "DUCK GAIN  0..-24 dB",
            9.0,
            color(PAPER).with_alpha(0.6),
            true,
        )
        .translate(left, top - 12.0),
        Rect::new(left, top, right, top + 0.5)
            .fill(color(PAPER).with_alpha(0.25))
            .into(),
        Rect::new(left, bottom, right, bottom + 0.5)
            .fill(color(PAPER).with_alpha(0.25))
            .into(),
    ];
    nodes.extend(trace(|sample| sample.1, GOLD));
    nodes.extend(trace(|sample| sample.0, CYAN));
    nodes.push(
        Rect::new(
            left + (right - left) * time / PROGRAM_SECONDS - 0.5,
            top,
            left + (right - left) * time / PROGRAM_SECONDS + 0.5,
            bottom,
        )
        .fill(Color::WHITE.with_alpha(0.7))
        .into(),
    );
    Node::group(nodes)
}

pub(super) fn bus_panel(rig: &Rig, time: f64, bass_duck: f64, music_duck: f64) -> Node {
    let rows: [(&str, &BusMeter, Option<&BusMeter>, u32); 5] = [
        ("KICK ", &rig.kick, None, CORAL),
        ("DRUMS", &rig.drums, None, GOLD),
        ("BASS ", &rig.bass, Some(&rig.bass_dry), CYAN),
        ("MUSIC", &rig.music, Some(&rig.music_dry), VIOLET),
        ("VOICE", &rig.voice, None, PAPER),
    ];
    let mut nodes: Vec<Node> =
        vec![text("BUS RMS", 10.0, color(PAPER).with_alpha(0.6), true).translate(36.0, 118.0)];
    for (index, (name, live, dry, hue)) in rows.into_iter().enumerate() {
        let top = 136.0 + index as f64 * 26.0;
        let level = meter_level(rms_db(live, time));
        nodes.push(text(name, 10.0, color(hue), true).translate(36.0, top + 9.0));
        nodes.push(
            Rect::new(100.0, top, 220.0, top + 8.0)
                .fill(color(PAPER).with_alpha(0.1))
                .into(),
        );
        nodes.push(
            Rect::new(100.0, top, 100.0 + 120.0 * level, top + 8.0)
                .fill(color(hue))
                .into(),
        );
        if let Some(dry) = dry {
            let reference = meter_level(rms_db(dry, time));
            nodes.push(
                Rect::new(
                    99.0 + 120.0 * reference,
                    top - 3.0,
                    101.0 + 120.0 * reference,
                    top + 11.0,
                )
                .fill(color(PAPER).with_alpha(0.8))
                .into(),
            );
        }
    }
    nodes.push(
        text(
            "| = un-ducked reference",
            8.5,
            color(PAPER).with_alpha(0.5),
            false,
        )
        .translate(36.0, 276.0),
    );
    nodes.push(
        text(
            &format!("BASS  {bass_duck:>6.1} dB"),
            11.0,
            color(CYAN),
            true,
        )
        .translate(36.0, 306.0),
    );
    nodes.push(
        text(
            &format!("MUSIC {music_duck:>6.1} dB"),
            11.0,
            color(GOLD),
            true,
        )
        .translate(36.0, 326.0),
    );
    Node::group(nodes)
}

/// Beat ticks and onset markers from the analysis, with the playhead and feature entries.
pub(super) fn timeline_strip(rig: &Rig, time: f64) -> Node {
    let (left, right, baseline) = (60.0, 900.0, 482.0);
    let x = |seconds: f64| left + (right - left) * seconds / PROGRAM_SECONDS;
    let mut beat_ticks = Path2D::new();
    let mut played = Path2D::new();
    for (index, beat) in rig.beats.iter().enumerate() {
        let height = if index % 4 == 0 { 16.0 } else { 9.0 };
        let path = if *beat <= time {
            &mut played
        } else {
            &mut beat_ticks
        };
        path.move_to(Point::new(x(*beat), baseline))
            .line_to(Point::new(x(*beat), baseline - height));
    }
    let mut nodes: Vec<Node> = vec![
        Rect::new(left, baseline, right, baseline + 1.0)
            .fill(color(PAPER).with_alpha(0.3))
            .into(),
        beat_ticks.stroke(1.5, color(PAPER).with_alpha(0.35)).into(),
        played.stroke(2.5, color(CORAL)).into(),
    ];
    for onset in &rig.onsets {
        let age = time - onset;
        let (radius, alpha) = if (0.0..0.25).contains(&age) {
            (3.0 + 3.0 * (1.0 - age / 0.25), 1.0)
        } else if age >= 0.25 {
            (2.2, 0.55)
        } else {
            (2.0, 0.18)
        };
        nodes.push(
            Circle::new(Point::new(x(*onset), baseline - 26.0), radius)
                .fill(color(GOLD).with_alpha(alpha))
                .into(),
        );
    }
    for (start, name, _, hue) in FEATURES {
        let tag = name.split(' ').next().unwrap_or(name);
        let reached = if time >= start { 1.0 } else { 0.35 };
        nodes.push(
            Rect::new(
                x(start) - 0.5,
                baseline + 6.0,
                x(start) + 0.5,
                baseline + 14.0,
            )
            .fill(color(hue).with_alpha(reached))
            .into(),
        );
        nodes.push(
            text(tag, 8.5, color(hue).with_alpha(reached), false)
                .translate(x(start) - 2.0, baseline + 26.0),
        );
    }
    nodes.push(
        Rect::new(
            x(time.min(PROGRAM_SECONDS)) - 1.0,
            baseline - 40.0,
            x(time.min(PROGRAM_SECONDS)) + 1.0,
            baseline + 8.0,
        )
        .fill(Color::WHITE)
        .into(),
    );
    Node::group(nodes)
}

pub(super) fn captions(rig: &Rig, time: f64) -> Vec<Node> {
    let mut nodes = vec![
        text("SIDECHAIN PULSE", 15.0, color(PAPER), true).translate(36.0, 44.0),
        text(
            "synthesized soundtrack / read back from the mix",
            9.5,
            color(PAPER).with_alpha(0.55),
            false,
        )
        .translate(36.0, 64.0),
    ];
    for (index, (start, name, detail, hue)) in FEATURES.into_iter().enumerate() {
        let age = time - start;
        if age < 0.0 {
            continue;
        }
        let enter = math::smoothstep(0.0, 0.35, age);
        let focus = 1.0 - math::smoothstep(1.2, 2.4, age);
        let top = 120.0 + index as f64 * 46.0;
        let slide = 18.0 * (1.0 - enter);
        let detail = if name == "MASTER" {
            format!(
                "limiter / {:.1} LUFS / TP {:.1} dB",
                rig.lufs, rig.true_peak_db
            )
        } else {
            detail.to_owned()
        };
        nodes.push(
            Node::group([
                Rect::new(0.0, 0.0, 3.0, 32.0).fill(color(hue)).into(),
                text(name, 12.0, color(hue), true).translate(12.0, 13.0),
                text(
                    &detail,
                    9.0,
                    color(PAPER).with_alpha(0.55 + 0.4 * focus),
                    false,
                )
                .translate(12.0, 28.0),
            ])
            .translate(716.0 + slide, top)
            .opacity(enter * (0.6 + 0.4 * focus)),
        );
    }
    nodes
}
