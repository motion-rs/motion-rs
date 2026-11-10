//! Playback clock and transport state, and the studio logic that advances frames and keeps
//! audio preview in sync.

use crate::app::{SceneDefinition, StudioApp};
use crate::audio::preview::{self as playback, AudioStatus};
use eframe::egui;
use motion_core::Result;
use motion_render::Composition;
use std::time::Instant;

#[derive(Clone, Copy)]
pub(crate) struct StudioClock {
    pub(crate) size: motion_core::PixelSize,
    pub(crate) rate: motion_core::FrameRate,
    pub(crate) frames: u32,
}

impl StudioClock {
    pub(crate) fn new(composition: &Composition) -> Result<Self> {
        Ok(Self {
            size: composition.output_size(),
            rate: composition.frame_rate(),
            frames: u32::try_from(composition.frame_count())
                .map_err(|error| motion_core::Error::Parameters(Box::new(error)))?,
        })
    }

    pub(crate) fn width(self) -> usize {
        self.size.width as usize
    }

    pub(crate) fn height(self) -> usize {
        self.size.height as usize
    }

    pub(crate) fn frame_count(self) -> u32 {
        self.frames
    }

    pub(crate) fn fps(self) -> f64 {
        f64::from(self.rate.numerator()) / f64::from(self.rate.denominator())
    }
}

/// Longest wait for the first frame before Play starts the clock regardless.
const START_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(2);

#[derive(Default)]
pub(crate) struct Transport {
    pub(crate) frame: u32,
    pub(crate) anchor: Option<(Instant, u32)>,
    /// Play was pressed and the clock waits for the first frame; holds when Play was pressed.
    pub(crate) starting: Option<Instant>,
    pub(crate) looping: bool,
}

impl Transport {
    /// Playing, or about to start once the first frame is on screen.
    pub(crate) fn playing(&self) -> bool {
        self.anchor.is_some() || self.starting.is_some()
    }

    #[cfg(test)]
    pub(crate) fn advance_rate(&mut self, now: Instant, rate: motion_core::FrameRate, total: u32) {
        self.advance_range(now, rate, (0, total));
    }

    pub(crate) fn advance_range(
        &mut self,
        now: Instant,
        rate: motion_core::FrameRate,
        range: (u32, u32),
    ) {
        let (start, end) = range;
        if let Some((started, first)) = self.anchor {
            let elapsed = now.saturating_duration_since(started).as_nanos();
            let step = elapsed.saturating_mul(u128::from(rate.numerator()))
                / (1_000_000_000 * u128::from(rate.denominator()));
            let position = u128::from(first).saturating_add(step);
            if position >= u128::from(end) && !self.looping {
                self.frame = end - 1;
                self.anchor = None;
            } else {
                self.frame = start
                    + ((position.saturating_sub(u128::from(start))) % u128::from(end - start))
                        as u32;
            }
        }
    }

    pub(crate) fn seek(&mut self, frame: u32, total: u32) {
        self.frame = frame.min(total - 1);
        self.anchor = None;
    }

    pub(crate) fn toggle(&mut self, now: Instant, total: u32) {
        if self.anchor.take().is_none() {
            if self.frame == total - 1 {
                self.frame = 0;
            }
            self.anchor = Some((now, self.frame));
        }
    }
}

impl StudioApp {
    pub(crate) fn seek(&mut self, frame: u32) {
        self.pause();
        self.playback.last_interaction = Instant::now();
        self.playback.audio_error = None;
        self.playback
            .transport
            .seek(frame, self.clock.frame_count());
    }

    pub(crate) fn pause(&mut self) {
        if self.playback.transport.playing() {
            self.playback.last_interaction = Instant::now();
        }
        self.playback.transport.anchor = None;
        self.playback.transport.starting = None;
        self.preview.stop_prefetch();
        self.audio_preview.stop();
    }

    pub(crate) fn start_audio(&mut self) {
        self.playback.audio_error = None;
        if self.playback.preview_muted {
            self.audio_preview.stop();
            return;
        }
        let result = match &self.scene.definition {
            SceneDefinition {
                audio_timeline: Some(timeline),
                ..
            } => self.audio_preview.start_timeline(
                &self.audio,
                timeline.clone(),
                self.playback.transport.frame,
            ),
            SceneDefinition { .. } => self.audio_preview.start_at_rate(
                &self.audio,
                self.clock.rate,
                self.clock.frame_count(),
                self.playback.transport.frame,
            ),
        };
        if let Err(error) = result {
            self.playback.audio_error = Some(error.to_string());
            self.pause();
        }
    }

    pub(crate) fn toggle_playback(&mut self) {
        if self.exports.export.is_some()
            || !self.exports.export_queue.is_empty()
            || self.close_when_idle
        {
            return;
        }
        if self.playback.transport.playing() {
            self.pause();
        } else {
            self.observations.presentation_times.clear();
            let transport = &mut self.playback.transport;
            transport.toggle(Instant::now(), self.clock.frame_count());
            if transport.frame < self.exports.export_range.0
                || transport.frame >= self.exports.export_range.1 - 1
            {
                transport.frame = self.exports.export_range.0;
            }
            // Starting the clock right away would drop every frame rendered while the first one
            // and its neighbors are still in flight; request_preview starts it once that frame
            // is shown. The neighbors render in parallel and finish with it.
            transport.anchor = None;
            transport.starting = Some(Instant::now());
        }
    }

    /// Starts the clock and sound at the current frame once Play's first frame is ready.
    pub(crate) fn start_clock(&mut self) {
        let transport = &mut self.playback.transport;
        if transport.starting.take().is_some() {
            transport.anchor = Some((Instant::now(), transport.frame));
            self.start_audio();
        }
    }

    /// Whether a pending Play should start: its first frame was answered, or it took too long.
    pub(crate) fn ready_to_start(&self) -> bool {
        self.playback
            .transport
            .starting
            .is_some_and(|since| !self.preview_state.pending || since.elapsed() >= START_TIMEOUT)
    }

    pub(crate) fn advance_transport(&mut self) {
        let frame = self.clock;
        if self.audio_preview.status().is_none() {
            self.playback.transport.advance_range(
                Instant::now(),
                self.scene.composition.frame_rate(),
                self.exports.export_range,
            );
            return;
        }
        if let Some(status) = self.audio_preview.status() {
            match status {
                AudioStatus::Preparing => {}
                AudioStatus::Playing(samples) => {
                    let rate = self.clock.rate;
                    self.playback.transport.frame =
                        playback::frame_at_sample(samples, rate).min(frame.frame_count() - 1);
                    if playback::frame_at_sample(samples, rate) >= self.exports.export_range.1 {
                        if self.playback.transport.looping {
                            self.playback.transport.frame = self.exports.export_range.0;
                            self.start_audio();
                        } else {
                            self.playback.transport.frame = self.exports.export_range.1 - 1;
                            self.pause();
                        }
                    }
                }
                AudioStatus::Finished if self.playback.transport.looping => {
                    self.playback.transport.frame = self.exports.export_range.0;
                    self.start_audio();
                }
                AudioStatus::Finished => {
                    self.playback.transport.frame = frame.frame_count() - 1;
                    self.pause();
                }
                AudioStatus::Failed(error) => {
                    self.playback.audio_error = Some(error);
                    self.pause();
                }
            }
        }
    }

    pub(crate) fn shortcuts(&mut self, context: &egui::Context) {
        if context.egui_wants_keyboard_input() {
            return;
        }
        let total = self.clock.frame_count();
        context.input(|input| {
            let step = if input.modifiers.shift { 10 } else { 1 };
            if input.key_pressed(egui::Key::Space) && !input.pointer.primary_down() {
                self.toggle_playback();
            }
            if input.key_pressed(egui::Key::L) {
                self.playback.transport.looping = !self.playback.transport.looping;
            }
            if input.key_pressed(egui::Key::ArrowLeft) {
                self.seek(self.playback.transport.frame.saturating_sub(step));
            }
            if input.key_pressed(egui::Key::ArrowRight) {
                self.seek(self.playback.transport.frame.saturating_add(step));
            }
            if input.key_pressed(egui::Key::Home) {
                self.seek(0);
            }
            if input.key_pressed(egui::Key::End) {
                self.seek(total - 1);
            }
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn range_clock_holds_exclusive_end_and_loops_inside_in_out() {
        let now = Instant::now();
        let rate = motion_core::FrameRate::from(30);
        let mut transport = Transport {
            frame: 20,
            anchor: Some((now, 20)),
            ..Default::default()
        };
        transport.advance_range(now + std::time::Duration::from_secs(1), rate, (20, 40));
        assert_eq!(transport.frame, 39);
        assert!(transport.anchor.is_none());
        transport.anchor = Some((now, 20));
        transport.looping = true;
        transport.advance_range(now + std::time::Duration::from_secs(1), rate, (20, 40));
        assert_eq!(transport.frame, 30);
        transport.anchor = Some((now, 20));
        transport.advance_range(now + std::time::Duration::from_secs(1), rate, (20, 21));
        assert_eq!(transport.frame, 20);
    }
}
