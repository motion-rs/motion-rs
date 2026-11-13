//! Shared loading and parity checks for independently compilable JSON atlas documents.

use crate::DemoParameters;
use motion::declarative::{Document, TrackItem};
use motion::prelude::*;

pub(super) const COMPOSITING_DOCUMENT: &str = include_str!("../../assets/compositing-atlas.json");

pub(super) const FILTER_DOCUMENT: &str = include_str!("../../assets/filter-atlas.json");

pub(super) const VECTOR_DOCUMENT: &str = include_str!("../../assets/vector-atlas.json");

pub(super) const PARTICLE_DOCUMENT: &str = include_str!("../../assets/particle-atlas.json");

pub(super) const PARTICLE_FEATURES_DOCUMENT: &str =
    include_str!("../../assets/particle-features.json");

pub(super) fn timeline(
    source: &str,
    parameters: DemoParameters,
    mut overrides: serde_json::Map<String, serde_json::Value>,
) -> Result<Timeline, motion::ErrorReport> {
    let parameters = parameters.validate()?;
    overrides.insert("seconds".into(), parameters.seconds.into());
    let mut document =
        Document::from_json_with(source, Some(&serde_json::Value::Object(overrides)))?;
    for track in &mut document.tracks {
        track.items.retain(|item| match item {
            TrackItem::Scene(scene) => scene.duration > 0.0,
            TrackItem::Transition(_) | TrackItem::Gap(_) => true,
        });
    }
    Ok(document.compile(&Default::default())?.into_parts().0)
}

#[cfg(test)]
pub(super) mod tests {
    use super::super::{atlas, compositing_atlas, filter_atlas, vector_atlas};
    use super::*;
    use crate::DemoMetadata;
    use atlas::Page;
    use motion::declarative::{Content, TrackItem};
    use serde_json::json;

    /// Declarative pages shared with the native atlas, before the three native-only studies.
    const VECTOR_PAGE_COUNT: usize = 13;

    #[test]
    fn json_filter_atlas_source() -> motion::Result {
        source(FILTER_DOCUMENT, filter_atlas::METADATA, filter_atlas::PAGES)
    }

    #[test]
    fn json_vector_atlas_source() -> motion::Result {
        source(
            VECTOR_DOCUMENT,
            vector_atlas::METADATA,
            &vector_atlas::PAGES[..VECTOR_PAGE_COUNT],
        )
    }

    #[test]
    fn json_compositing_source_and_modes_preserve_metadata_and_duration() -> motion::Result {
        source(
            COMPOSITING_DOCUMENT,
            compositing_atlas::METADATA,
            compositing_atlas::PAGES,
        )
    }

    #[test]
    fn radial_blur_document_matches_core() -> motion::Result {
        let progress = 20.0 / 30.0 * 17.0 - 11.0;
        let mut source: serde_json::Value = serde_json::from_str(FILTER_DOCUMENT).unwrap();
        source["size"] = json!([512, 304]);
        source["tracks"] = json!([{"items":[{"scene":{"duration":1,"content":{"children":[
            {"id":"clock","rotate":progress},
            {"use":"specimen","filters":[{"optics":{"radial_blur":{"center":[220,100],"strength":0.5 * (0.25 + 0.75 * progress),"samples":16,"decay":0.92}}}]}
        ]}}}]}]);
        let actual = Document::from_json(&source.to_string())?
            .compile(&Default::default())?
            .into_parts()
            .0;
        let expected = Timeline::new(Size::new(512.0, 304.0), motion::fps!(30))
            .background(Color::from_hex(atlas::PAPER))
            .track(|track| {
                track.scene(1.seconds(), move |_| -> motion::Result<Node> {
                    Ok(
                        atlas::specimen(progress)?.filter(Filter::Optics(Optics::RadialBlur {
                            center: Point::new(220.0, 100.0),
                            strength: 0.5 * (0.25 + 0.75 * progress),
                            samples: 16,
                            decay: 0.92,
                        })),
                    )
                })
            })
            .build()?;
        let options = RenderOptions::default()
            .backend(BackendPreference::Cpu)
            .size(PixelSize::new(288, 162));
        assert_eq!(
            render_still(&actual, Time::ZERO, &options)?.bytes(),
            render_still(&expected, Time::ZERO, &options)?.bytes()
        );
        Ok(())
    }

    #[test]
    #[ignore = "requires explicit MOTION_TEST_FONT; compares complete native and JSON CPU frames"]
    fn json_filter_atlas_frames() -> motion::Result {
        frames(filter_atlas::METADATA, filter_atlas::PAGES)
    }

    #[test]
    #[ignore = "requires explicit MOTION_TEST_FONT; compares complete native and JSON CPU frames"]
    fn json_vector_atlas_frames() -> motion::Result {
        frames(
            vector_atlas::METADATA,
            &vector_atlas::PAGES[..VECTOR_PAGE_COUNT],
        )
    }

    #[test]
    #[ignore = "requires explicit MOTION_TEST_FONT; compares complete native and JSON CPU frames"]
    fn json_compositing_frames_match_native_and_replay() -> motion::Result {
        frames(compositing_atlas::METADATA, compositing_atlas::PAGES)
    }

    pub(in super::super) fn source(
        source: &str,
        metadata: DemoMetadata,
        pages: &[Page],
    ) -> motion::Result {
        let document = Document::from_json(source)?;
        assert_eq!(document.name.as_deref(), Some(metadata.title));
        assert_eq!(document.params["pages"], pages.len());
        assert_eq!(
            document
                .compile(&Default::default())?
                .timeline()
                .frame_count(),
            240
        );
        assert_eq!(
            Document::from_json(&serde_json::to_string(&document).unwrap())?,
            document
        );
        let single_scene = metadata.id == compositing_atlas::METADATA.id;
        assert_eq!(
            document.tracks[0].items.len(),
            if single_scene { 1 } else { pages.len() }
        );
        let mut titles = Vec::new();
        for item in &document.tracks[0].items {
            let TrackItem::Scene(scene) = item else {
                panic!("Atlas content must be a scene");
            };
            let Content::Group(children) = &scene.content.content else {
                panic!("Atlas content must be a group");
            };
            let captions = children
                .iter()
                .find(|element| element.name.as_deref() == Some("Page captions"))
                .unwrap();
            let Content::Group(captions) = &captions.content else {
                panic!("Atlas captions must be a group");
            };
            assert_eq!(captions.len(), if single_scene { pages.len() } else { 1 });
            titles.extend(captions.iter().map(|caption| caption.name.as_deref()));
        }
        assert_eq!(
            titles,
            pages
                .iter()
                .map(|page| Some(page.title))
                .collect::<Vec<_>>()
        );
        let project = crate::project(RenderOptions::default())?;
        assert_eq!(
            crate::catalog()
                .iter()
                .filter(|entry| entry.id == metadata.id)
                .count(),
            1
        );
        for seconds in [1, 8, 60] {
            for mode in ["default", "json"] {
                let timeline = project.build(
                    metadata.id,
                    Some(json!({"seconds": seconds, "seed": u64::MAX, "mode": mode})),
                )?;
                assert_eq!(timeline.duration(), seconds.seconds());
                assert_eq!(timeline.frame_count(), seconds as u64 * 30);
            }
        }
        for seconds in [0, 61] {
            assert!(
                project
                    .build(
                        metadata.id,
                        Some(json!({"seconds": seconds, "mode": "json"}))
                    )
                    .is_err()
            );
        }
        Ok(())
    }

    pub(in super::super) fn frames(metadata: DemoMetadata, pages: &[Page]) -> motion::Result {
        let font = std::env::var_os("MOTION_TEST_FONT")
            .ok_or_else(|| std::io::Error::other("Set MOTION_TEST_FONT"))?;
        let options = crate::render_options(Some(std::path::Path::new(&font)))?
            .backend(BackendPreference::Cpu);
        let project = crate::project(options.clone())?;
        for (seconds, seed, width, height) in [
            (1, 17, 288, 162),
            (8, 20261027, 288, 162),
            (60, u64::MAX, 288, 162),
            (60, 17, 960, 540),
        ] {
            let options = options.clone().size(PixelSize::new(width, height));
            let actual = project.build(
                metadata.id,
                Some(json!({"seconds": seconds, "seed": seed, "mode": "json"})),
            )?;
            let expected = if metadata.id == vector_atlas::METADATA.id {
                vector_atlas::timeline_pages(
                    DemoParameters {
                        seconds,
                        seed,
                        ..DemoParameters::default()
                    },
                    &vector_atlas::PAGES[..VECTOR_PAGE_COUNT],
                )?
            } else {
                project.build(
                    metadata.id,
                    Some(json!({"seconds": seconds, "seed": seed, "mode": "default"})),
                )?
            };
            let rate = motion::fps!(30);
            let mut frames = if seconds <= 8 {
                (0..actual.frame_count() as i64).collect::<Vec<_>>()
            } else {
                (0..pages.len())
                    .flat_map(|page| {
                        let fractions: &[f64] = if metadata.id == compositing_atlas::METADATA.id {
                            if width == 960 && page == 24 {
                                &[0.49, 0.5, 0.51]
                            } else if width == 960 {
                                &[0.5]
                            } else {
                                &[0.0, 0.18, 0.49, 0.5, 0.51, 0.62, 0.99]
                            }
                        } else if width == 960 {
                            &[0.5]
                        } else {
                            &[0.0, 0.18, 0.62, 0.99]
                        };
                        fractions.iter().map(move |fraction| {
                            Time::from_seconds(
                                (page as f64 + fraction) / pages.len() as f64 * seconds as f64,
                            )
                            .to_frame(rate)
                        })
                    })
                    .collect()
            };
            frames.push(actual.frame_count() as i64 - 1);
            frames.sort_unstable();
            frames.dedup();
            let mut maximum = 0;
            for frame in &frames {
                let time = Time::from_frame(*frame, rate);
                let first = render_still(&actual, time, &options)?;
                let reference = render_still(&expected, time, &options)?;
                assert_eq!(first.bytes().len(), reference.bytes().len());
                let difference = first
                    .bytes()
                    .iter()
                    .zip(reference.bytes())
                    .map(|(first, reference)| first.abs_diff(*reference))
                    .max()
                    .unwrap();
                maximum = maximum.max(difference);
                if difference > 1
                    && let Some(directory) = std::env::var_os("MOTION_ATLAS_DIFF_DIR")
                {
                    let directory = std::path::Path::new(&directory);
                    first.save_png(
                        directory.join(format!("{}-{seconds}-{frame}-json.png", metadata.id)),
                    )?;
                    reference.save_png(
                        directory.join(format!("{}-{seconds}-{frame}-native.png", metadata.id)),
                    )?;
                }
                assert!(
                    difference <= 1,
                    "{} seconds={seconds} seed={seed} size={width}x{height} frame={frame} full-frame channel difference={difference}",
                    metadata.id
                );
                assert_eq!(
                    first.bytes(),
                    render_still(&actual, time, &options)?.bytes()
                );
            }
            println!(
                "{} parity: seconds={seconds} seed={seed} size={width}x{height} frames={} max_channel_difference={maximum} replay=exact",
                metadata.id,
                frames.len()
            );
        }
        Ok(())
    }
}
