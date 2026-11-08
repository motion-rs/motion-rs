//! Last Order: a rainy night seen from inside a neon diner. Seven one-second beats
//! (the first lasts two) introduce the cinematic toolkit in four acts. The effects
//! accumulate, so the final beat shows them working together:
//!
//! 1. Exposure: over-exposed lights, clean clip against an AgX shoulder.
//! 2. Light: bloom, halation, anamorphic streaks, then rack focus with bokeh.
//! 3. Space: elevation shadows, then fog, dust and a light leak.
//! 4. Film: a CDL + colour-wheel grade baked into a LUT, then `Look::film`.
//!
//! The gallery renders without a display transform, so the tone map is baked into a
//! `Lut3d` with `DisplayTransform::apply` and applied as an ordinary filter.

use super::{frame_seed, label};
use crate::{DemoMetadata, DemoParameters};
use diner::{street_plane, world, world_with_street};
use grade::{grade_strength, look_lut};
use motion::atmosphere::Atmosphere;
use motion::prelude::*;
use motion::render::RasterLimits;

mod diner;
mod grade;

const WIDTH: f64 = 960.0;

const HEIGHT: f64 = 540.0;

const HORIZON: f64 = 312.0;

const MONO: &str = "Menlo";

const MAGENTA: u32 = 0xff3fa4;

const CYAN: u32 = 0x4fe6ff;

const AMBER: u32 = 0xffb25c;

const FILM_SECONDS: f64 = 8.0;

/// Stop range of the log-encoded look lattice; 49 nodes give a third of a stop each.
const LOG_MIN: f32 = -10.0;

const LOG_MAX: f32 = 5.0;

const LUT_EDGE: u16 = 49;

/// Steps used to cache the grade strength baked into the LUT.
const GRADE_STEPS: f64 = 16.0;

struct Beat {
    act: &'static str,
    start: f64,
    title: &'static str,
    detail: &'static str,
    accent: u32,
}

const BEATS: [Beat; 7] = [
    Beat {
        act: "I  EXPOSURE",
        start: 0.0,
        title: "TONE MAP",
        detail: "clean clip  |  ToneMap::AgX shoulder",
        accent: AMBER,
    },
    Beat {
        act: "II  LIGHT",
        start: 2.0,
        title: "BLOOM + HALATION + STREAKS",
        detail: "Filter::Bloom, halation, Optics::Anamorphic",
        accent: CYAN,
    },
    Beat {
        act: "II  LIGHT",
        start: 3.0,
        title: "RACK FOCUS + BOKEH",
        detail: "DepthOfField radius -> Optics::Bokeh",
        accent: MAGENTA,
    },
    Beat {
        act: "III  SPACE",
        start: 4.0,
        title: "ELEVATION",
        detail: "Elevation::shadow contact + key + ambient",
        accent: AMBER,
    },
    Beat {
        act: "III  SPACE",
        start: 5.0,
        title: "FOG + DUST + LIGHT LEAK",
        detail: "Atmosphere::fog / dust / light_leak",
        accent: CYAN,
    },
    Beat {
        act: "IV  FILM",
        start: 6.0,
        title: "GRADE: CDL + WHEELS -> LUT",
        detail: "Cdl, ColorWheels, Lut3d with Filter::Lut",
        accent: MAGENTA,
    },
    Beat {
        act: "IV  FILM",
        start: 7.0,
        title: "LOOK::FILM",
        detail: "shutter, grain and gate weave via Look::scene",
        accent: AMBER,
    },
];

pub(super) const METADATA: DemoMetadata = DemoMetadata::new(
    "cinematic-look",
    "Last Order",
    "Cinematic",
    &[
        "tone-map",
        "bloom-halation-anamorphic",
        "depth-of-field-bokeh",
        "elevation-shadows",
        "atmosphere",
        "cdl-lut-grade",
        "look-film",
    ],
);

pub(super) fn timeline(parameters: DemoParameters) -> motion::Result<Timeline> {
    let seed = parameters.seed;
    Ok(
        Timeline::new(Size::new(WIDTH, HEIGHT), 30).track(move |track| {
            track.scene(parameters.seconds.seconds(), move |context| {
                film(context, seed)
            })
        }),
    )
}

fn color(hex: u32) -> Color {
    Color::from_hex(hex)
}

fn mix(from: f64, to: f64, amount: f64) -> f64 {
    from + (to - from) * amount
}

/// Film-relative time, so any requested duration plays the same beats.
fn beat_time(context: &SceneContext<'_>) -> f64 {
    context.progress() * FILM_SECONDS
}

fn exposure(nodes: Vec<Node>, stops: f64) -> Node {
    Node::group(nodes).filter(Filter::ColorAdjust {
        exposure: stops,
        contrast: 1.0,
        saturation: 1.0,
        hue_degrees: 0.0,
    })
}

/// A soft emissive disc: a hot core that falls off into its own colour.
fn bulb(x: f64, y: f64, radius: f64, hue: u32) -> Node {
    let center = Point::new(x, y);
    Circle::new(center, radius)
        .fill(
            RadialGradient::new(center, radius)
                .stop(0.0, Color::WHITE.interpolate(&color(hue), 0.2))
                .stop(0.5, color(hue))
                .stop(1.0, color(hue).with_alpha(0.0))
                .units(GradientUnits::UserSpace),
        )
        .into()
}

/// A streak of light mirrored in the wet road.
fn sheen(x: f64, y: f64, width: f64, height: f64, hue: u32, alpha: f64) -> Node {
    let center = Point::new(x, y);
    Circle::new(center, height)
        .fill(
            RadialGradient::new(center, height)
                .stop(0.0, color(hue).with_alpha(alpha))
                .stop(1.0, color(hue).with_alpha(0.0))
                .units(GradientUnits::UserSpace),
        )
        .scale_xy(width / height, 1.0)
        .blend_mode(BlendMode::Plus)
}

fn film(context: &SceneContext<'_>, seed: u64) -> motion::Result<Node> {
    let t = beat_time(context);
    let frame = frame_seed(context);
    let output = PixelSize::new(WIDTH as u32, HEIGHT as u32);
    let size = PixelSize::new(WIDTH as u32 / 2, HEIGHT as u32 / 2);
    let limits = RasterLimits::default();

    let ramp = |from: f64, to: f64| math::smoothstep(from, to, t);
    let fog_amount = ramp(5.0, 5.5) * (1.0 - 0.45 * ramp(6.0, 7.0));
    let leak_amount = ramp(5.2, 5.8) * (1.0 - 0.5 * ramp(6.3, 7.0));
    let dust_amount = ramp(5.0, 5.4);
    let atmosphere = Atmospheres {
        fog: haze(
            Atmosphere::fog(7),
            fog_amount,
            size,
            context,
            limits,
            |fog| {
                fog.color = color(0x7fa9c8);
                fog.opacity = 0.32 * fog_amount;
                fog.scale = 150.0;
                fog.drift = [14.0, -3.0];
            },
        )?,
        dust: haze(
            Atmosphere::dust(11),
            dust_amount,
            output,
            context,
            limits,
            |dust| {
                dust.opacity = 0.7 * dust_amount;
                dust.intensity = 4.0;
                dust.scale = 36.0;
            },
        )?,
        leak: haze(
            Atmosphere::light_leak(3),
            leak_amount,
            size,
            context,
            limits,
            |leak| {
                leak.kind = motion::atmosphere::AtmosphereKind::LightLeak {
                    center: [0.0, 0.42],
                    extent: [0.34, 0.6],
                };
                leak.opacity = 0.7 * leak_amount;
                leak.intensity = 1.8;
            },
        )?,
    };

    let build = |inner: &SceneContext<'_>| world(inner, seed, &atmosphere);
    let mut picture = if t >= 7.0 {
        let mut look = Look::film(seed);
        look.gate_weave = Some(GateWeave { seed, amount: 0.8 });
        // Eight fixed samples keep the repeated street inside the scene node budget.
        look.sampling.adaptive = false;
        // Four bloom octaves fit the default 256 MiB raster budget at 960x540.
        for filter in &mut look.filters {
            if let Filter::Bloom { levels, .. } = filter {
                *levels = 4;
            }
        }
        // Only the street still moves in this act (the passing car and the flickering sign),
        // so only it is exposed over the shutter; the static planes, their bokeh and the
        // look's filters are evaluated once instead of once per sample.
        let street = match look.shutter.take() {
            Some(blur) => look.sampling.capture(blur, context, output, street_plane)?,
            None => street_plane(context),
        };
        let scene = world_with_street(context, seed, &atmosphere, street);
        // Clipping to the frame stops the look's halos from growing every later stage.
        look.scene(context, output, |_| scene.clone())?
            .clip_with(context.bounds(), ClipMode::Geometric)
    } else {
        build(context)
    };

    // Highlight chain: each stage joins the film at its own beat and then stays.
    let bloom = ramp(2.0, 2.4);
    if bloom > 0.0 {
        picture = picture.filter(Filter::Bloom {
            threshold: 0.85,
            knee: 0.3,
            radius: 5.0,
            levels: 4,
            falloff: 0.55,
            intensity: 0.7 * bloom,
            tint: Color::WHITE,
        });
    }
    let halation = ramp(2.4, 2.8);
    if halation > 0.0 {
        picture = picture.filter(Filter::halation(0.9 * halation, 12.0)?);
    }
    let streaks = ramp(2.7, 3.0);
    if streaks > 0.0 {
        picture = picture.filter(Filter::Optics(Optics::Anamorphic {
            threshold: 1.4,
            length: 56.0,
            stretch: 24.0,
            tint: color(0x8fc4ff),
            intensity: 1.1 * streaks,
        }));
    }
    // `Look::film` brings its own lens and vignette, so the layers are not stacked twice.
    if t < 7.0 {
        picture = picture.filter(Filter::Optics(Optics::LensVignette {
            angle_of_view: 70.0,
            strength: 0.45,
            roundness: 0.4,
        }));
    }

    let grade = grade_strength(t);
    let lut = look_lut(context, grade)?;
    let display = |node: Node| {
        node.filter(Filter::Lut {
            lut: lut.clone(),
            input: LutInput::Log {
                minimum: LOG_MIN,
                maximum: LOG_MAX,
            },
        })
    };

    let mut result = if t < 2.0 {
        let divider = split_position(t);
        let mut parts = vec![
            display(picture.clone()),
            picture.clip_with(Rect::new(0.0, 0.0, divider, HEIGHT), ClipMode::Geometric),
            Rect::new(divider - 1.0, 0.0, divider + 1.0, HEIGHT)
                .fill(Color::WHITE.with_alpha(0.85))
                .into(),
        ];
        parts.extend(split_labels(divider, t));
        Node::group(parts)
    } else {
        display(picture)
    };
    if t >= 7.0 {
        // The gallery has no display stage, so the film grain is an ordinary filter.
        result = result.filter(Filter::grain(seed ^ frame, 0.028));
    }
    Ok(Node::group([
        result.clip_with(context.bounds(), ClipMode::Geometric),
        caption(t),
    ]))
}

struct Atmospheres {
    fog: Node,
    dust: Node,
    leak: Node,
}

/// Renders one atmosphere overlay at half resolution, or nothing while it is off.
fn haze(
    mut recipe: Atmosphere,
    amount: f64,
    size: PixelSize,
    context: &SceneContext<'_>,
    limits: RasterLimits,
    configure: impl FnOnce(&mut Atmosphere),
) -> motion::Result<Node> {
    if amount <= 0.0 {
        return Ok(Node::empty());
    }
    configure(&mut recipe);
    Ok(recipe
        .overlay(size, context.time(), limits)?
        .pivot(Pivot::Point(Point::ORIGIN))
        .scale(WIDTH / f64::from(size.width)))
}

/// Divider path: hold near the middle for the comparison, then wipe off the left edge.
fn split_position(t: f64) -> f64 {
    let hold = mix(720.0, 680.0, math::smoothstep(0.0, 1.0, t));
    mix(hold, -4.0, math::smoothstep(1.0, 1.95, t))
}

fn split_labels(divider: f64, t: f64) -> Vec<Node> {
    let appear = math::smoothstep(0.1, 0.4, t) * (1.0 - math::smoothstep(1.5, 1.8, t));
    vec![
        label(
            "CLEAN: CLIPPED",
            MONO,
            12.0,
            3.0,
            FontWeight::BOLD,
            Color::WHITE,
        )
        .translate(divider - 150.0, 504.0)
        .opacity(appear),
        label(
            "AGX: SHOULDER",
            MONO,
            12.0,
            3.0,
            FontWeight::BOLD,
            color(AMBER),
        )
        .translate(divider + 14.0, 504.0)
        .opacity(appear),
    ]
}

/// A compact caption naming the beat on screen, with a tick per beat.
fn caption(t: f64) -> Node {
    let index = BEATS
        .iter()
        .rposition(|beat| t >= beat.start)
        .unwrap_or_default();
    let beat = &BEATS[index];
    let visible = math::smoothstep(beat.start, beat.start + 0.2, t);
    let accent = color(beat.accent);
    let mut nodes = vec![
        Node::from(
            RoundedRect::new(Rect::new(54.0, 44.0, 426.0, 128.0), 8.0)
                .fill(color(0x04050c).with_alpha(0.78)),
        ),
        Rect::new(54.0, 44.0, 58.0, 128.0).fill(accent).into(),
        label(beat.act, MONO, 10.0, 3.0, FontWeight::BOLD, accent).translate(72.0, 56.0),
        label(beat.title, MONO, 14.0, 1.5, FontWeight::BOLD, Color::WHITE).translate(72.0, 72.0),
        label(
            beat.detail,
            MONO,
            10.0,
            0.5,
            FontWeight::NORMAL,
            Color::WHITE.with_alpha(0.7),
        )
        .translate(72.0, 98.0),
    ];
    for tick in 0..BEATS.len() {
        let left = 72.0 + tick as f64 * 18.0;
        nodes.push(
            Rect::new(left, 119.0, left + 14.0, 121.0)
                .fill(if tick <= index {
                    accent
                } else {
                    Color::WHITE.with_alpha(0.25)
                })
                .into(),
        );
    }
    Node::group(nodes).opacity(visible)
}

#[cfg(test)]
mod tests {
    use super::grade::bake_look;
    use super::*;

    #[test]
    fn tone_map_lattice_keeps_hue_in_over_exposed_light_and_black_stays_black() -> motion::Result {
        let lut = bake_look(0.0)?;
        let input = LutInput::Log {
            minimum: LOG_MIN,
            maximum: LOG_MAX,
        };
        let orange = lut.apply([6.0, 3.6, 1.4], input);
        assert!(
            orange[0] <= 1.001 && orange[0] >= orange[1] && orange[1] > orange[2],
            "{orange:?}"
        );
        assert!(lut.apply([0.0; 3], input).iter().all(|value| *value < 0.01));
        let graded = bake_look(1.0)?.apply([0.5; 3], input);
        assert!(graded[0] > graded[2], "highlights lean warm");
        Ok(())
    }

    #[test]
    fn every_beat_renders_and_replays_exactly() -> motion::Result {
        let font = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../tests/fixtures/fonts/NotoSans-Regular.ttf");
        let options = crate::render_options(Some(&font))?.size(PixelSize::new(192, 108));
        let timeline = timeline(DemoParameters::default())?;
        let mut previous = None;
        for seconds in [1.0, 2.5, 3.5, 4.5, 5.5, 6.5, 7.5] {
            let time = Time::from_seconds(seconds);
            let frame = render_still(&timeline, time, &options)?;
            assert_eq!(
                frame.bytes(),
                render_still(&timeline, time, &options)?.bytes(),
                "replay at {seconds}"
            );
            assert_ne!(Some(frame.bytes().to_vec()), previous, "beat at {seconds}");
            previous = Some(frame.bytes().to_vec());
        }
        Ok(())
    }
}
