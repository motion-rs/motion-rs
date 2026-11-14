//! The gallery is a small film program. Each film owns its metadata and timeline.

use crate::{AuthoringMode, DemoMetadata, DemoParameters};
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
mod json_animation_atlas;
mod json_atlas;
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
        build: animation_atlas_timeline,
    },
    Film {
        metadata: brush_atlas::METADATA,
        build: brush_atlas::timeline,
    },
    Film {
        metadata: particle_atlas::METADATA,
        build: particle_atlas_timeline,
    },
    Film {
        metadata: particle_features::METADATA,
        build: particle_features_timeline,
    },
    Film {
        metadata: filter_atlas::METADATA,
        build: filter_atlas_timeline,
    },
    Film {
        metadata: compositing_atlas::METADATA,
        build: compositing_atlas_timeline,
    },
    Film {
        metadata: vector_atlas::METADATA,
        build: vector_atlas_timeline,
    },
    Film {
        metadata: ink_features::METADATA,
        build: ink_features_timeline,
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
        build: ink_journey_timeline,
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

fn animation_atlas_timeline(parameters: DemoParameters) -> Result<Timeline, motion::ErrorReport> {
    match parameters.mode {
        AuthoringMode::Default => animation_atlas::timeline(parameters),
        AuthoringMode::Json => json_animation_atlas::timeline(parameters),
    }
}

fn compositing_atlas_timeline(parameters: DemoParameters) -> Result<Timeline, motion::ErrorReport> {
    match parameters.mode {
        AuthoringMode::Default => compositing_atlas::timeline(parameters),
        AuthoringMode::Json => json_atlas::timeline(
            json_atlas::COMPOSITING_DOCUMENT,
            parameters,
            Default::default(),
        ),
    }
}

fn particle_atlas_timeline(parameters: DemoParameters) -> Result<Timeline, motion::ErrorReport> {
    match parameters.mode {
        AuthoringMode::Default => particle_atlas::timeline(parameters),
        AuthoringMode::Json => json_atlas::timeline(
            json_atlas::PARTICLE_DOCUMENT,
            parameters,
            [
                ("seed".into(), parameters.seed.into()),
                ("count".into(), parameters.count.into()),
            ]
            .into_iter()
            .collect(),
        ),
    }
}

fn filter_atlas_timeline(parameters: DemoParameters) -> Result<Timeline, motion::ErrorReport> {
    match parameters.mode {
        AuthoringMode::Default => filter_atlas::timeline(parameters),
        AuthoringMode::Json => json_atlas::timeline(
            json_atlas::FILTER_DOCUMENT,
            parameters,
            [("seed".into(), parameters.seed.into())]
                .into_iter()
                .collect(),
        ),
    }
}

fn particle_features_timeline(parameters: DemoParameters) -> Result<Timeline, motion::ErrorReport> {
    match parameters.mode {
        AuthoringMode::Default => particle_features::timeline(parameters),
        AuthoringMode::Json => json_atlas::timeline(
            json_atlas::PARTICLE_FEATURES_DOCUMENT,
            parameters,
            [
                ("seed".into(), parameters.seed.into()),
                // Native curl fields use the low 32 bits of the seed.
                ("curl_seed".into(), (parameters.seed as u32).into()),
                ("count".into(), parameters.count.into()),
            ]
            .into_iter()
            .collect(),
        ),
    }
}

fn vector_atlas_timeline(parameters: DemoParameters) -> Result<Timeline, motion::ErrorReport> {
    match parameters.mode {
        AuthoringMode::Default => vector_atlas::timeline(parameters),
        AuthoringMode::Json => json_atlas::timeline(
            json_atlas::VECTOR_DOCUMENT,
            parameters,
            [("seed".into(), parameters.seed.into())]
                .into_iter()
                .collect(),
        ),
    }
}

fn ink_features_timeline(parameters: DemoParameters) -> Result<Timeline, motion::ErrorReport> {
    match parameters.mode {
        AuthoringMode::Default => ink_features::timeline(parameters),
        AuthoringMode::Json => json_atlas::timeline(
            json_atlas::INK_FEATURES_DOCUMENT,
            parameters,
            [
                ("seed".into(), parameters.seed.into()),
                (
                    "ink_seed".into(),
                    (parameters.seed as u32 ^ (parameters.seed >> 32) as u32).into(),
                ),
                ("fonts".into(), serde_json::json!([])),
            ]
            .into_iter()
            .collect(),
        ),
    }
}

fn ink_journey_timeline(parameters: DemoParameters) -> Result<Timeline, motion::ErrorReport> {
    match parameters.mode {
        AuthoringMode::Default => ink_journey::timeline(parameters),
        AuthoringMode::Json => json_atlas::timeline(
            json_atlas::INK_JOURNEY_DOCUMENT,
            parameters,
            [
                ("seed".into(), parameters.seed.into()),
                ("ink_seed".into(), (parameters.seed as u32).into()),
                ("fonts".into(), serde_json::json!([])),
            ]
            .into_iter()
            .collect(),
        ),
    }
}
