//! Sci-fi instrument study wired to a live mix: a synthesized kick, hat, bass and
//! lead program is routed through audio buses with sidechain ducking, delay and
//! reverb, panning and stereo width and a limiting master, then read back offline.
//! FFT bands drive a radial spectrum and Plexus particle constellations, the
//! measured duck gain makes the whole garden pump with the kick, and a
//! motion-blurred radar sweep is lit with bloom and bokeh beneath crisp HUD meters.

use crate::{DemoMetadata, DemoParameters};
use motion::audio::pcm::BusMeter;
use motion::audio::{AudioAnalysis, AudioSignal, MixOptions, MixedAudio};
use motion::particles::{Orbit, ParticleStyle, ParticleTrajectory};
use motion::prelude::*;
use soundtrack::{audio_timeline, duck_db, meter, rms_db, soundtrack};
use std::{
    f64::consts::{FRAC_PI_2, TAU},
    sync::Arc,
};

mod soundtrack;

const RATE: u32 = 48_000;

const BPM: f64 = 120.0;

/// Pentatonic pluck cycle, one note per eighth.
const NOTES: [f64; 6] = [220.0, 277.18, 329.63, 415.3, 440.0, 554.37];

/// Gain reduction the kick applies to the bass bus.
const KICK_DEPTH_DB: f64 = 14.0;

const CENTER: Point = Point::new(480.0, 270.0);

const VOID: u32 = 0x03060f;

const DEEP: u32 = 0x0a1530;

const CYAN: u32 = 0x3fd8ff;

const BLUE: u32 = 0x2f6bff;

const ICE: u32 = 0xd4f1ff;

const AMBER: u32 = 0xffb347;

const MONO: &str = "Menlo";

const BANDS: [(f64, f64); 5] = [
    (40.0, 120.0),
    (120.0, 360.0),
    (360.0, 720.0),
    (720.0, 1400.0),
    (1400.0, 4000.0),
];

const BUSES: [&str; 4] = ["kick", "hats", "bass", "lead"];

pub(super) const METADATA: DemoMetadata = DemoMetadata::new(
    "signal-garden",
    "Signal Garden",
    "Sci-Fi",
    &[
        "synthesis",
        "audio-buses",
        "sidechain-ducking",
        "audio-effects",
        "pan-and-width",
        "master-limiter",
        "fft-analysis",
        "plexus-particles",
        "motion-blur",
        "filters",
    ],
);

struct Rig {
    bands: Vec<AudioSignal>,
    systems: Vec<ParticleTrajectory>,
    meters: Vec<BusMeter>,
    bass: BusMeter,
    bass_dry: BusMeter,
    lufs: f64,
}

impl Rig {
    /// Sidechain depth in 0..1: how far the kick currently pushes the bass bus down.
    fn pump(&self, time: f64) -> f64 {
        (-duck_db(&self.bass, &self.bass_dry, time) / KICK_DEPTH_DB).clamp(0.0, 1.0)
    }
}

pub(super) fn timeline(parameters: DemoParameters) -> Result<Timeline, motion::ErrorReport> {
    let duration = i64::from(parameters.seconds).seconds();
    let seed = parameters.seed as u32;
    let output = audio_timeline(&soundtrack(duration, true, seed)?, duration).build()?;
    let mix = MixedAudio::new(&output, MixOptions::new(RATE))?;
    let dry = MixedAudio::new(
        &audio_timeline(&soundtrack(duration, false, seed)?, duration).build()?,
        MixOptions::new(RATE),
    )?;
    let analysis = AudioAnalysis::from_source(&mix, duration)?;
    let bands = BANDS
        .iter()
        .map(|(low, high)| {
            analysis
                .band(*low, *high)?
                .normalize(0.95)?
                .attack_release(5.milliseconds(), 160.milliseconds())
        })
        .collect::<motion::Result<Vec<_>>>()?;
    let meters = BUSES
        .iter()
        .map(|name| meter(&mix, name))
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
    let rig = Arc::new(Rig {
        bands,
        systems,
        bass: meter(&mix, "bass")?,
        bass_dry: meter(&dry, "bass")?,
        meters,
        lufs: mix.loudness_report()?.integrated_lufs.unwrap_or(-70.0),
    });
    output
        .to_builder()
        .track(|track| track.scene(duration, move |context| frame(context, &rig)))
        .build()
}

fn color(hex: u32) -> Color {
    Color::from_hex(hex)
}

/// Maps a dBFS level to 0..1 over a 60 dB window.
fn meter_level(db: f64) -> f64 {
    ((db + 60.0) / 60.0).clamp(0.0, 1.0)
}

fn frame(context: &SceneContext<'_>, rig: &Rig) -> motion::Result<Node> {
    let levels: Vec<f64> = rig
        .bands
        .iter()
        .map(|band| context.sample(band) * 1.15)
        .collect();
    let bass = levels[0];
    let seconds = context.seconds();
    let pump = rig.pump(seconds);
    let intro = math::smoothstep(0.0, 0.12, context.progress());

    let mut layers = vec![
        backdrop(context)?,
        bokeh(context)?,
        grid(context)?,
        shockwave(seconds, rig),
        rings(context, bass, pump)?,
        spectrum(&levels, seconds, pump)?,
        radar(context),
    ];
    // The constellations inhale while the sidechain holds the bass down.
    for (index, system) in rig.systems.iter().enumerate() {
        layers.push(
            system
                .render(context)
                .pivot(Pivot::Point(CENTER))
                .scale(1.0 + bass * (0.08 + index as f64 * 0.05) - pump * 0.06)
                .opacity(intro),
        );
    }
    layers.push(core(context, bass, pump)?);
    layers.push(foreground(context)?);
    let instrument = Node::group(layers)
        .clip_with(context.bounds(), ClipMode::Geometric)
        .filter(Filter::Glow {
            threshold: 0.35,
            sigma: 8.0,
            intensity: 0.8 + pump * 0.4,
        })
        .filter(Filter::Optics(motion::Optics::Lens {
            distortion: 0.0,
            chromatic: 0.0006 + pump * 0.0006,
            auto_fit: false,
            center: None,
        }))
        .filter(Filter::Optics(motion::Optics::LensVignette {
            angle_of_view: 75.0,
            strength: 0.55,
            roundness: 0.0,
        }))
        .filter(Filter::grain(7, 0.002));
    Ok(Node::group([
        instrument,
        Node::group(hud(context, rig, &levels, pump, intro)?),
    ]))
}

fn backdrop(context: &SceneContext<'_>) -> motion::Result<Node> {
    Ok(context.memo("signal-backdrop", || {
        context.bounds().fill(
            RadialGradient::new(CENTER, 520.0)
                .stop(0.0, color(DEEP))
                .stop(1.0, color(VOID))
                .units(GradientUnits::UserSpace),
        )
    }))
}

/// Hairline square grid; split into chunks to stay inside the stroke verb limit.
fn grid(context: &SceneContext<'_>) -> motion::Result<Node> {
    let grid = context.try_cache("signal-grid", || {
        let mut vertical = Path2D::new();
        for column in 0..=24 {
            let x = column as f64 * 40.0;
            vertical.try_move_to(Point::new(x, 0.0))?;
            vertical.try_line_to(Point::new(x, 540.0))?;
        }
        let mut horizontal = Path2D::new();
        for row in 0..=13 {
            let y = row as f64 * 40.0;
            horizontal.try_move_to(Point::new(0.0, y))?;
            horizontal.try_line_to(Point::new(960.0, y))?;
        }
        let paint = color(CYAN).with_alpha(0.07);
        Ok::<_, motion::Error>(Node::group([
            Node::from(vertical.stroke(0.8, paint)),
            Node::from(horizontal.stroke(0.8, paint)),
        ]))
    })?;
    Ok(grid.as_ref().clone())
}

/// Far-field light discs, defocused as one blurred layer for cheap depth of field.
fn bokeh(context: &SceneContext<'_>) -> motion::Result<Node> {
    let seconds = context.seconds();
    let discs = (0..16).map(|index| -> motion::Result<Node> {
        let drift = context.random(("bokeh-speed", index)) * 14.0 + 6.0;
        let x = (context.random(("bokeh-x", index)) * 1100.0 + seconds * drift).rem_euclid(1100.0)
            - 70.0;
        let y = context.random(("bokeh-y", index)) * 540.0;
        let radius = 10.0 + context.random(("bokeh-r", index)) * 26.0;
        let hue = if index % 3 == 0 { BLUE } else { CYAN };
        Ok(
            Node::from(Circle::new(Point::new(x, y), radius)?.fill(color(hue).with_alpha(0.16)))
                .blend_mode(BlendMode::Plus),
        )
    });
    Ok(Node::group(discs.collect::<motion::Result<Vec<_>>>()?).filter(Filter::Blur { sigma: 5.0 }))
}

fn rings(context: &SceneContext<'_>, bass: f64, pump: f64) -> motion::Result<Node> {
    let spin = context.seconds() * 0.12;
    let ticks = |radius: f64, count: usize, length: f64, offset: f64| -> motion::Result<Path2D> {
        let mut path = Path2D::new();
        for tick in 0..count {
            let angle = offset + tick as f64 * TAU / count as f64;
            let long = if tick % 5 == 0 { length * 2.0 } else { length };
            path.try_move_to(Point::new(
                CENTER.x + math::cos(angle) * radius,
                CENTER.y + math::sin(angle) * radius,
            ))?;
            path.try_line_to(Point::new(
                CENTER.x + math::cos(angle) * (radius + long),
                CENTER.y + math::sin(angle) * (radius + long),
            ))?;
        }
        Ok(path)
    };
    Ok(Node::group([
        Node::from(
            Circle::new(CENTER, 222.0)?
                .stroke(0.8 + pump * 1.6, color(CYAN).with_alpha(0.35 + pump * 0.4)),
        ),
        Node::from(ticks(226.0, 90, 4.0, spin)?.stroke(1.0, color(CYAN).with_alpha(0.6))),
        Node::from(ticks(112.0, 60, 3.0, -spin * 2.0)?.stroke(1.0, color(ICE).with_alpha(0.55))),
        Node::from(
            Ring::new(CENTER, 126.0, 128.5 + bass * 5.0)
                .sweep(spin * 4.0, TAU * 0.62)
                .fill(color(CYAN)),
        ),
        Node::from(
            Ring::new(CENTER, 150.0, 151.0 + pump * 3.0)
                .sweep(-spin * 3.0 + 1.2, TAU * (0.3 + pump * 0.12))
                .fill(color(AMBER)),
        ),
    ]))
}

/// Expanding ring launched on every kick, its strength read from the kick bus meter.
fn shockwave(seconds: f64, rig: &Rig) -> Node {
    let period = 60.0 / BPM;
    let age = seconds.rem_euclid(period) / period;
    let hit = meter_level(rms_db(&rig.meters[0], seconds - age * period + 0.03) + 12.0);
    let radius = 70.0 + math::smoothstep(0.0, 1.0, math::sqrt(age)) * 190.0;
    let alpha = (1.0 - age) * (1.0 - age) * 0.7 * hit;
    Node::from(
        Ring::new(CENTER, radius, radius + 2.0 + 10.0 * (1.0 - age))
            .fill(color(CYAN).with_alpha(alpha)),
    )
    .blend_mode(BlendMode::Plus)
}

/// One bar per angle; heights blend the five analysis bands around the circle and
/// the base radius kicks outward with the sidechain.
fn spectrum(levels: &[f64], seconds: f64, pump: f64) -> motion::Result<Node> {
    let base = 134.0 + pump * 8.0;
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
        path.try_move_to(Point::new(
            CENTER.x + math::cos(angle) * base,
            CENTER.y + math::sin(angle) * base,
        ))?;
        path.try_line_to(Point::new(
            CENTER.x + math::cos(angle) * (base + length),
            CENTER.y + math::sin(angle) * (base + length),
        ))?;
    }
    Ok(Node::from(
        path.stroke(
            3.6,
            LinearGradient::new(Point::new(CENTER.x, 100.0), Point::new(CENTER.x, 440.0))
                .stop(0.0, color(ICE))
                .stop(0.5, color(CYAN))
                .stop(1.0, color(BLUE))
                .units(GradientUnits::UserSpace),
        ),
    ))
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

fn core(context: &SceneContext<'_>, bass: f64, pump: f64) -> motion::Result<Node> {
    let breath = 0.5 + 0.5 * math::sin(context.seconds() * 2.0);
    let swell = bass * 14.0 + pump * 18.0;
    let glow = Circle::new(CENTER, 46.0 + swell)?.fill(
        RadialGradient::new(CENTER, 60.0 + swell)
            .stop(0.0, Color::WHITE)
            .stop(0.35, color(CYAN))
            .stop(1.0, color(BLUE).with_alpha(0.0))
            .units(GradientUnits::UserSpace),
    );
    let halo = Circle::new(CENTER, 56.0 + breath * 4.0)?.stroke(1.2, color(ICE).with_alpha(0.8));
    Ok(Node::group([Node::from(glow), Node::from(halo)]).blend_mode(BlendMode::Plus))
}

/// Near-field discs drifting past in front, out of focus for parallax.
fn foreground(context: &SceneContext<'_>) -> motion::Result<Node> {
    let seconds = context.seconds();
    let discs = (0..4).map(|index| -> motion::Result<Node> {
        let x = (context.random(("near-x", index)) * 1200.0
            - seconds * (30.0 + index as f64 * 11.0))
            .rem_euclid(1200.0)
            - 120.0;
        let y = 60.0 + context.random(("near-y", index)) * 420.0;
        Ok(Node::from(
            Circle::new(Point::new(x, y), 34.0 + index as f64 * 10.0)?
                .fill(color(ICE).with_alpha(0.07)),
        )
        .blend_mode(BlendMode::Plus))
    });
    Ok(
        Node::group(discs.collect::<motion::Result<Vec<_>>>()?)
            .filter(Filter::Blur { sigma: 14.0 }),
    )
}

fn readout(value: &str, size: f64, fill: Color) -> Text {
    Text::new(value.to_owned())
        .family(MONO)
        .size(size)
        .weight(FontWeight::BOLD)
        .letter_spacing(0.0)
        .fill(fill)
}

fn bracket(left: f64, top: f64, dx: f64, dy: f64) -> motion::Result<Node> {
    let path = Path2D::builder()
        .move_to(Point::new(left, top + dy * 18.0))
        .line_to(Point::new(left, top))
        .line_to(Point::new(left + dx * 18.0, top))
        .build()?;
    Ok(path.stroke(1.4, color(CYAN).with_alpha(0.85)).into())
}

/// Gain-reduction history of the bass bus over the last two seconds.
fn duck_trace(rig: &Rig, seconds: f64) -> motion::Result<Node> {
    let (left, top, width, height) = (720.0, 410.0, 212.0, 46.0);
    let mut path = Path2D::new();
    for step in 0..=60 {
        let fraction = f64::from(step) / 60.0;
        let time = seconds - 2.0 * (1.0 - fraction);
        let depth = if time < 0.0 { 0.0 } else { rig.pump(time) };
        let point = Point::new(left + fraction * width, top + 3.0 + depth * (height - 6.0));
        if step == 0 {
            path.try_move_to(point)?;
        } else {
            path.try_line_to(point)?;
        }
    }
    Ok(Node::group([
        Node::from(
            Rect::new(left, top, left + width, top + height).fill(color(CYAN).with_alpha(0.06)),
        ),
        Node::from(path.stroke(1.4, color(AMBER))),
    ]))
}

fn hud(
    context: &SceneContext<'_>,
    rig: &Rig,
    levels: &[f64],
    pump: f64,
    intro: f64,
) -> motion::Result<Vec<Node>> {
    let seconds = context.seconds();
    let dim = color(CYAN).with_alpha(0.95);
    let mut nodes = vec![
        bracket(28.0, 28.0, 1.0, 1.0)?,
        bracket(932.0, 28.0, -1.0, 1.0)?,
        bracket(28.0, 512.0, 1.0, -1.0)?,
        bracket(932.0, 512.0, -1.0, -1.0)?,
        readout("SIGNAL GARDEN", 20.0, color(ICE)).translate(48.0, 40.0),
        readout("LIVE MIX // 4 BUS + SIDECHAIN", 14.0, dim).translate(48.0, 70.0),
        readout(&format!("T+{seconds:05.2}s"), 16.0, color(ICE)).translate(768.0, 40.0),
        readout(&format!("BPM {BPM:.0}  RADIAL"), 14.0, dim).translate(768.0, 68.0),
    ];
    for (index, name) in BUSES.iter().enumerate() {
        let top = 98.0 + index as f64 * 24.0;
        let level = meter_level(rms_db(&rig.meters[index], seconds) + 6.0);
        nodes.push(readout(&name.to_uppercase(), 14.0, dim).translate(768.0, top));
        nodes.push(
            Rect::new(824.0, top + 4.0, 932.0, top + 14.0)
                .fill(color(CYAN).with_alpha(0.14))
                .into(),
        );
        nodes.push(
            Rect::new(824.0, top + 4.0, 824.0 + 108.0 * level, top + 14.0)
                .fill(color(CYAN))
                .into(),
        );
    }
    for (index, (low, high)) in BANDS.iter().enumerate() {
        let top = 392.0 + index as f64 * 22.0;
        let level = levels[index].clamp(0.0, 1.0);
        nodes.push(readout(&format!("{low:>4.0}-{high:<4.0}"), 14.0, dim).translate(48.0, top));
        nodes.push(
            Rect::new(138.0, top + 4.0, 258.0, top + 14.0)
                .fill(color(CYAN).with_alpha(0.14))
                .into(),
        );
        nodes.push(
            Rect::new(138.0, top + 4.0, 138.0 + 120.0 * level, top + 14.0)
                .fill(if level > 0.85 {
                    color(AMBER)
                } else {
                    color(CYAN)
                })
                .into(),
        );
    }
    nodes.push(readout("SIDECHAIN // KICK > BASS", 14.0, dim).translate(720.0, 384.0));
    nodes.push(duck_trace(rig, seconds)?);
    nodes.push(
        readout(
            &format!("GR {:+05.1} dB", duck_db(&rig.bass, &rig.bass_dry, seconds)),
            16.0,
            if pump > 0.5 { color(AMBER) } else { color(ICE) },
        )
        .translate(720.0, 466.0),
    );
    nodes.push(readout(&format!("{:.1} LUFS", rig.lufs), 14.0, dim).translate(720.0, 490.0));
    Ok(nodes.into_iter().map(|node| node.opacity(intro)).collect())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    #[ignore = "requires explicit MOTION_TEST_FONT and native GPU when enabled; HUD bounds and replay"]
    fn readable_hud_fits_and_frames_replay() -> motion::Result {
        let font = std::env::var_os("MOTION_TEST_FONT")
            .ok_or_else(|| std::io::Error::other("Set MOTION_TEST_FONT"))?;
        let mut fonts = FontLibrary::new();
        fonts.load_file(std::path::Path::new(&font))?;
        for value in ["KICK", "HATS", "BASS", "LEAD"] {
            assert!(
                readout(value, 14.0, color(ICE))
                    .layout(&fonts)?
                    .bounds()
                    .width()
                    <= 56.0
            );
        }
        for (low, high) in BANDS {
            let value = format!("{low:>4.0}-{high:<4.0}");
            assert!(
                readout(&value, 14.0, color(ICE))
                    .layout(&fonts)?
                    .bounds()
                    .width()
                    <= 90.0
            );
        }
        assert!(
            readout("SIDECHAIN // KICK > BASS", 14.0, color(ICE))
                .layout(&fonts)?
                .bounds()
                .width()
                <= 212.0
        );
        let film = timeline(DemoParameters::default())?;
        assert_eq!(film.frame_count(), 240);
        let options = crate::render_options(Some(std::path::Path::new(&font)))?
            .backend(BackendPreference::Cpu);
        assert_eq!(options.limits.max_bytes, crate::GALLERY_FRAME_BYTES);
        for size in [PixelSize::new(960, 540), PixelSize::new(1920, 1080)] {
            let options = options.clone().size(size);
            for seconds in [0.0, 1.375, 3.0, 7.8] {
                let first = render_still(&film, seconds.seconds().after_origin(), &options)?;
                render_still(&film, Time::ZERO, &options)?;
                let replay = render_still(&film, seconds.seconds().after_origin(), &options)?;
                assert_eq!(first.bytes(), replay.bytes());
                #[cfg(feature = "gpu")]
                {
                    let gpu_options = options.clone().backend(BackendPreference::gpu());
                    let gpu = render_still(&film, seconds.seconds().after_origin(), &gpu_options)?;
                    let difference = first
                        .bytes()
                        .iter()
                        .zip(gpu.bytes())
                        .map(|(cpu, gpu)| cpu.abs_diff(*gpu))
                        .max()
                        .unwrap_or(0);
                    assert!(difference <= 2, "CPU/GPU difference: {difference}");
                    render_still(&film, Time::ZERO, &gpu_options)?;
                    let replay =
                        render_still(&film, seconds.seconds().after_origin(), &gpu_options)?;
                    assert_eq!(gpu.bytes(), replay.bytes());
                }
            }
        }
        Ok(())
    }
}
