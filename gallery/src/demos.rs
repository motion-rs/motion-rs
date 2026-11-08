//! The gallery is a small film program. Each film owns its metadata and timeline.

use crate::{DemoMetadata, DemoParameters};
use motion::prelude::*;

mod audio_brush;
mod chidori;
mod chromatic_tide;
mod cinematic_look;
#[cfg(feature = "formula")]
mod harmonics;
mod ink_journey;
mod particle_garden;
pub(crate) mod particle_recipes;
mod quiet_hours;
mod sidechain_pulse;
mod signal_garden;
mod type_in_motion;

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
    build: fn(DemoParameters) -> motion::Result<Timeline>,
}

const PROGRAM: &[Film] = &[
    Film {
        metadata: audio_brush::METADATA,
        build: audio_brush::timeline,
    },
    Film {
        metadata: particle_recipes::METADATA,
        build: particle_recipes::timeline,
    },
    Film {
        metadata: particle_garden::METADATA,
        build: particle_garden::timeline,
    },
    Film {
        metadata: chromatic_tide::METADATA,
        build: chromatic_tide::timeline,
    },
    Film {
        metadata: cinematic_look::METADATA,
        build: cinematic_look::timeline,
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
    Film {
        metadata: sidechain_pulse::METADATA,
        build: sidechain_pulse::timeline,
    },
    Film {
        metadata: chidori::METADATA,
        build: chidori::timeline,
    },
    Film {
        metadata: quiet_hours::METADATA,
        build: quiet_hours::timeline,
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
    (film.build)(parameters)
}
