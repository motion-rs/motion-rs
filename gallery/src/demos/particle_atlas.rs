//! One page per particle preset, pairing a mechanism description with a retained simulation.

use crate::{DemoMetadata, DemoParameters};
use motion::particles::ParticleSimulation;
use motion::particles::presets::{Preset, PresetOptions};
use motion::prelude::*;

pub(super) const METADATA: DemoMetadata = DemoMetadata::new(
    "particle-atlas",
    "Particle Atlas",
    "Particles",
    &[
        "particle-presets",
        "fixed-step",
        "seeded-replay",
        "sub-emitters",
        "procedural-sprites",
        "colored-vector-emitters",
        "bloom",
    ],
);

/// Local design bounds shared by all thirteen simulation viewports.
const VIEWPORT: Rect = Rect::new(0.0, 0.0, 512.0, 304.0);

/// Finite source duration in seconds, independent of the selected film duration.
const SIMULATION_SECONDS: f64 = 4.0;

/// Emission baseline in viewport design units for the magnified smoke and fire specimens.
const SPECIMEN_FLOOR: f64 = 264.0;

pub(super) fn timeline(parameters: DemoParameters) -> Result<Timeline, motion::ErrorReport> {
    let logo = super::particle_recipes::logo(VIEWPORT)?;
    let simulations = Preset::ALL
        .into_iter()
        .map(|preset| {
            let mut options =
                PresetOptions::new(parameters.seed, SIMULATION_SECONDS.seconds(), VIEWPORT);
            options.density = f64::from(parameters.count) / 72.0;
            options.logo = Some(logo.clone());
            preset.simulation(options)?.build()
        })
        .collect::<Result<Vec<_>, _>>()?;
    Timeline::new(Size::new(960.0, 540.0), 30)
        .background(Color::from_hex(super::atlas::PAPER))
        .track(|track| {
            track.scene(parameters.seconds.seconds(), move |context| {
                frame(context, &simulations)
            })
        })
        .build()
}

fn frame(context: &SceneContext<'_>, simulations: &[ParticleSimulation]) -> motion::Result<Node> {
    let (page, local) = page_at(context.progress());
    let preset = Preset::ALL[page];
    let (category, lines) = description(preset);
    let sampled = context
        .try_shifted(context.time() - sample_time(preset, local))?
        .with_clip_duration(SIMULATION_SECONDS.seconds());
    let mut particles = simulations[page].try_render(&sampled)?;
    let zoom = match preset {
        Preset::Smoke => 2.0,
        Preset::Fire => 4.0,
        _ => 1.0,
    };
    if zoom > 1.0 {
        particles = particles
            .pivot(Pivot::Point(Point::ORIGIN))
            .scale(zoom)
            .translate(
                VIEWPORT.width() * (1.0 - zoom) * 0.5,
                SPECIMEN_FLOOR - VIEWPORT.bottom * zoom,
            );
    }
    if matches!(
        preset,
        Preset::Sparks | Preset::Embers | Preset::Fire | Preset::Fireworks | Preset::Fireflies
    ) {
        particles = particles.filter(Filter::Bloom {
            threshold: 0.4,
            knee: 0.2,
            radius: 4.0,
            levels: 3,
            falloff: 0.6,
            intensity: 0.8,
            tint: Color::WHITE,
        });
    }
    super::atlas::sheet(
        &super::atlas::Sheet {
            metadata: METADATA,
            page,
            count: Preset::ALL.len(),
            progress: context.progress(),
            title: preset.name(),
            lines: &lines,
            tag: Some(category),
            next: Preset::ALL.get(page + 1).map(|next| next.name()),
        },
        Node::group([stage(preset), particles]),
    )
}

fn page_at(progress: f64) -> (usize, f64) {
    let scaled = progress * Preset::ALL.len() as f64;
    let page = (scaled as usize).min(Preset::ALL.len() - 1);
    (page, (scaled - page as f64).clamp(0.0, 1.0))
}

fn sample_time(preset: Preset, local: f64) -> Time {
    match preset {
        Preset::Sparks => (0.12 + local * 1.35).seconds().after_origin(),
        Preset::Fireworks => (0.1 + local * 1.6).seconds().after_origin(),
        Preset::LogoDissolve | Preset::LogoAssemble => {
            (0.04 + local * 3.9).seconds().after_origin()
        }
        _ => (0.2 + local * 3.5).seconds().after_origin(),
    }
}

fn description(preset: Preset) -> (&'static str, [&'static str; 3]) {
    match preset {
        Preset::Sparks => (
            "BURST / TRAILS",
            [
                "A ballistic burst with",
                "tapered HDR trails and",
                "cooling blackbody color.",
            ],
        ),
        Preset::Embers => (
            "CURL / TEMPERATURE",
            [
                "Warm embers rise through",
                "a curl-noise field, cooling",
                "as their glow fades.",
            ],
        ),
        Preset::Dust => (
            "DRIFT / BOKEH",
            [
                "Slow dust drifts through",
                "a depth field with small,",
                "hexagonal bokeh.",
            ],
        ),
        Preset::Motes => (
            "DEPTH / PARALLAX",
            [
                "Soft floating motes use",
                "larger bokeh and subtle",
                "depth-driven parallax.",
            ],
        ),
        Preset::Snow => (
            "WIND / DEPTH",
            [
                "Three depth layers of snow",
                "fall through a gentle wind",
                "with drifting parallax.",
            ],
        ),
        Preset::Rain => (
            "COLLISION / SPLASH",
            [
                "Stretched drops strike",
                "the floor and trigger",
                "short-lived splash bursts.",
            ],
        ),
        Preset::Smoke => (
            "SPRITES / CURL",
            [
                "Procedural smoke sprites",
                "expand, curl and fade",
                "as they rise from below.",
            ],
        ),
        Preset::Fire => (
            "SPRITES / TEMPERATURE",
            [
                "Buoyant fire cores rise",
                "and shrink, cooling from",
                "white heat toward red.",
            ],
        ),
        Preset::Confetti => (
            "GRAVITY / FLIP",
            [
                "Colored quads tumble",
                "under gravity and drag,",
                "with seeded flip phases.",
            ],
        ),
        Preset::Fireworks => (
            "DEATH / SUB-EMITTERS",
            [
                "A rocket bursts on death,",
                "then its sparks trigger",
                "smaller secondary bursts.",
            ],
        ),
        Preset::Fireflies => (
            "DRIFT / INTENSITY",
            [
                "Seeded drift carries",
                "warm lights with gently",
                "breathing intensity.",
            ],
        ),
        Preset::LogoDissolve => (
            "VECTOR / DISPERSAL",
            [
                "A colored vector surface",
                "breaks into particles that",
                "keep their source colors.",
            ],
        ),
        Preset::LogoAssemble => (
            "VECTOR / CONVERGENCE",
            [
                "Scattered colored points",
                "ease back into their",
                "original vector surface.",
            ],
        ),
    }
}

fn stage(preset: Preset) -> Node {
    let light = matches!(preset, Preset::Confetti);
    let background = if light { 0xe7eeea } else { 0x17231f };
    let grid = if light { 0xd5dfd9 } else { 0x26372f };
    let mut nodes: Vec<Node> = vec![VIEWPORT.fill(Color::from_hex(background)).into()];
    for column in 1..8 {
        let left = column as f64 * 64.0;
        nodes.push(
            Rect::new(left, 0.0, left + 0.5, 304.0)
                .fill(Color::from_hex(grid))
                .into(),
        );
    }
    for row in 1..5 {
        let top = row as f64 * 64.0;
        nodes.push(
            Rect::new(0.0, top, 512.0, top + 0.5)
                .fill(Color::from_hex(grid))
                .into(),
        );
    }
    Node::group(nodes)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Maximum 8-bit channel difference between empty and HDR composition paths.
    const COMPOSITING_TOLERANCE: u8 = 1;

    #[test]
    fn every_preset_has_its_own_page_description_and_visible_clock() {
        assert_eq!(Preset::ALL.len(), 13);
        for (page, preset) in Preset::ALL.into_iter().enumerate() {
            let (actual, local) = page_at((page as f64 + 0.5) / Preset::ALL.len() as f64);
            assert_eq!(actual, page);
            assert!((local - 0.5).abs() <= 1e-12);
            let (category, lines) = description(preset);
            assert!(!category.is_empty());
            assert!(lines.iter().all(|line| !line.is_empty()));
            assert!(sample_time(preset, 0.0) > Time::ZERO);
            assert!(sample_time(preset, 1.0) < SIMULATION_SECONDS.seconds().after_origin());
        }
        assert_eq!(page_at(1.0), (Preset::ALL.len() - 1, 1.0));
        assert!(sample_time(Preset::Fireworks, 0.0) < 0.8.seconds().after_origin());
        assert!(sample_time(Preset::Fireworks, 1.0) > 0.8.seconds().after_origin());
    }

    #[test]
    fn fireworks_page_reaches_secondary_bursts() -> motion::Result {
        let simulation = Preset::Fireworks
            .simulation(PresetOptions::new(
                DemoParameters::default().seed,
                SIMULATION_SECONDS.seconds(),
                VIEWPORT,
            ))?
            .build()?;
        let particles = simulation.sample(sample_time(Preset::Fireworks, 0.95))?;
        assert!(particles.iter().any(|particle| particle.custom[3] == 2.0));
        Ok(())
    }

    #[test]
    fn timeline_retains_shared_size_frame_rate_and_duration() -> motion::Result {
        for seconds in [1, 8, 52, 60] {
            let timeline = timeline(DemoParameters {
                seconds,
                ..DemoParameters::default()
            })?;
            assert_eq!(timeline.size(), Size::new(960.0, 540.0));
            assert_eq!(timeline.duration(), seconds.seconds());
            assert_eq!(timeline.frame_count(), u64::from(seconds) * 30);
        }
        Ok(())
    }

    #[test]
    #[ignore = "requires explicit MOTION_TEST_FONT; all preset CPU subjects, motion and exact replay"]
    fn every_preset_renders_animates_and_replays_exactly() -> motion::Result {
        let font = std::env::var_os("MOTION_TEST_FONT")
            .ok_or_else(|| std::io::Error::other("Set MOTION_TEST_FONT"))?;
        let options = crate::render_options(Some(std::path::Path::new(&font)))?
            .backend(BackendPreference::Cpu)
            .size(PixelSize::new(288, 162));
        let timeline = timeline(DemoParameters {
            seconds: 52,
            ..DemoParameters::default()
        })?;
        let empty_simulations = vec![
            ParticleSimulation::new(0)
                .duration(SIMULATION_SECONDS.seconds())
                .build()?;
            Preset::ALL.len()
        ];
        let background = Timeline::new(Size::new(960.0, 540.0), 30)
            .background(Color::from_hex(super::super::atlas::PAPER))
            .track(|track| {
                track.scene(52.seconds(), move |context| {
                    frame(context, &empty_simulations)
                })
            })
            .build()?;
        for (page, preset) in Preset::ALL.into_iter().enumerate() {
            let time = (page as f64 * 4.0 + 1.0).seconds();
            let early = render_still(&timeline, time.after_origin(), &options)?;
            let later = render_still(&timeline, (time + 2.seconds()).after_origin(), &options)?;
            let replay = render_still(&timeline, time.after_origin(), &options)?;
            assert_eq!(early.bytes(), replay.bytes(), "{preset:?} replay");
            let offsets =
                || (46..136).flat_map(|row| (117..269).map(move |column| (row * 288 + column) * 4));
            let changed = offsets()
                .filter(|offset| {
                    early.bytes()[*offset..*offset + 4] != later.bytes()[*offset..*offset + 4]
                })
                .count();
            assert!(changed > 8, "{preset:?} did not animate");
            let empty = render_still(&background, time.after_origin(), &options)?;
            let leaked = (0..162)
                .flat_map(|row| (0..288).map(move |column| (row, column)))
                .filter(|(row, column)| !(36..141).contains(row) || !(102..276).contains(column))
                .map(|(row, column)| (row * 288 + column) * 4)
                .filter(|offset| {
                    early.bytes()[*offset..*offset + 4]
                        .iter()
                        .zip(&empty.bytes()[*offset..*offset + 4])
                        .any(|(actual, background)| {
                            actual.abs_diff(*background) > COMPOSITING_TOLERANCE
                        })
                })
                .take(8)
                .map(|offset| {
                    (
                        offset / 4 % 288,
                        offset / 4 / 288,
                        early.bytes()[offset..offset + 4].to_vec(),
                        empty.bytes()[offset..offset + 4].to_vec(),
                    )
                })
                .collect::<Vec<_>>();
            assert!(
                leaked.is_empty(),
                "{preset:?} escaped its viewport: {leaked:?}"
            );
            let painted = offsets()
                .filter(|offset| {
                    early.bytes()[*offset..*offset + 3]
                        .iter()
                        .zip(&empty.bytes()[*offset..*offset + 3])
                        .any(|(actual, background)| actual.abs_diff(*background) >= 8)
                })
                .count();
            assert!(painted > 4, "{preset:?} missing subject");
        }
        Ok(())
    }
}
