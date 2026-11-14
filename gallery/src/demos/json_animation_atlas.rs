//! JSON-authored Animation Atlas with document-calculated, frame-aligned timing.

use super::animation_atlas::METADATA;
use crate::DemoParameters;
use motion::declarative::{Document, TrackItem};
use motion::prelude::*;
use serde_json::json;

const DOCUMENT: &str = include_str!("../../assets/animation-atlas.json");

pub(super) fn timeline(parameters: DemoParameters) -> Result<Timeline, motion::ErrorReport> {
    Ok(document(parameters)?
        .compile(&Default::default())?
        .into_parts()
        .0)
}

fn document(parameters: DemoParameters) -> motion::Result<Document> {
    let parameters = parameters.validate()?;
    let overrides = json!({
        "seconds": parameters.seconds,
        "seed": parameters.seed,
        "title": METADATA.title,
    });
    let mut document = Document::from_json_with(DOCUMENT, Some(&overrides))?;
    for track in &mut document.tracks {
        track.items.retain(|item| match item {
            TrackItem::Scene(scene) => scene.duration > 0.0,
            TrackItem::Transition(_) | TrackItem::Gap(_) => true,
        });
    }
    Ok(document)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn json_source_loads_without_rust_atlas_builders() -> motion::Result {
        let document = Document::from_json(DOCUMENT)?;
        assert_eq!(document.name.as_deref(), document.params["title"].as_str());
        assert_eq!(document.tracks[0].items.len(), 43);
        assert_eq!(
            document
                .compile(&Default::default())?
                .timeline()
                .frame_count(),
            240
        );
        Ok(())
    }

    #[test]
    fn json_parameters_preserve_page_boundaries_and_noise() -> motion::Result {
        for seconds in [1, 8, 60] {
            for seed in [17, u64::MAX] {
                let parameters = DemoParameters {
                    seconds,
                    seed,
                    ..DemoParameters::default()
                };
                let actual = document(parameters)?;
                assert_eq!(actual.name.as_deref(), Some(METADATA.title));
                assert_eq!(actual.params["title"], METADATA.title);
                assert_page_boundaries(&actual, seconds);
                assert_eq!(actual.params["seed"], json!(seed));
                assert_eq!(
                    actual
                        .compile(&Default::default())?
                        .timeline()
                        .frame_count(),
                    u64::from(seconds) * 30
                );
                assert_eq!(Document::from_json(&actual.to_json()?)?, actual);
            }
        }
        let first = document(DemoParameters {
            seed: 17,
            ..DemoParameters::default()
        })?;
        let second = document(DemoParameters {
            seed: 18,
            ..DemoParameters::default()
        })?;
        assert!(!first.params.contains_key("noise_curve"));
        assert_ne!(first.tracks[0].items[31], second.tracks[0].items[31]);
        Ok(())
    }

    #[test]
    fn json_seconds_override_recalculates_timing_without_adapter() -> motion::Result {
        for seconds in [8, 60] {
            let actual = Document::from_json_with(DOCUMENT, Some(&json!({"seconds": seconds})))?;
            assert_page_boundaries(&actual, seconds);
            assert_eq!(
                actual
                    .compile(&Default::default())?
                    .timeline()
                    .frame_count(),
                u64::from(seconds) * 30
            );
        }
        Ok(())
    }

    #[test]
    fn json_seed_only_override_compiles_sampled_curves() -> motion::Result {
        let document = Document::from_json_with(DOCUMENT, Some(&json!({"seed": 17})))?;
        assert_eq!(document.params["seed"], json!(17));
        assert!(!document.params.contains_key("noise_curve"));
        let count: usize = document.tracks[0]
            .items
            .iter()
            .map(|item| match item {
                TrackItem::Scene(scene) => sampled_curves(&scene.content),
                TrackItem::Transition(_) | TrackItem::Gap(_) => 0,
            })
            .sum();
        assert_eq!(count, 41);
        document.compile(&Default::default())?;
        Ok(())
    }

    fn assert_page_boundaries(document: &Document, seconds: u32) {
        let pages = super::super::animation_atlas::PAGES;
        let count = pages.len() as u64;
        let frames = u64::from(seconds) * 30;
        let rate = motion::fps!(30);
        let expected = pages
            .iter()
            .enumerate()
            .filter_map(|(index, page)| {
                let start = (frames * index as u64).div_ceil(count);
                let end = (frames * (index as u64 + 1)).div_ceil(count);
                (start != end).then_some((page, start, end))
            })
            .collect::<Vec<_>>();
        assert_eq!(document.tracks[0].items.len(), expected.len());
        for (item, (page, start, end)) in document.tracks[0].items.iter().zip(expected) {
            let TrackItem::Scene(scene) = item else {
                panic!("Atlas pages must be scenes");
            };
            let start = Duration::from_frame(start, rate);
            let end = Duration::from_frame(end, rate);
            assert_eq!(scene.start, Some(start.as_seconds()));
            assert_eq!(scene.duration, (end - start).as_seconds());
            assert_eq!(scene.name.as_deref(), Some(page.title));
        }
    }

    fn sampled_curves(element: &motion::declarative::Element) -> usize {
        match &element.content {
            motion::declarative::Content::Group(children) => {
                children.iter().map(sampled_curves).sum()
            }
            motion::declarative::Content::Polyline(spec) => usize::from(spec.sample.is_some()),
            _ => 0,
        }
    }

    #[test]
    fn json_parameters_reject_invalid_duration() {
        for seconds in [0, 61] {
            assert!(
                document(DemoParameters {
                    seconds,
                    ..DemoParameters::default()
                })
                .is_err()
            );
        }
    }

    #[test]
    #[ignore = "requires explicit MOTION_TEST_FONT; compares complete native and JSON CPU frames"]
    fn json_frames_match_native_reference_and_replay() -> motion::Result {
        let font = std::env::var_os("MOTION_TEST_FONT")
            .ok_or_else(|| std::io::Error::other("Set MOTION_TEST_FONT"))?;
        let options = crate::render_options(Some(std::path::Path::new(&font)))?
            .backend(BackendPreference::Cpu)
            .size(PixelSize::new(288, 162));
        let project = crate::project(options.clone())?;
        for (seconds, seed) in [
            (8, DemoParameters::default().seed),
            (1, 17),
            (8, 18),
            (60, u64::MAX),
        ] {
            let actual = project.build(
                "animation-atlas",
                Some(json!({
                    "mode": "json", "seconds": seconds, "seed": seed,
                })),
            )?;
            let reference = project.build(
                "animation-atlas",
                Some(json!({
                    "mode": "default", "seconds": seconds, "seed": seed,
                })),
            )?;
            assert_eq!(actual.size(), reference.size());
            assert_eq!(actual.frame_count(), reference.frame_count());
            let rate = motion::fps!(30);
            let mut frames = if seconds <= 8 {
                (0..actual.frame_count() as i64).collect::<Vec<_>>()
            } else {
                let document = document(DemoParameters {
                    seconds,
                    seed,
                    ..DemoParameters::default()
                })?;
                let mut frames = Vec::new();
                for item in &document.tracks[0].items {
                    let TrackItem::Scene(scene) = item else {
                        panic!("Atlas pages must be scenes");
                    };
                    for fraction in [0.0, 0.18, 0.62, 0.99] {
                        let time =
                            Time::from_seconds(scene.start.unwrap() + scene.duration * fraction);
                        frames.push(time.to_frame(rate));
                    }
                }
                frames.push(actual.frame_count() as i64 - 1);
                frames
            };
            frames.sort_unstable();
            frames.dedup();
            let mut maximum = 0;
            for frame in &frames {
                let time = Time::from_frame(*frame, rate);
                let first = render_still(&actual, time, &options)?;
                let expected = render_still(&reference, time, &options)?;
                assert_eq!(first.bytes().len(), expected.bytes().len());
                let difference = first
                    .bytes()
                    .iter()
                    .zip(expected.bytes())
                    .map(|(first, second)| first.abs_diff(*second))
                    .max()
                    .unwrap();
                maximum = maximum.max(difference);
                assert!(
                    difference <= 1,
                    "seconds {seconds} seed {seed} frame {frame}: full-frame channel difference {difference}"
                );
                assert_eq!(
                    first.bytes(),
                    render_still(&actual, time, &options)?.bytes()
                );
                assert_eq!(
                    expected.bytes(),
                    render_still(&reference, time, &options)?.bytes()
                );
            }
            println!(
                "Atlas parity: seconds={seconds} seed={seed} frames={} size=288x162 full_frame=true max_channel_difference={maximum} replay=exact",
                frames.len()
            );
        }
        Ok(())
    }
}
