//! Worker bridge for the workbench: polls preview replies and export completion into
//! UI state, issues preview requests for the transport frame, and queues export jobs
//! from the export form.

use super::StudioApp;
use crate::AudioOptions;
use crate::preview;
use crate::preview::{ExportJob, JobState, PreviewKey};
use eframe::egui;
use std::path::PathBuf;
use std::time::Instant;

impl StudioApp {
    pub(crate) fn poll_workers(&mut self, context: &egui::Context) {
        self.cached_previews = self.preview.thumbnails();
        if let Some(reply) = self.preview.take_reply()
            && self.preview_state.requested_key() == Some(reply.key)
        {
            self.preview_state.pending = false;
            self.observations.backend_status = reply.backend;
            self.observations.elapsed = reply.elapsed;
            self.observations.frame_stats = reply.stats;
            self.observations.timings = reply.timings;
            self.cached_frames = reply.cached_frames;
            self.inspection.timeline_view.set_cache(&self.cached_frames);
            self.thumbnail = None;
            self.cached_bytes = reply.cached_bytes;
            self.inspection.nodes = reply.nodes;
            if let Some(message) = &self.inspection.viewer.diagnostic_focus {
                self.inspection.viewer.selected = self
                    .inspection
                    .nodes
                    .iter()
                    .enumerate()
                    .filter(|(_, node)| message.contains(&node.path))
                    .max_by_key(|(_, node)| node.path.len())
                    .map(|(index, _)| index);
            }
            self.observations.preview_count += 1;
            self.observations.cache_hits += u64::from(reply.cache_hit);
            if self.playback.transport.anchor.is_some() {
                self.observations.dropped_frames +=
                    self.preview_state.displayed.map_or(0, |previous| {
                        u64::from(
                            reply
                                .key
                                .frame
                                .saturating_sub(previous.frame)
                                .saturating_sub(1),
                        )
                    });
            }
            for diagnostic in &reply.diagnostics {
                self.observations.diagnostics.push_back((
                    reply.key.frame,
                    diagnostic.to_string().chars().take(8192).collect(),
                ));
            }
            while self.observations.diagnostics.len() > 128 {
                self.observations.diagnostics.pop_front();
            }
            match reply.image {
                Ok(image) => {
                    if self.playback.transport.anchor.is_some() {
                        self.observations
                            .presentation_times
                            .push_back(Instant::now());
                        while self.observations.presentation_times.len() > 32 {
                            self.observations.presentation_times.pop_front();
                        }
                    }
                    self.preview_pixels = Some(image.clone());
                    #[cfg(feature = "gpu")]
                    {
                        self.preview_only = None;
                        if let Some(texture) = reply.texture
                            && let Some(register) = &self.register_texture
                        {
                            let id = if let Some((_, id)) = self
                                .registered_previews
                                .iter()
                                .find(|entry| entry.0 == texture.id())
                            {
                                *id
                            } else {
                                let id = register(&texture);
                                self.registered_previews.push_back((texture.id(), id));
                                while self.registered_previews.len()
                                    > crate::preview::MAX_TEXTURE_SLOTS
                                {
                                    if let Some((_, old)) = self.registered_previews.pop_front()
                                        && let Some(free) = &self.free_texture
                                    {
                                        free(old);
                                    }
                                }
                                id
                            };
                            self.preview_only = Some((id, texture));
                        }
                    }
                    #[cfg(feature = "gpu")]
                    let upload = self.preview_only.is_none();
                    #[cfg(not(feature = "gpu"))]
                    let upload = true;
                    if upload {
                        if let Some(texture) = &mut self.texture {
                            texture.set(image, egui::TextureOptions::LINEAR);
                        } else {
                            self.texture = Some(context.load_texture(
                                "scene-preview",
                                image,
                                egui::TextureOptions::LINEAR,
                            ));
                        }
                    }
                    self.preview_state.displayed = Some(reply.key);
                    self.preview_state.error = (!reply.diagnostics.is_empty()).then(|| {
                        reply
                            .diagnostics
                            .iter()
                            .take(4)
                            .map(ToString::to_string)
                            .collect::<Vec<_>>()
                            .join("\n")
                    });
                    if self.preview_state.error.is_some() {
                        self.pause();
                    }
                }
                Err(error) => {
                    if self.observations.diagnostics.len() == 128 {
                        self.observations.diagnostics.pop_front();
                    }
                    self.observations
                        .diagnostics
                        .push_back((reply.key.frame, error.to_string()));
                    self.preview_state.error = Some(error.to_string());
                    self.pause();
                }
            }
        }
        let finished = self
            .exports
            .export
            .as_ref()
            .and_then(|job| match job.state() {
                JobState::Finished(outcome) => Some((job.path.clone(), outcome)),
                _ => None,
            });
        if let Some((path, outcome)) = finished {
            self.exports.export = None;
            let (error, message) = match outcome {
                Ok(()) => (false, format!("Saved {}", path.display())),
                Err(error) if error.kind() == motion_core::ErrorKind::Cancelled => {
                    (false, "Export cancelled".into())
                }
                Err(error) => (true, error.to_string()),
            };
            self.exports.export_message = Some((error, message.clone()));
            if self.exports.completed_exports.len() == 64 {
                self.exports.completed_exports.pop_front();
            }
            self.exports
                .completed_exports
                .push_back((path, error, message));
        }
        if self.exports.export.is_none()
            && !self.close_when_idle
            && let Some(task) = self.exports.export_queue.pop_front()
        {
            match ExportJob::start_with_codec(
                self.scene.clone(),
                task.path.clone(),
                task.draft,
                task.audio,
                context.clone(),
                task.codec,
            ) {
                Ok(job) => self.exports.export = Some(job),
                Err(error) => {
                    if self.exports.completed_exports.len() == 64 {
                        self.exports.completed_exports.pop_front();
                    }
                    self.exports
                        .completed_exports
                        .push_back((task.path, true, error.to_string()));
                }
            }
        }
    }

    pub(crate) fn request_preview(&mut self) {
        let key = PreviewKey {
            frame: self.playback.transport.frame,
            renderer: self.renderer,
        };
        let playing = self.playback.transport.playing();
        let settings = self.preview_settings.for_interaction(
            playing,
            self.playback.scrub_dragging,
            self.playback.last_interaction.elapsed(),
        );
        if !self.preview_state.is_requested(key, settings) {
            if playing {
                self.preview.request_playback(key, settings);
            } else if settings.quality == motion_core::RenderQuality::Final {
                self.preview.request_idle(key, settings);
            } else {
                self.preview.request_with_settings(key, settings);
            }
            self.preview_state.request(key, settings);
        }
        if self.ready_to_start() {
            self.start_clock();
        }
    }

    pub(crate) fn start_export(&mut self, context: &egui::Context) {
        self.pause();
        self.exports.export_message = None;
        if self.exports.export_queue.len() >= 16 {
            self.exports.export_message =
                Some((true, "Export queue is full (16 pending tasks)".into()));
            return;
        }
        let extension = if self.exports.export_png {
            Some("png")
        } else {
            self.exports.export_format.extension()
        };
        let path = PathBuf::from(self.exports.export_destination.trim());
        if self.exports.export_destination.trim().is_empty()
            || extension.is_some_and(|extension| {
                !path
                    .extension()
                    .and_then(|value| value.to_str())
                    .is_some_and(|value| value.eq_ignore_ascii_case(extension))
            })
        {
            self.exports.export_message = Some((
                true,
                format!(
                    "Choose a {} destination",
                    extension.unwrap_or("PNG directory")
                ),
            ));
            return;
        }
        let draft = preview::ExportForm {
            size: self.exports.export_size,
            range: self.exports.export_range,
            backend: self
                .preview_settings
                .backend
                .unwrap_or(motion_render::BackendPreference::Cpu),
            quality: self.exports.export_quality,
            overwrite: self.exports.overwrite,
            motion_blur: self.exports.export_motion_blur,
            png: self.exports.export_png,
            frame: self.playback.transport.frame,
        };
        let audio = if self.exports.export_png {
            Ok(AudioOptions::default())
        } else {
            self.audio.apply(AudioOptions::default())
        };
        match audio {
            Ok(audio) => {
                self.exports
                    .export_queue
                    .push_back(crate::export::QueuedExport {
                        path,
                        draft,
                        audio,
                        codec: self
                            .exports
                            .export_format
                            .codec(draft.quality, self.clock.rate),
                    });
                context.request_repaint();
            }
            Err(error) => self.exports.export_message = Some((true, error.to_string())),
        }
    }
}
