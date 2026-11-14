//! Motion brand ident: a JSON `motion::declarative` document draws the construction
//! grid, chrome, burst and wordmark lockup, while Rust springs three primitives on
//! screen, morphs them into the SVG logo mark and sweeps a trimmed orbit around it.

use crate::{DemoMetadata, DemoParameters};
use motion::declarative::{CompileOptions, Document};
use motion::eager::{Morph, VectorTree};
use motion::prelude::*;
use motion::svg::SvgAsset;
use std::path::Path;
use std::sync::Arc;

pub(super) const METADATA: DemoMetadata = DemoMetadata::new(
    "logo-sting",
    "Motion",
    "Branding",
    &[
        "declarative-json",
        "spring-keyframes",
        "vector-morph",
        "trim-path",
        "svg-artwork",
    ],
);

const DOCUMENT: &str = include_str!("../../assets/solstice-ident.json");

const MARK: &[u8] = include_bytes!("../../assets/solstice-mark.svg");

const LIME: u32 = 0xd7ff3c;

const CREAM: u32 = 0xf4f1ea;

/// The authored document timing is designed for an eight-second film.
const DESIGN_SECONDS: f64 = 8.0;

/// Centre of the Motion monogram inside the 240x240 artwork, used as its pivot.
const MARK_CENTER: Point = Point::new(120.0, 124.0);

/// The same three ids as the mark, drawn as plain geometry spread along the horizon.
const PRIMITIVES: &str = r##"<svg xmlns="http://www.w3.org/2000/svg" width="240" height="240" viewBox="0 0 240 240">
    <circle id="rise" cx="-56" cy="124" r="44" fill="#d7ff3c"/>
    <rect id="fold" x="80" y="84" width="80" height="80" fill="#f4f1ea"/>
    <path id="drive" d="M296 78 L344 164 L248 164 Z" fill="#d7ff3c"/>
</svg>"##;

pub(super) fn timeline(parameters: DemoParameters) -> Result<Timeline, motion::ErrorReport> {
    let seconds = parameters.seconds as f64;
    let k = seconds / DESIGN_SECONDS;
    let document = scaled_document(k)?;
    let (base, _) = document
        .compile(&CompileOptions::default().base_dir(assets_dir()))?
        .into_parts();

    let mark = Arc::new(SvgAsset::from_bytes(MARK)?);
    let primitives = SvgAsset::from_bytes(PRIMITIVES.as_bytes())?.tree;
    let morph = Morph::new(&primitives, &mark.tree)?;
    let mark_tree: Arc<VectorTree> = mark.tree.clone();

    // Each primitive lands on a stiff spring, sampled in design seconds.
    let pop = Spring::builder(0.0, 1.0)
        .stiffness(220.0)
        .damping(11.0)
        .build()?;
    let at = move |fraction: f64| (seconds * fraction).seconds();
    let lockup_scale = Keyframes::new(1.0)
        .to(at(0.5), 1.0, Easing::Linear)
        .to(at(0.53), 1.1, Easing::QuadraticOut)
        .to(at(0.62), 0.7, Easing::back_out(1.4)?);
    let lockup_y = Keyframes::new(250.0)
        .to(at(0.53), 250.0, Easing::Linear)
        .to(at(0.62), 196.0, Easing::back_out(1.4)?);

    base.to_builder()
        .track(|track| {
            track.scene(
                parameters.seconds.seconds(),
                move |context: &SceneContext<'_>| -> motion::Result<_> {
                    let time = context.time();
                    let progress = context.progress();
                    let design = context.seconds() / k;
                    let scale = lockup_scale.sample(time);
                    let centre = Point::new(480.0, lockup_y.sample(time));
                    let window = |start: f64, end: f64, easing: Easing| {
                        easing.apply(((progress - start) / (end - start)).clamp(0.0, 1.0))
                    };

                    let shape = if progress < 0.25 {
                        let pops = [0.24, 0.5, 0.76]
                            .map(|start| pop.sample(Time::from_seconds(design - start)));
                        primitives.node_mapped(&|address, node| {
                            node.scale(
                                pops.get(address.first().copied().unwrap_or(0))
                                    .copied()
                                    .unwrap_or(1.0),
                            )
                        })
                    } else if progress < 0.62 {
                        morph.sample(window(0.25, 0.5, Easing::CubicInOut))?
                    } else {
                        mark_tree.node()
                    };
                    let mark = shape
                        .pivot(Pivot::Point(MARK_CENTER))
                        .at(centre)
                        .scale(scale);

                    // A lime orbit sweeps around the forming monogram and leaves before the burst.
                    let head = window(0.2, 0.48, Easing::CubicInOut);
                    let tail = window(0.3, 0.52, Easing::CubicIn);
                    let orbit = Circle::new(Point::new(480.0, 250.0), 112.0)?
                        .stroke(4.0, Color::from_hex(LIME))
                        .trim(tail, head);
                    let baseline = Rect::new(0.0, 0.0, 1.0, 2.0)
                        .fill(Color::from_hex(CREAM))
                        .pivot(Pivot::Center)
                        .at(Point::new(480.0, 250.0))
                        .scale_xy(560.0 * window(0.05, 0.2, Easing::ExponentialOut), 1.0)
                        .opacity(1.0 - window(0.22, 0.3, Easing::Linear));
                    Ok((baseline, orbit, mark))
                },
            )
        })
        .build()
}

fn assets_dir() -> std::path::PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("assets")
}

/// Rescales every timing parameter of the eight-second document to the film length.
fn scaled_document(k: f64) -> motion::Result<Document> {
    let defaults = Document::from_json(DOCUMENT)?;
    let mut overrides = serde_json::Map::new();
    for (name, value) in &defaults.params {
        if let Some(number) = value.as_f64() {
            let scaled = if name == "k" { k } else { number * k };
            overrides.insert(name.clone(), serde_json::json!(scaled));
        }
    }
    Document::from_json_with(DOCUMENT, Some(&serde_json::Value::Object(overrides)))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    #[ignore = "requires explicit MOTION_TEST_FONT and native GPU when enabled; identity frames and replay"]
    fn motion_identity_renders_morphs_and_replays() -> motion::Result {
        let font = std::env::var_os("MOTION_TEST_FONT")
            .ok_or_else(|| std::io::Error::other("Set MOTION_TEST_FONT"))?;
        let options = crate::render_options(Some(std::path::Path::new(&font)))?
            .backend(BackendPreference::Cpu)
            .size(PixelSize::new(960, 540));
        for seconds in [8, 30] {
            let film = timeline(DemoParameters {
                seconds,
                ..DemoParameters::default()
            })?;
            for progress in [0.1, 0.3, 0.5, 0.75, 0.975] {
                let time = (f64::from(seconds) * progress).seconds();
                let first = render_still(&film, time.after_origin(), &options)?;
                render_still(&film, Time::ZERO, &options)?;
                let replay = render_still(&film, time.after_origin(), &options)?;
                assert_eq!(first.bytes(), replay.bytes());
                #[cfg(feature = "gpu")]
                {
                    let gpu_options = options.clone().backend(BackendPreference::gpu());
                    let gpu = render_still(&film, time.after_origin(), &gpu_options)?;
                    let difference = first
                        .bytes()
                        .iter()
                        .zip(gpu.bytes())
                        .map(|(cpu, gpu)| cpu.abs_diff(*gpu))
                        .max()
                        .unwrap_or(0);
                    assert!(difference <= 2, "CPU/GPU difference: {difference}");
                    let replay = render_still(&film, time.after_origin(), &gpu_options)?;
                    assert_eq!(gpu.bytes(), replay.bytes());
                }
            }
        }
        Ok(())
    }
}
