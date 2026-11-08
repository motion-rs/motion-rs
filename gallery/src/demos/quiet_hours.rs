//! Hand-painted pastoral day in four equal shots: a sunlit ancient forest, a clear
//! stream, a dusk village above a sea of clouds and a firefly night. Everything is
//! procedural vector art: layered gradients and silhouettes, wind-swayed grass,
//! screen/plus light, per-shot bloom and a shared film grain. Run it for a one
//! minute film with `--params '{"seconds":60}'`.

use crate::{DemoMetadata, DemoParameters};
use dusk::dusk;
use forest::forest;
use motion::prelude::*;
use night::night;
use paint::color;
use stream::stream;

mod dusk;
mod forest;
mod night;
mod paint;
mod stream;

const WIDTH: f64 = 960.0;

const HEIGHT: f64 = 540.0;

const SHOTS: usize = 4;

/// Cross-dissolve half width as a fraction of the whole film.
const BLEND: f64 = 0.012;

const KAPPA: f64 = 0.552_284_75;

pub(super) const METADATA: DemoMetadata = DemoMetadata::new(
    "quiet-hours",
    "Quiet Hours",
    "Pastoral",
    &[
        "gradients",
        "procedural-paths",
        "blend-modes",
        "filters",
        "seeded-randomness",
    ],
);

#[derive(Clone, Copy)]
struct Rig {
    seed: u64,
    count: u64,
}

pub(super) fn timeline(parameters: DemoParameters) -> motion::Result<Timeline> {
    let duration = i64::from(parameters.seconds).seconds();
    let rig = Rig {
        seed: parameters.seed,
        count: u64::from(parameters.count),
    };
    Ok(Timeline::new(Size::new(WIDTH, HEIGHT), 30)
        .track(|track| track.scene(duration, move |context| film(context, rig))))
}

fn film(context: &SceneContext<'_>, rig: Rig) -> Node {
    let total = context
        .clip_duration()
        .map_or(8.0, |duration| duration.as_seconds());
    let progress = context.progress();
    let slot = 1.0 / SHOTS as f64;
    let mut layers: Vec<Node> = Vec::new();
    for shot in 0..SHOTS {
        let start = shot as f64 * slot;
        let end = start + slot;
        // A shot paints from just before its slot until the next one is opaque.
        if progress < start - BLEND || progress > end + BLEND {
            continue;
        }
        let local = ((progress - start) / slot).clamp(0.0, 1.0);
        let seconds = (progress - start) * total;
        let node = match shot {
            0 => forest(context, rig, local, seconds),
            1 => stream(context, rig, local, seconds),
            2 => dusk(context, rig, local, seconds),
            _ => night(context, rig, local, seconds),
        };
        let fade_in = if shot == 0 {
            1.0
        } else {
            math::smoothstep(start - BLEND, start + BLEND, progress)
        };
        layers.push(node.opacity(fade_in));
    }
    let veil = 1.0 - math::smoothstep(0.0, BLEND * 2.0, progress)
        + math::smoothstep(1.0 - BLEND * 2.0, 1.0, progress);
    layers.push(
        context
            .bounds()
            .fill(color(0x050912).with_alpha(veil.clamp(0.0, 1.0)))
            .z_index(10),
    );
    let frame = context.global_time().to_frame(context.frame_rate()).max(0) as u64;
    Node::group(layers)
        .clip_with(context.bounds(), ClipMode::Geometric)
        .filter(Filter::Optics(motion::Optics::LensVignette {
            angle_of_view: 75.0,
            strength: 0.28,
            roundness: 0.0,
        }))
        .filter(Filter::grain(rig.seed ^ frame, 0.008))
}
