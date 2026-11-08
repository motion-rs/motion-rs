//! Persists and restores studio UI state in `.motion/studio.json`.

use crate::app::StudioApp;
use std::io;

impl StudioApp {
    pub(crate) fn restore_session(&mut self) {
        let path = std::path::Path::new(".motion/studio.json");
        if let Some(value) = read_session(path) {
            self.apply_session(&value);
        }
    }

    pub(crate) fn apply_session(&mut self, value: &serde_json::Value) {
        if value["version"] == 1 {
            if value["scene"].as_str() == Some(self.scene.name.as_str()) {
                self.playback.transport.seek(
                    value["frame"]
                        .as_u64()
                        .unwrap_or(0)
                        .min(u64::from(u32::MAX)) as u32,
                    self.clock.frame_count(),
                );
                if let Some(range) = value["range"].as_array().filter(|range| range.len() == 2) {
                    let start = range[0].as_u64().unwrap_or(0);
                    let end = range[1].as_u64().unwrap_or(u64::from(self.clock.frames));
                    if start < end && end <= u64::from(self.clock.frames) {
                        self.exports.export_range = (start as u32, end as u32);
                    }
                }
                if let Some(cues) = value["cues"].as_array() {
                    self.inspection.timeline_view.cues = cues
                        .iter()
                        .take(256)
                        .filter_map(|cue| {
                            let frame = cue[0].as_u64()?;
                            let name = cue[1].as_str()?;
                            (frame < u64::from(self.clock.frames) && name.chars().count() <= 128)
                                .then(|| (frame as u32, name.into()))
                        })
                        .collect();
                }
            }
            if let Some(scale) = value["scale"]
                .as_u64()
                .filter(|scale| [25, 50, 100].contains(scale))
            {
                self.preview_settings.scale = scale as u32;
            }
            self.preview_settings.transparent = value["transparent"].as_bool().unwrap_or(true);
            self.inspection.viewer.background = if self.preview_settings.transparent {
                0
            } else {
                3
            };
        }
    }

    pub(crate) fn save_session(&self) {
        let value = serde_json::json!({"version":1,"scene":self.scene.name,"frame":self.playback.transport.frame,"scale":self.preview_settings.scale,"transparent":self.preview_settings.transparent,"range":self.exports.export_range,"cues":self.inspection.timeline_view.cues});
        let result = write_session(std::path::Path::new(".motion/studio.json"), &value);
        if let Err(error) = result {
            eprintln!("Studio session save failed: {error}");
        }
    }
}

pub(crate) fn read_session(path: &std::path::Path) -> Option<serde_json::Value> {
    use std::io::Read;
    let mut bytes = Vec::new();
    std::fs::File::open(path)
        .ok()?
        .take(65_537)
        .read_to_end(&mut bytes)
        .ok()?;
    if bytes.len() > 65_536 {
        return None;
    }
    let value: serde_json::Value = serde_json::from_slice(&bytes).ok()?;
    (value["version"] == 1).then_some(value)
}

pub(crate) fn write_session(path: &std::path::Path, value: &serde_json::Value) -> io::Result<()> {
    use std::io::Write;
    let bytes = serde_json::to_vec(value)?;
    if bytes.len() > 65_536 {
        return Err(io::Error::other("Studio session exceeds 64 KiB"));
    }
    let parent = path
        .parent()
        .filter(|parent| !parent.as_os_str().is_empty())
        .unwrap_or_else(|| std::path::Path::new("."));
    std::fs::create_dir_all(parent)?;
    let mut temporary = tempfile::NamedTempFile::new_in(parent)?;
    temporary.write_all(&bytes)?;
    temporary.persist(path).map_err(|error| error.error)?;
    Ok(())
}
