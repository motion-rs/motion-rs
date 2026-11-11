//! Background audio analysis for the timeline: a worker thread mixes the scene's audio
//! timeline once, then publishes waveform peaks, loudness, onsets, beat grid and bus
//! meters into a shared slot the timeline panel polls while painting.

use eframe::egui;
use motion_core::sync::MutexExt;
use std::sync::{Arc, Mutex};

#[derive(Default)]
pub(super) struct AudioOverview {
    pub(super) peaks: Vec<f32>,
    pub(super) markers: Vec<motion_core::Duration>,
    pub(super) beat: Option<motion_encode::audio::analysis::BeatGrid>,
    pub(super) report: Option<motion_encode::audio::LoudnessReport>,
    pub(super) analysis_error: Option<String>,
    pub(super) curve: Vec<f32>,
    pub(super) meters: Vec<motion_encode::audio::pcm::BusMeter>,
}

pub(super) type Waveform = Arc<Mutex<Option<Result<AudioOverview, String>>>>;

/// Starts the `motion-waveform` worker that fills `waveform_result` with the analysis of
/// `timeline` and repaints `context` when it finishes. A failed spawn stores the error.
pub(super) fn spawn_analysis(
    waveform_result: &Waveform,
    timeline: Arc<motion_core::Timeline>,
    context: &egui::Context,
) {
    let result = waveform_result.clone();
    let context = context.clone();
    let spawned = std::thread::Builder::new()
        .name("motion-waveform".into())
        .spawn(move || {
            let sampled = std::panic::catch_unwind(std::panic::AssertUnwindSafe(
                || -> motion_core::Result<AudioOverview> {
                    if timeline.soundtrack().clips().is_empty() {
                        return Ok(AudioOverview::default());
                    }
                    let mixed = motion_encode::MixedAudio::new(
                        &timeline,
                        motion_encode::MixOptions::new(8000),
                    )?;
                    let peaks = motion_encode::audio::WaveformPyramid::from_source(
                        &mixed,
                        timeline.duration(),
                    )?
                    .peaks(4096)?;
                    let mut overview = AudioOverview {
                        peaks,
                        ..Default::default()
                    };
                    overview.meters = mixed.bus_meters().to_vec();
                    let analyzed = (|| -> motion_core::Result {
                        let analysis =
                            motion_encode::AudioAnalysis::from_source(&mixed, timeline.duration())?;
                        overview.curve = analysis
                            .loudness()?
                            .values()
                            .iter()
                            .map(|value| *value as f32)
                            .collect();
                        overview.markers = analysis.onsets()?.times().to_vec();
                        overview.beat = analysis.beat_grid().ok();
                        overview.report = Some(
                            motion_encode::MixedAudio::new(
                                &timeline,
                                motion_encode::MixOptions::new(48000),
                            )?
                            .loudness_report()?,
                        );
                        Ok(())
                    })();
                    overview.analysis_error = analyzed.err().map(|error| error.to_string());
                    Ok(overview)
                },
            ))
            .unwrap_or_else(|payload| Err(motion_core::Error::scene_panic("waveform", 0, payload)))
            .map_err(|error| error.to_string());
            *result.lock_unpoisoned() = Some(sampled);
            context.request_repaint();
        });
    if let Err(error) = spawned {
        *waveform_result.lock_unpoisoned() = Some(Err(error.to_string()));
    }
}
