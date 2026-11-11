//! Per-scene audio export drafts, independent of preview state and running jobs.

use std::io;
use std::path::PathBuf;
use std::time::Duration;

use eframe::egui;
use egui_phosphor::regular as icons;

use crate::app::tool;
use motion_core::AudioClip;

#[derive(Clone, Debug, Default, PartialEq, serde::Serialize, serde::Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct AudioForm {
    pub enabled: bool,
    pub clips: Vec<ClipSettings>,
}

#[derive(Clone, Debug, PartialEq, serde::Serialize, serde::Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct ClipSettings {
    #[serde(with = "crate::audio::file::source_path")]
    pub path: PathBuf,
    pub enabled: bool,
    pub source_in: f64,
    pub source_out: f64,
    pub start: f64,
    pub volume: f32,
    pub fade_in: f64,
    pub fade_out: f64,
}

impl ClipSettings {
    pub fn new(path: PathBuf, duration: f64) -> Self {
        Self {
            path,
            enabled: true,
            source_in: 0.0,
            source_out: duration.min(motion_encode::audio::MAX_SECONDS as f64),
            start: 0.0,
            volume: 1.0,
            fade_in: 0.0,
            fade_out: 0.0,
        }
    }

    fn clip(&self) -> io::Result<AudioClip> {
        let time = |value: f64| {
            if !value.is_finite()
                || !(0.0..=motion_encode::audio::MAX_SECONDS as f64).contains(&value)
            {
                return Err(io::Error::new(
                    io::ErrorKind::InvalidInput,
                    "Audio seconds must be finite and in 0..=86400",
                ));
            }
            let duration = Duration::try_from_secs_f64(value)
                .map_err(|error| io::Error::new(io::ErrorKind::InvalidInput, error))?;
            Ok(motion_core::Duration::from_seconds(duration.as_secs_f64()))
        };
        let path = self.path.canonicalize()?;
        if !path.is_file() {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "Audio source must be a regular file",
            ));
        }
        let offset = time(self.source_in)?;
        let end = time(self.source_out)?;
        if motion_core::audio::frames(offset, 48_000) >= motion_core::audio::frames(end, 48_000) {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "Audio interval must contain at least one sample",
            ));
        }
        Ok(AudioClip::file(path)
            .offset(offset)
            .duration(end - offset)
            .start(time(self.start)?.after_origin())
            .volume(f64::from(self.volume))
            .fade_in(time(self.fade_in)?)
            .fade_out(time(self.fade_out)?))
    }
}

impl AudioForm {
    pub fn show(&mut self, ui: &mut egui::Ui, duration: f64) -> Option<(bool, String)> {
        let mut message = None;
        ui.horizontal(|ui| {
            ui.checkbox(&mut self.enabled, "Audio");
            ui.weak(format!(
                "{}/{}",
                self.clips.len(),
                motion_encode::audio::MAX_CLIPS
            ));
            ui.add_enabled_ui(self.clips.len() < motion_encode::audio::MAX_CLIPS, |ui| {
                if tool(ui, icons::PLUS, "Add audio clip")
                    && let Some(path) = choose_file()
                {
                    self.clips.push(ClipSettings::new(path, duration));
                    self.enabled = true;
                }
            });
            ui.add_enabled_ui(!self.clips.is_empty(), |ui| {
                if tool(ui, icons::TRASH, "Remove all audio clips") {
                    self.clips.clear();
                }
            });
        });
        ui.horizontal(|ui| {
            ui.label("Configuration");
            if tool(ui, icons::FLOPPY_DISK, "Save audio configuration")
                && let Some(path) = rfd::FileDialog::new()
                    .set_title("Save audio configuration")
                    .add_filter("Motion audio configuration", &["json"])
                    .set_file_name("audio.motion.json")
                    .save_file()
            {
                message = Some(match self.save(&path) {
                    Ok(()) => (
                        false,
                        format!("Saved audio configuration: {}", path.display()),
                    ),
                    Err(error) => (true, format!("Cannot save audio configuration: {error}")),
                });
            }
            if tool(ui, icons::FOLDER_OPEN, "Load audio configuration")
                && let Some(path) = rfd::FileDialog::new()
                    .set_title("Load audio configuration")
                    .add_filter("Motion audio configuration", &["json"])
                    .pick_file()
            {
                message = Some(match self.load(&path) {
                    Ok(()) => (
                        false,
                        format!("Loaded audio configuration: {}", path.display()),
                    ),
                    Err(error) => (true, format!("Cannot load audio configuration: {error}")),
                });
            }
        });
        let mut remove = None;
        for (index, clip) in self.clips.iter_mut().enumerate() {
            ui.push_id(index, |ui| {
                ui.horizontal(|ui| {
                    ui.checkbox(&mut clip.enabled, format!("{:02}", index + 1));
                    let name = clip
                        .path
                        .file_name()
                        .unwrap_or(clip.path.as_os_str())
                        .to_string_lossy();
                    let width =
                        (ui.available_width() - 60.0 - 2.0 * ui.spacing().item_spacing.x).max(1.0);
                    ui.add_sized([width, 30.0], egui::Label::new(name.as_ref()).truncate())
                        .on_hover_text(clip.path.display().to_string());
                    if tool(ui, icons::FOLDER_OPEN, "Replace audio source")
                        && let Some(path) = choose_file()
                    {
                        clip.path = path;
                    }
                    if tool(ui, icons::TRASH, "Remove audio clip") {
                        remove = Some(index);
                    }
                });
                ui.add_enabled_ui(self.enabled && clip.enabled, |ui| {
                    egui::Grid::new("audio-fields")
                        .num_columns(2)
                        .spacing([8.0, 5.0])
                        .show(ui, |ui| {
                            seconds(ui, "Source in", &mut clip.source_in);
                            seconds(ui, "Source out", &mut clip.source_out);
                            seconds(ui, "Start", &mut clip.start);
                            ui.label("Volume");
                            ui.add(
                                egui::DragValue::new(&mut clip.volume)
                                    .range(0.0..=16.0)
                                    .speed(0.01)
                                    .min_decimals(2)
                                    .max_decimals(3),
                            );
                            ui.end_row();
                            seconds(ui, "Fade in", &mut clip.fade_in);
                            seconds(ui, "Fade out", &mut clip.fade_out);
                        });
                });
                ui.add_space(6.0);
                ui.separator();
            });
        }
        if let Some(index) = remove {
            self.clips.remove(index);
        }
        message
    }

    pub fn apply(&self, options: AudioOptions) -> io::Result<AudioOptions> {
        if !self.enabled {
            return options.audio([]);
        }
        if self.clips.len() > motion_encode::audio::MAX_CLIPS {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                format!(
                    "At most {} audio drafts are allowed",
                    motion_encode::audio::MAX_CLIPS
                ),
            ));
        }
        let mut clips = Vec::new();
        for (index, settings) in self
            .clips
            .iter()
            .enumerate()
            .filter(|(_, clip)| clip.enabled)
        {
            let result = settings.clip().and_then(|clip| {
                clip.resolved_duration().map_err(io::Error::other)?;
                Ok(clip)
            });
            clips.push(result.map_err(|error| {
                io::Error::new(
                    error.kind(),
                    format!(
                        "Audio clip {} ({}): {error}",
                        index + 1,
                        settings.path.display()
                    ),
                )
            })?);
        }
        options.audio(clips)
    }
}

fn choose_file() -> Option<PathBuf> {
    rfd::FileDialog::new()
        .add_filter(
            "Audio",
            &["wav", "mp3", "aac", "m4a", "flac", "ogg", "aiff", "aif"],
        )
        .add_filter("All files", &["*"])
        .pick_file()
}

fn seconds(ui: &mut egui::Ui, label: &str, value: &mut f64) {
    ui.label(label);
    ui.add(
        egui::DragValue::new(value)
            .range(0.0..=motion_encode::audio::MAX_SECONDS as f64)
            .speed(0.01)
            .min_decimals(3)
            .max_decimals(6)
            .suffix(" s"),
    );
    ui.end_row();
}

#[derive(Clone, Debug, Default)]
pub(crate) struct AudioOptions {
    clips: Vec<AudioClip>,
    #[cfg(test)]
    overwrite: bool,
}

impl AudioOptions {
    pub(crate) fn audio(mut self, clips: impl IntoIterator<Item = AudioClip>) -> io::Result<Self> {
        let mut values = Vec::new();
        for clip in clips {
            if values.len() >= motion_encode::audio::MAX_CLIPS {
                return Err(io::Error::new(
                    io::ErrorKind::InvalidInput,
                    format!(
                        "At most {} audio clips are allowed",
                        motion_encode::audio::MAX_CLIPS
                    ),
                ));
            }
            clip.resolved_duration().map_err(io::Error::other)?;
            values.push(clip);
        }
        self.clips = values;
        Ok(self)
    }

    pub(crate) fn audio_clips(&self) -> &[AudioClip] {
        &self.clips
    }

    #[cfg(test)]
    pub(crate) fn quality(&self) -> u8 {
        18
    }

    #[cfg(test)]
    pub(crate) fn overwrite(mut self, enabled: bool) -> Self {
        self.overwrite = enabled;
        self
    }

    #[cfg(test)]
    pub(crate) fn overwrite_enabled(&self) -> bool {
        self.overwrite
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn audio_form_remains_bounded_at_output_panel_widths_with_long_filenames() {
        for width in [240.0, 276.0, 360.0] {
            let context = egui::Context::default();
            let mut settings = AudioForm {
                enabled: true,
                clips: vec![
                    ClipSettings::new(
                        PathBuf::from(
                            "a-very-long-audio-source-name-that-must-not-widen-the-output-panel.wav"
                        ),
                        86400.0
                    );
                    2
                ],
            };
            let mut output = context.run_ui(
                egui::RawInput {
                    screen_rect: Some(egui::Rect::from_min_size(
                        egui::Pos2::ZERO,
                        egui::vec2(width, 620.0),
                    )),
                    ..Default::default()
                },
                |ui| {
                    egui::CentralPanel::default().show(ui, |ui| {
                        egui::ScrollArea::vertical().show(ui, |ui| {
                            let available = ui.available_width();
                            let message = settings.show(ui, 86400.0);
                            assert!(message.is_none());
                            assert!(ui.min_rect().width() <= available + 1.0);
                        });
                    });
                },
            );
            output.textures_delta.clear();
        }
    }

    #[test]
    fn disabled_audio_and_disabled_rows_do_not_read_missing_files_or_invalid_drafts()
    -> io::Result<()> {
        let mut settings = AudioForm::default();
        let mut missing = ClipSettings::new(PathBuf::from("no-such-source.wav"), 2.0);
        missing.source_in = f64::NAN;
        settings.clips.push(missing);
        assert!(
            settings
                .apply(AudioOptions::default())?
                .audio_clips()
                .is_empty()
        );
        settings.enabled = true;
        settings.clips[0].enabled = false;
        assert!(
            settings
                .apply(AudioOptions::default())?
                .audio_clips()
                .is_empty()
        );
        Ok(())
    }

    #[test]
    fn exported_options_snapshot_validated_values_and_scene_drafts_are_independent()
    -> io::Result<()> {
        let file = tempfile::NamedTempFile::new()?;
        let mut scenes = vec![AudioForm::default(); 2];
        scenes[0].enabled = true;
        let mut clip = ClipSettings::new(file.path().into(), 2.0);
        clip.source_in = 0.25;
        clip.start = 1.0;
        clip.volume = 0.5;
        clip.fade_in = 0.1;
        clip.fade_out = 0.2;
        scenes[0].clips.push(clip);
        let options = scenes[0].apply(AudioOptions::default())?;
        scenes[0].clips[0].start = 8.0;
        let updated = scenes[0].apply(AudioOptions::default())?;
        assert_eq!(
            updated.audio_clips()[0].parameters().start.as_seconds(),
            8.0
        );
        scenes[0].clips.clear();
        scenes[0].enabled = false;
        assert_eq!(
            options.audio_clips()[0].parameters().offset.as_seconds(),
            0.25
        );
        assert_eq!(
            options.audio_clips()[0]
                .resolved_duration()
                .unwrap()
                .as_seconds(),
            1.75
        );
        assert_eq!(
            options.audio_clips()[0].parameters().start.as_seconds(),
            1.0
        );
        assert!(scenes[1].apply(options.clone())?.audio_clips().is_empty());
        assert_eq!(options.audio_clips().len(), 1);
        Ok(())
    }

    #[test]
    fn invalid_draft_values_report_the_row_and_bounded_list_is_enforced() -> io::Result<()> {
        let file = tempfile::NamedTempFile::new()?;
        let base = ClipSettings::new(file.path().into(), 2.0);
        let mut invalids = Vec::new();
        for seconds in [f64::NAN, f64::INFINITY, -1.0, 86401.0] {
            for field in 0..5 {
                let mut clip = base.clone();
                match field {
                    0 => clip.source_in = seconds,
                    1 => clip.source_out = seconds,
                    2 => clip.start = seconds,
                    3 => clip.fade_in = seconds,
                    _ => clip.fade_out = seconds,
                }
                invalids.push(clip);
            }
        }
        let mut reversed = base.clone();
        reversed.source_in = 3.0;
        invalids.push(reversed);
        let mut fade = base.clone();
        fade.fade_out = 3.0;
        invalids.push(fade);
        let mut volume = base.clone();
        volume.volume = f32::NAN;
        invalids.push(volume);
        let mut endpoint = base.clone();
        endpoint.start = 86400.0;
        invalids.push(endpoint);
        for clip in invalids {
            let settings = AudioForm {
                enabled: true,
                clips: vec![clip],
            };
            assert!(
                settings
                    .apply(AudioOptions::default())
                    .unwrap_err()
                    .to_string()
                    .starts_with("Audio clip 1 (")
            );
        }
        let mut settings = AudioForm {
            enabled: true,
            clips: vec![base; motion_encode::audio::MAX_CLIPS],
        };
        assert_eq!(
            settings.apply(AudioOptions::default())?.audio_clips().len(),
            motion_encode::audio::MAX_CLIPS
        );
        settings.clips.push(settings.clips[0].clone());
        assert!(settings.apply(AudioOptions::default()).is_err());
        Ok(())
    }

    #[cfg(unix)]
    #[test]
    fn non_utf8_source_paths_are_preserved_or_report_filesystem_rejection() -> io::Result<()> {
        use std::ffi::OsString;
        use std::os::unix::ffi::OsStringExt;

        let directory = tempfile::tempdir()?;
        let path = directory
            .path()
            .join(OsString::from_vec(b"voice-\xff.wav".to_vec()));
        let creation = std::fs::write(&path, []);
        let settings = AudioForm {
            enabled: true,
            clips: vec![ClipSettings::new(path.clone(), 1.0)],
        };
        match creation {
            Ok(()) => {
                let options = settings.apply(AudioOptions::default())?;
                assert_eq!(
                    options.audio_clips()[0]
                        .parameters()
                        .source
                        .file_path()
                        .unwrap(),
                    path.canonicalize()?.as_path()
                );
            }
            #[cfg(target_os = "macos")]
            Err(error) if error.raw_os_error() == Some(92) => {
                let error = settings.apply(AudioOptions::default()).unwrap_err();
                assert!(error.to_string().starts_with("Audio clip 1 ("));
                assert_eq!(settings.clips[0].path, path);
            }
            Err(error) => return Err(error),
        }
        Ok(())
    }
}
