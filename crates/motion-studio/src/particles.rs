//! Particle recipes registered as ordinary workbench scenes.

use crate::StudioScene;
use motion_content::particles::presets::{Preset, PresetOptions};
use motion_core::{Color, Filter, Node, NodeModifiers, Size};
use motion_render::Composition;

/// Registers every recipe for preview/export using the supplied output settings.
/// Logo recipes require options.logo; failures are reported instead of skipping entries.
pub fn particle_presets(
    composition: Composition,
    options: PresetOptions,
    bloom: bool,
) -> motion_core::Result<Vec<StudioScene>> {
    let mut scenes = Vec::new();
    for preset in Preset::ALL {
        let simulation = preset.simulation(options.clone())?.build()?;
        let design = Size::new(options.bounds.width(), options.bounds.height());
        scenes.push(StudioScene::new(
            preset.name(),
            composition.clone(),
            design,
            move |context| {
                let particles = simulation
                    .try_render(context)
                    .unwrap_or_else(motion_core::authoring::transition::diagnostic);
                let particles = if bloom {
                    particles.filter(Filter::Bloom {
                        threshold: 0.4,
                        knee: 0.2,
                        radius: 5.0,
                        levels: 3,
                        falloff: 0.6,
                        intensity: 0.8,
                        tint: Color::WHITE,
                    })
                } else {
                    particles
                };
                Node::group([
                    Node::from(context.bounds().fill(Color::from_hex(0x101418))),
                    particles,
                ])
            },
        ));
    }
    Ok(scenes)
}
