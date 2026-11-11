//! The gallery is a small film program. Each film owns its metadata and timeline.

use crate::{DemoMetadata, DemoParameters};
use motion::prelude::*;

mod animation_atlas;
mod atlas;
mod audio_atlas;
mod brush_atlas;
mod caption_atlas;
mod compositing_atlas;
mod data_story;
mod filter_atlas;
#[cfg(feature = "formula")]
mod harmonics;
mod ink_features;
mod ink_journey;
mod logo_sting;
mod noise_atlas;
mod particle_atlas;
mod particle_features;
mod particle_garden;
pub(crate) mod particle_recipes;
mod signal_garden;
mod type_in_motion;
mod typography_atlas;
mod vector_atlas;

/// A tracked single-line label. Families that are not installed fall back to the
/// system sans, so the films stay renderable with an explicit `--font`.
fn label(
    value: &str,
    family: &str,
    size: f64,
    spacing: f64,
    weight: FontWeight,
    fill: impl Into<Paint>,
) -> Text {
    Text::new(value.to_owned())
        .family(family.to_owned())
        .size(size)
        .weight(weight)
        .letter_spacing(spacing)
        .fill(fill)
}

/// Frame number used to reseed film grain so it flickers like emulsion.
fn frame_seed(context: &SceneContext<'_>) -> u64 {
    context.global_time().to_frame(context.frame_rate()).max(0) as u64
}

struct Film {
    metadata: DemoMetadata,
    build: fn(DemoParameters) -> Result<Timeline, motion::ErrorReport>,
}

const PROGRAM: &[Film] = &[
    Film {
        metadata: animation_atlas::METADATA,
        build: animation_atlas::timeline,
    },
    Film {
        metadata: brush_atlas::METADATA,
        build: brush_atlas::timeline,
    },
    Film {
        metadata: particle_atlas::METADATA,
        build: particle_atlas::timeline,
    },
    Film {
        metadata: particle_features::METADATA,
        build: particle_features::timeline,
    },
    Film {
        metadata: filter_atlas::METADATA,
        build: filter_atlas::timeline,
    },
    Film {
        metadata: compositing_atlas::METADATA,
        build: compositing_atlas::timeline,
    },
    Film {
        metadata: vector_atlas::METADATA,
        build: vector_atlas::timeline,
    },
    Film {
        metadata: ink_features::METADATA,
        build: ink_features::timeline,
    },
    Film {
        metadata: noise_atlas::METADATA,
        build: noise_atlas::timeline,
    },
    Film {
        metadata: audio_atlas::METADATA,
        build: audio_atlas::timeline,
    },
    Film {
        metadata: typography_atlas::METADATA,
        build: typography_atlas::timeline,
    },
    Film {
        metadata: caption_atlas::METADATA,
        build: caption_atlas::timeline,
    },
    Film {
        metadata: particle_garden::METADATA,
        build: particle_garden::timeline,
    },
    Film {
        metadata: data_story::METADATA,
        build: data_story::timeline,
    },
    Film {
        metadata: logo_sting::METADATA,
        build: logo_sting::timeline,
    },
    Film {
        metadata: type_in_motion::METADATA,
        build: type_in_motion::timeline,
    },
    Film {
        metadata: ink_journey::METADATA,
        build: ink_journey::timeline,
    },
    Film {
        metadata: signal_garden::METADATA,
        build: signal_garden::timeline,
    },
    #[cfg(feature = "formula")]
    Film {
        metadata: harmonics::METADATA,
        build: harmonics::timeline,
    },
];

pub(super) fn catalog() -> Vec<DemoMetadata> {
    PROGRAM.iter().map(|film| film.metadata).collect()
}

pub(super) fn timeline(info: DemoMetadata, parameters: DemoParameters) -> motion::Result<Timeline> {
    let film = PROGRAM
        .iter()
        .find(|film| film.metadata.id == info.id)
        .ok_or_else(|| {
            std::io::Error::new(std::io::ErrorKind::InvalidInput, "Unknown gallery film")
        })?;
    Ok((film.build)(parameters)?)
}
