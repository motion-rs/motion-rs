//! Recipe showcase; each time slot selects one preset with a Bloom comparison.

use crate::{DemoMetadata, DemoParameters};
use motion::particles::presets::{Preset, PresetOptions};
use motion::prelude::*;
use motion::vector::{VectorGroup, VectorItem, VectorPath, VectorTree};
use std::sync::Arc;

pub(super) const METADATA: DemoMetadata = DemoMetadata::new(
    "particle-recipes",
    "Particle Recipes",
    "Particles",
    &[
        "particles",
        "bokeh",
        "sprites",
        "collision",
        "sub-emitters",
        "bloom",
    ],
);

/// Shared outline source used by the two colored logo recipes.
pub(super) fn logo(bounds: Rect) -> motion::Result<Arc<VectorTree>> {
    Ok(Arc::new(VectorTree::new(
        bounds,
        VectorGroup {
            children: vec![VectorItem::Path(VectorPath::filled(
                Path2D::from_rect(Rect::new(
                    bounds.left + bounds.width() * 0.35,
                    bounds.top + bounds.height() * 0.3,
                    bounds.left + bounds.width() * 0.65,
                    bounds.top + bounds.height() * 0.7,
                ))?,
                LinearGradient::new(Point::ORIGIN, Point::new(1.0, 0.0))
                    .stop(0.0, Color::from_hex(0x25bb90))
                    .stop(1.0, Color::from_hex(0xff4060)),
            ))],
            ..Default::default()
        },
    )?))
}

pub(super) fn timeline(parameters: DemoParameters) -> motion::Result<Timeline> {
    let duration = parameters.seconds.seconds();
    let bounds = Rect::new(0.0, 0.0, 480.0, 540.0);
    let source = logo(bounds)?;
    let mut simulations = Vec::new();
    for preset in Preset::ALL {
        let mut options = PresetOptions::new(parameters.seed, 3.seconds(), bounds);
        options.logo = Some(source.clone());
        simulations.push(preset.simulation(options)?);
    }
    Ok(Timeline::new(Size::new(960.0, 540.0), 30).track(|track| {
        track.scene(duration, move |context| {
            let index = ((context.progress() * Preset::ALL.len() as f64) as usize)
                .min(Preset::ALL.len() - 1);
            let simulation = &simulations[index];
            let slot = duration.as_seconds() / Preset::ALL.len() as f64;
            let local = (context.seconds() / slot).fract() * 2.9;
            let plain = context.during(context.time() - local.seconds(), 3.seconds(), |inner| {
                simulation.render(inner)
            });
            let bloom = plain
                .clone()
                .filter(Filter::Bloom {
                    threshold: 0.4,
                    knee: 0.2,
                    radius: 12.0,
                    intensity: 0.8,
                    levels: 3,
                    falloff: 0.6,
                    tint: Color::WHITE,
                })
                .translate(480.0, 0.0);
            Node::group([
                context.bounds().fill(Color::from_hex(0x101418)).into(),
                plain,
                bloom,
            ])
            .clip_with(context.bounds(), ClipMode::Geometric)
        })
    }))
}

/// Writes Bloom off/on x output tone-map off/AgX frames for every recipe.
/// Callers choose the output directory; no fonts, external assets or device are required.
pub fn comparison_frames(directory: &std::path::Path, options: &RenderOptions) -> motion::Result {
    std::fs::create_dir_all(directory)?;
    let bounds = Rect::new(0.0, 0.0, 320.0, 180.0);
    let source = logo(bounds)?;
    for preset in Preset::ALL {
        let mut recipe = PresetOptions::new(7, 3.seconds(), bounds);
        recipe.logo = Some(source.clone());
        let simulation = preset.simulation(recipe)?;
        let time = if matches!(preset, Preset::Sparks) {
            0.35.seconds()
        } else if preset == Preset::Fireworks {
            1.seconds()
        } else {
            1.5.seconds()
        };
        for bloom in [false, true] {
            for tone_map in [ToneMap::None, ToneMap::AgX] {
                let simulation = simulation.clone();
                let timeline = Timeline::new(Size::new(320.0, 180.0), 30).track(|track| {
                    track.scene(3.seconds(), move |context| {
                        let particles = simulation.render(context);
                        let particles = if bloom {
                            particles.filter(Filter::Bloom {
                                threshold: 0.4,
                                knee: 0.2,
                                radius: 4.0,
                                levels: 3,
                                falloff: 0.6,
                                intensity: 0.8,
                                tint: Color::WHITE,
                            })
                        } else {
                            particles
                        };
                        Node::group([
                            context.bounds().fill(Color::from_hex(0x101418)).into(),
                            particles,
                        ])
                    })
                });
                let frame = render_still(
                    &timeline,
                    time,
                    &options
                        .clone()
                        .size(PixelSize::new(320, 180))
                        .display(DisplayTransform {
                            tone_map,
                            ..Default::default()
                        }),
                )?;
                let stem = preset.name().to_ascii_lowercase().replace(' ', "-");
                frame.save_png(directory.join(format!(
                    "{stem}-bloom-{bloom}-tone-{}.png",
                    if tone_map == ToneMap::None {
                        "none"
                    } else {
                        "agx"
                    }
                )))?;
            }
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn particle_comparison_frames_keep_subjects_and_output_controls() -> motion::Result {
        let directory = tempfile::tempdir()?;
        let options = RenderOptions::default().backend(BackendPreference::Cpu);
        comparison_frames(directory.path(), &options)?;
        assert_eq!(
            std::fs::read_dir(directory.path())?.count(),
            Preset::ALL.len() * 4
        );
        let mut bloom_changes = 0;
        for preset in Preset::ALL {
            let stem = preset.name().to_ascii_lowercase().replace(' ', "-");
            let load = |bloom: bool, tone: &str| -> motion::Result<image::RgbImage> {
                Ok(image::open(
                    directory
                        .path()
                        .join(format!("{stem}-bloom-{bloom}-tone-{tone}.png")),
                )
                .map_err(std::io::Error::other)?
                .to_rgb8())
            };
            let raw = load(false, "none")?;
            let bloom = load(true, "none")?;
            let tone = load(false, "agx")?;
            assert_eq!(raw.dimensions(), (320, 180));
            let background = raw.get_pixel(0, 0);
            assert!(
                raw.pixels().filter(|pixel| *pixel != background).count() > 4,
                "missing subject: {preset:?}"
            );
            assert_ne!(
                raw.as_raw(),
                tone.as_raw(),
                "missing tone-map change: {preset:?}"
            );
            if raw.as_raw() != bloom.as_raw() {
                bloom_changes += 1;
            }
        }
        assert!(
            bloom_changes >= 8,
            "Bloom must change all HDR recipes and visible highlights"
        );
        let first = directory.path().join("sparks-bloom-false-tone-none.png");
        let bytes = std::fs::read(&first)?;
        assert!(comparison_frames(directory.path(), &options).is_err());
        assert_eq!(bytes, std::fs::read(first)?);
        Ok(())
    }
}
