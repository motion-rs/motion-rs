//! Four particle acts in eight seconds: bouncing fireworks, a boid murmuration, dissolving
//! text/image/path emitters, and bokeh dust with embers under noise-driven atmosphere.

use crate::{DemoMetadata, DemoParameters};
use motion::atmosphere::Atmosphere;
use motion::particles::ParticleSimulation;
use motion::prelude::*;
use motion::render::RasterLimits;
use simulations::{dissolve, embers, fireworks, murmuration, slabs};
use std::sync::{Arc, OnceLock};

mod behaviors;
mod simulations;

const WIDTH: f64 = 960.0;

const HEIGHT: f64 = 540.0;

const MONO: &str = "Menlo";

const SANS: &str = "Helvetica Neue";

const FLOOR: f64 = 506.0;

pub(super) const METADATA: DemoMetadata = DemoMetadata::new(
    "particle-garden",
    "Particle Garden",
    "Particles",
    &[
        "sub-emitters",
        "colliders",
        "boids",
        "flow-field",
        "ribbon-trails",
        "paint-aware-emitters",
        "sprite-atlas",
        "bokeh",
        "temperature",
        "atmosphere",
    ],
);

/// Act windows on the nominal eight-second timeline; `(start, end, caption)`.
const ACTS: [(f64, f64, &str, &str); 4] = [
    (
        0.0,
        2.6,
        "I  SPARKS",
        "sub-emitters / polygon + segment colliders",
    ),
    (
        2.2,
        4.6,
        "II  MURMURATION",
        "boids / curl flow field / ribbon trails",
    ),
    (
        4.2,
        6.5,
        "III  DISSOLVE",
        "text + image + path emitters / padded atlas",
    ),
    (
        6.1,
        8.0,
        "IV  EMBERS",
        "bokeh / blackbody curve / noise atmosphere",
    ),
];

fn color(hex: u32) -> Color {
    Color::from_hex(hex)
}

fn seconds(value: f64) -> Duration {
    Duration::from_seconds(value)
}

struct Rig {
    fireworks: ParticleSimulation,
    flock: ParticleSimulation,
    dissolve: OnceLock<ParticleSimulation>,
    dust: ParticleSimulation,
    embers: ParticleSimulation,
    seed: u64,
    scale: f64,
    look: Look,
}

pub(super) fn timeline(parameters: DemoParameters) -> Result<Timeline, motion::ErrorReport> {
    let scale = f64::from(parameters.seconds) / 8.0;
    let window = |index: usize| seconds((ACTS[index].1 - ACTS[index].0) * scale);
    let (dust, ember_sim) = embers(parameters.seed, window(3))?;
    let mut look = Look::cinematic();
    look.effects.shutter = None;
    // Bloom and vignette carry the cinematic feel; grade, halation and dispersion cost more than they add.
    look.effects.filters.retain(|filter| {
        matches!(
            filter,
            Filter::Bloom { .. } | Filter::Optics(Optics::LensVignette { .. })
        )
    });
    let rig = Arc::new(Rig {
        fireworks: fireworks(parameters.seed, window(0))?,
        flock: murmuration(parameters.seed, window(1))?,
        dissolve: OnceLock::new(),
        dust,
        embers: ember_sim,
        seed: parameters.seed,
        scale,
        look,
    });
    let duration = i64::from(parameters.seconds).seconds();
    Timeline::new(Size::new(WIDTH, HEIGHT), motion::fps!(30))
        .track(|track| track.scene(duration, move |context| frame(context, &rig)))
        .build()
}

fn opacity(seconds: f64, (start, end): (f64, f64), last: bool) -> f64 {
    let fade_in = math::smoothstep(start, start + 0.35, seconds);
    let fade_out = if last {
        1.0
    } else {
        1.0 - math::smoothstep(end - 0.5, end, seconds)
    };
    fade_in * fade_out
}

fn frame(context: &SceneContext<'_>, rig: &Rig) -> motion::Result<Node> {
    if rig.dissolve.get().is_none() {
        let window = seconds((ACTS[2].1 - ACTS[2].0) * rig.scale);
        let _ = rig.dissolve.set(dissolve(rig.seed, window, context)?);
    }
    let dissolve = rig.dissolve.get().expect("initialised above");
    rig.look.scene(
        context,
        PixelSize::new(960, 540),
        scene(|context| compose(context, rig, dissolve)),
    )
}

fn compose(
    context: &SceneContext<'_>,
    rig: &Rig,
    dissolve: &ParticleSimulation,
) -> motion::Result<Node> {
    let now = context.seconds() / rig.scale;
    let act = |index: usize, simulation: &ParticleSimulation| {
        let (start, end, ..) = ACTS[index];
        let weight = opacity(now, (start, end), index == 3);
        context
            .during(
                seconds(start * rig.scale).after_origin(),
                seconds((end - start) * rig.scale),
                |inner| simulation.render(inner),
            )
            .opacity(weight)
    };
    let mut layers = vec![
        backdrop(now),
        stage_geometry(opacity(now, (ACTS[0].0, ACTS[0].1), false))?,
        act(0, &rig.fireworks),
        act(1, &rig.flock),
        act(2, dissolve),
        act(3, &rig.dust),
        act(3, &rig.embers),
    ];
    layers.push(atmosphere(context, now));
    layers.push(captions(now));
    Ok(Node::group(layers))
}

fn backdrop(now: f64) -> Node {
    let warmth = math::smoothstep(ACTS[3].0 - 0.2, ACTS[3].0 + 1.0, now);
    let base = LinearGradient::new(Point::ORIGIN, Point::new(0.0, 1.0))
        .stop(0.0, color(0x04060f))
        .stop(1.0, color(0x0b1226));
    let glow = LinearGradient::new(Point::ORIGIN, Point::new(0.0, 1.0))
        .stop(0.0, color(0x1a0a0c).with_alpha(0.0))
        .stop(1.0, color(0x401a10));
    let bounds = Rect::new(0.0, 0.0, WIDTH, HEIGHT);
    Node::group([
        Node::from(bounds.fill(base)),
        Node::from(bounds.fill(glow)).opacity(warmth),
    ])
}

fn stage_geometry(weight: f64) -> motion::Result<Node> {
    let edge = color(0x86a8ff).with_alpha(0.55);
    let floor = Path2D::polyline([Point::new(0.0, FLOOR), Point::new(WIDTH, FLOOR)])?;
    let mut nodes = vec![Node::from(floor.stroke(1.2, edge))];
    for mut slab in slabs() {
        slab.push(slab[0]);
        let outline = Path2D::polyline(slab)?;
        nodes.push(Node::from(
            outline.clone().fill(color(0x86a8ff).with_alpha(0.08)),
        ));
        nodes.push(Node::from(outline.stroke(1.2, edge)));
    }
    Ok(Node::group(nodes).opacity(weight))
}

/// Fog and dust advect through shared noise; a half-resolution image is enough for soft haze.
fn atmosphere(context: &SceneContext<'_>, now: f64) -> Node {
    let weight = math::smoothstep(ACTS[3].0 + 0.6, 8.0, now);
    if weight <= 0.0 {
        return Node::empty();
    }
    let size = PixelSize::new(480, 270);
    let time = context.time();
    let mut fog = Atmosphere::fog(17);
    fog.color = color(0xff9a62);
    fog.opacity = 0.5;
    fog.scale = 70.0;
    fog.drift = [8.0, -5.0];
    let mut nodes = Vec::new();
    for (index, recipe) in [fog, Atmosphere::light_leak(9)].into_iter().enumerate() {
        if let Ok(overlay) = recipe.overlay(size, time, RasterLimits::default()) {
            nodes.push(
                overlay
                    .pivot(Pivot::Point(Point::ORIGIN))
                    .scale(2.0)
                    .opacity(weight * if index == 0 { 0.75 } else { 0.35 }),
            );
        }
    }
    Node::group(nodes)
}

fn captions(now: f64) -> Node {
    let mut nodes = Vec::new();
    for (index, (start, end, title, detail)) in ACTS.iter().enumerate() {
        let weight = math::smoothstep(start + 0.3, start + 0.6, now)
            * if index == 3 {
                1.0
            } else {
                1.0 - math::smoothstep(end - 0.8, end - 0.55, now)
            };
        if weight <= 0.0 {
            continue;
        }
        let ink = color(0xe9f1ff);
        let title = super::label(title, MONO, 15.0, 3.0, FontWeight::BOLD, ink);
        let detail = super::label(
            detail,
            MONO,
            11.0,
            1.2,
            FontWeight::NORMAL,
            ink.with_alpha(0.6),
        );
        nodes.push(
            Node::group([
                Node::from(title).translate(40.0, 34.0),
                Node::from(detail).translate(40.0, 58.0),
            ])
            .opacity(weight),
        );
    }
    Node::group(nodes)
}
