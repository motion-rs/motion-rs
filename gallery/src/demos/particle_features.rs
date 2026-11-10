//! Particle capability studies with explanatory text and isolated, seekable specimens.

use super::atlas::text;
use crate::{DemoMetadata, DemoParameters};
use catalog::PAGES;
use motion::particles::{
    BurstEmitter, DiskEmitter, ParticleSimulation, ParticleSimulationBuilder, ParticleStyle,
    ParticleTrajectory, PointEmitter, RectEmitter, SimulationLimits,
};
use motion::prelude::*;

mod appearance;
mod catalog;
mod emission;
mod physics;
mod timing;

pub(super) const METADATA: DemoMetadata = DemoMetadata::new(
    "particle-features",
    "Particle Features",
    "Particles",
    &[
        "emitters",
        "analytic-trajectories",
        "forces",
        "collisions",
        "styles",
        "time-control",
        "seeded-replay",
    ],
);

/// Local specimen bounds in design units; clipping belongs to the outer group.
const VIEWPORT: Rect = Rect::new(0.0, 0.0, 512.0, 304.0);

/// Finite source duration in seconds, independent of presentation speed.
const SOURCE_SECONDS: f64 = 4.0;

/// Per-simulation preparation/query work allowance for the full capability specimens.
const SIMULATION_WORK: u64 = 1_600_000_000;

/// Multiplier from the user `count` to the population every specimen derives from.
const DENSITY: u32 = 2;

struct Page {
    title: &'static str,
    category: &'static str,
    lines: [&'static str; 3],
    build: fn(DemoParameters) -> motion::Result<Study>,
}

struct Study {
    source: Specimen,
    guides: Node,
}

impl Study {
    fn new(simulation: ParticleSimulationBuilder) -> Self {
        Self {
            source: Specimen::Simulation(simulation.build()),
            guides: Node::empty(),
        }
    }

    fn trajectory(trajectory: ParticleTrajectory) -> Self {
        Self {
            source: Specimen::Trajectory(trajectory),
            guides: Node::empty(),
        }
    }

    fn render(&self, context: &SceneContext<'_>) -> motion::Result<Node> {
        Ok(match &self.source {
            Specimen::Simulation(simulation) => simulation
                .as_ref()
                .map_err(Clone::clone)?
                .try_render(context)?,
            Specimen::Trajectory(trajectory) => trajectory.try_render(context)?,
            Specimen::Layers(layers) => Node::group(
                layers
                    .iter()
                    .map(|(simulation, offset)| {
                        simulation
                            .as_ref()
                            .map_err(Clone::clone)?
                            .try_render(context)
                            .map(|node| node.translate(offset.x, offset.y))
                    })
                    .collect::<Result<Vec<_>, _>>()?,
            ),
            Specimen::Text(parameters) => {
                let simulation = context.cache(
                    ("particle-feature-text", parameters.seed, parameters.count),
                    || emission::text(context, *parameters),
                );
                let simulation = simulation.as_ref().as_ref().map_err(Clone::clone)?;
                simulation.try_render(context)?.translate(108.0, 100.0)
            }
        })
    }
}

enum Specimen {
    Simulation(Result<ParticleSimulation, ValidationError>),
    Trajectory(ParticleTrajectory),
    Layers(Vec<(Result<ParticleSimulation, ValidationError>, Point)>),
    Text(DemoParameters),
}

pub(super) fn timeline(parameters: DemoParameters) -> motion::Result<Timeline> {
    let dense = DemoParameters {
        count: parameters.count * DENSITY,
        ..parameters
    };
    let studies = PAGES
        .iter()
        .map(|page| (page.build)(dense))
        .collect::<motion::Result<Vec<_>>>()?;
    Timeline::new(Size::new(960.0, 540.0), 30)
        .background(Color::from_hex(super::atlas::PAPER))
        .track(|track| {
            track.scene(parameters.seconds.seconds(), move |context| {
                frame(context, &studies)
            })
        })
        .build()
}

fn frame(context: &SceneContext<'_>, studies: &[Study]) -> motion::Result<Node> {
    let (page, local) = page_at(context.progress());
    let description = &PAGES[page];
    let sampled = context
        .try_shifted(context.time() - (0.25 + local * 3.4).seconds())?
        .with_clip_duration(SOURCE_SECONDS.seconds());
    let study = &studies[page];
    let particles = study.render(&sampled)?;
    Ok(super::atlas::sheet(
        &super::atlas::Sheet {
            metadata: METADATA,
            page,
            count: PAGES.len(),
            progress: context.progress(),
            title: description.title,
            lines: &description.lines,
            tag: Some(description.category),
            next: PAGES.get(page + 1).map(|next| next.title),
        },
        Node::group([stage(), study.guides.clone(), particles]),
    ))
}

fn page_at(progress: f64) -> (usize, f64) {
    let scaled = progress * PAGES.len() as f64;
    let page = (scaled as usize).min(PAGES.len() - 1);
    (page, (scaled - page as f64).clamp(0.0, 1.0))
}

fn base(parameters: DemoParameters) -> ParticleSimulationBuilder {
    ParticleSimulation::new(parameters.seed)
        .duration(SOURCE_SECONDS.seconds())
        .limits(SimulationLimits {
            max_work: SIMULATION_WORK,
            ..SimulationLimits::default()
        })
        .style(ParticleStyle::Dots { radius: 2.4 })
}

fn point(parameters: DemoParameters) -> motion::Result<Study> {
    Ok(Study::new(
        base(parameters).emitter(
            PointEmitter::new(VIEWPORT.center())
                .rate(f64::from(parameters.count))
                .speed(32.0)
                .lifetime(3.seconds())
                .color([0.15, 0.85, 0.65, 1.0]),
        ),
    ))
}

fn disk(parameters: DemoParameters) -> motion::Result<Study> {
    let mut study = Study::new(
        base(parameters).emitter(
            DiskEmitter::new(VIEWPORT.center(), 72.0)
                .rate(f64::from(parameters.count))
                .speed(8.0)
                .lifetime(3.seconds())
                .color([0.3, 0.65, 1.0, 1.0]),
        ),
    );
    study.guides = Circle::new(VIEWPORT.center(), 72.0)
        .fill(Color::from_hex(0x263e35))
        .into();
    Ok(study)
}

fn rectangle(parameters: DemoParameters) -> motion::Result<Study> {
    let bounds = Rect::new(96.0, 72.0, 416.0, 232.0);
    let mut study = Study::new(
        base(parameters).emitter(
            RectEmitter::new(bounds)
                .rate(f64::from(parameters.count))
                .speed(4.0)
                .lifetime(3.seconds())
                .color([1.0, 0.55, 0.25, 1.0]),
        ),
    );
    study.guides = bounds.fill(Color::from_hex(0x263e35)).into();
    Ok(study)
}

fn burst(parameters: DemoParameters) -> motion::Result<Study> {
    Ok(Study::new(
        base(parameters).emitter(
            BurstEmitter::new(Time::ZERO, parameters.count as usize * 2, VIEWPORT.center())
                .speed(34.0)
                .lifetime(SOURCE_SECONDS.seconds())
                .color([1.0, 0.65, 0.25, 1.0]),
        ),
    ))
}

fn stage() -> Node {
    let mut nodes: Vec<Node> = vec![VIEWPORT.fill(Color::from_hex(0x17231f)).into()];
    for column in 1..8 {
        let left = column as f64 * 64.0;
        nodes.push(
            Rect::new(left, 0.0, left + 0.5, 304.0)
                .fill(Color::from_hex(0x26372f))
                .into(),
        );
    }
    for row in 1..5 {
        let top = row as f64 * 64.0;
        nodes.push(
            Rect::new(0.0, top, 512.0, top + 0.5)
                .fill(Color::from_hex(0x26372f))
                .into(),
        );
    }
    Node::group(nodes)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pages_have_unique_titles_and_exact_duration() -> motion::Result {
        let mut titles = std::collections::BTreeSet::new();
        assert_eq!(PAGES.len(), 47);
        for (page, description) in PAGES.iter().enumerate() {
            assert!(titles.insert(description.title));
            assert!(description.lines.iter().all(|line| !line.is_empty()));
            assert_eq!(page_at((page as f64 + 0.5) / PAGES.len() as f64).0, page);
        }
        assert_eq!(page_at(1.0), (PAGES.len() - 1, 1.0));
        for seconds in [1, 8, 60] {
            let timeline = timeline(DemoParameters {
                seconds,
                ..DemoParameters::default()
            })?;
            assert_eq!(timeline.duration(), seconds.seconds());
            assert_eq!(timeline.frame_count(), u64::from(seconds) * 30);
        }
        Ok(())
    }

    #[test]
    #[ignore = "requires explicit MOTION_TEST_FONT; every capability subject, motion and exact replay"]
    fn every_page_renders_animates_and_replays() -> motion::Result {
        let font = std::env::var_os("MOTION_TEST_FONT")
            .ok_or_else(|| std::io::Error::other("Set MOTION_TEST_FONT"))?;
        let options = crate::render_options(Some(std::path::Path::new(&font)))?
            .backend(BackendPreference::Cpu)
            .size(PixelSize::new(288, 162));
        let parameters = DemoParameters {
            seconds: 60,
            count: 96,
            ..DemoParameters::default()
        };
        let timeline = timeline(parameters)?;
        let empty_studies = PAGES
            .iter()
            .map(|page| {
                let mut study = (page.build)(parameters)?;
                study.source = Specimen::Simulation(base(parameters).build());
                Ok(study)
            })
            .collect::<motion::Result<Vec<_>>>()?;
        let background = Timeline::new(Size::new(960.0, 540.0), 30)
            .background(Color::from_hex(super::super::atlas::PAPER))
            .track(|track| track.scene(60.seconds(), move |context| frame(context, &empty_studies)))
            .build()?;
        for (page, description) in PAGES.iter().enumerate() {
            let moment = |local: f64| ((page as f64 + local) * 60.0 / PAGES.len() as f64).seconds();
            let early = render_still(&timeline, moment(0.25), &options)?;
            let late = render_still(&timeline, moment(0.75), &options)?;
            for local in [0.05, 0.5, 0.95] {
                render_still(&timeline, moment(local), &options)?;
            }
            let replay = render_still(&timeline, moment(0.25), &options)?;
            assert_eq!(
                early.bytes(),
                replay.bytes(),
                "{} replay",
                description.title
            );
            let changed = (46..136)
                .flat_map(|row| (117..269).map(move |column| (row * 288 + column) * 4))
                .filter(|offset| {
                    early.bytes()[*offset..*offset + 4] != late.bytes()[*offset..*offset + 4]
                })
                .count();
            assert!(changed > 8, "{} did not animate", description.title);
            let empty = render_still(&background, moment(0.25), &options)?;
            let painted = (46..136)
                .flat_map(|row| (117..269).map(move |column| (row * 288 + column) * 4))
                .filter(|offset| {
                    early.bytes()[*offset..*offset + 3]
                        .iter()
                        .zip(&empty.bytes()[*offset..*offset + 3])
                        .any(|(actual, background)| actual.abs_diff(*background) >= 8)
                })
                .count();
            assert!(painted > 4, "{} missing visible subject", description.title);
            let leaked = (0..162)
                .flat_map(|row| (0..288).map(move |column| (row, column)))
                .filter(|(row, column)| !(36..141).contains(row) || !(102..276).contains(column))
                .map(|(row, column)| (row * 288 + column) * 4)
                .any(|offset| {
                    early.bytes()[offset..offset + 4]
                        .iter()
                        .zip(&empty.bytes()[offset..offset + 4])
                        .any(|(actual, background)| actual.abs_diff(*background) > 1)
                });
            assert!(!leaked, "{} escaped its viewport", description.title);
        }
        Ok(())
    }
}
