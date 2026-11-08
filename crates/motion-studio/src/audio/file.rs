//! Bounded, versioned audio draft files; loading never opens media sources.

use std::io::{self, Read, Write};
use std::path::Path;

use serde::{Deserialize, Serialize};

use crate::audio::settings::AudioForm;

const MAX_BYTES: usize = 1_048_576;

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Document {
    version: u32,
    audio: AudioForm,
}

impl AudioForm {
    pub fn save(&self, path: impl AsRef<Path>) -> io::Result<()> {
        self.validate_document()?;
        let destination = std::path::absolute(path)?;
        let mut audio = self.clone();
        for clip in &mut audio.clips {
            clip.path = std::path::absolute(&clip.path)?;
            if clip.path == destination
                || std::fs::canonicalize(&clip.path)
                    .ok()
                    .zip(std::fs::canonicalize(&destination).ok())
                    .is_some_and(|(source, output)| source == output)
            {
                return Err(invalid("Audio configuration cannot replace a source file"));
            }
        }
        let bytes = serde_json::to_vec_pretty(&Document { version: 1, audio }).map_err(invalid)?;
        if bytes.len() + 1 > MAX_BYTES {
            return Err(invalid("Audio configuration exceeds 1 MiB"));
        }
        let parent = destination
            .parent()
            .ok_or_else(|| invalid("Choose an audio configuration file"))?;
        let mut temporary = tempfile::NamedTempFile::new_in(parent)?;
        temporary.write_all(&bytes)?;
        temporary.write_all(b"\n")?;
        temporary.as_file().sync_all()?;
        temporary
            .persist(destination)
            .map_err(|error| error.error)?;
        Ok(())
    }

    pub fn load(&mut self, path: impl AsRef<Path>) -> io::Result<()> {
        let path = std::path::absolute(path)?;
        let mut bytes = Vec::new();
        std::fs::File::open(&path)?
            .take(MAX_BYTES as u64 + 1)
            .read_to_end(&mut bytes)?;
        if bytes.len() > MAX_BYTES {
            return Err(invalid("Audio configuration exceeds 1 MiB"));
        }
        let mut document: Document = serde_json::from_slice(&bytes).map_err(invalid)?;
        if document.version != 1 {
            return Err(invalid("Unsupported audio configuration version"));
        }
        document.audio.validate_document()?;
        let parent = path
            .parent()
            .ok_or_else(|| invalid("Choose an audio configuration file"))?;
        for clip in &mut document.audio.clips {
            if clip.path.is_relative() {
                clip.path = parent.join(&clip.path);
            }
        }
        *self = document.audio;
        Ok(())
    }

    fn validate_document(&self) -> io::Result<()> {
        if self.clips.len() > motion_encode::audio::MAX_CLIPS {
            return Err(invalid(format!(
                "At most {} audio drafts are allowed",
                motion_encode::audio::MAX_CLIPS
            )));
        }
        for (index, clip) in self.clips.iter().enumerate() {
            if clip.path.as_os_str().is_empty()
                || ![
                    clip.source_in,
                    clip.source_out,
                    clip.start,
                    clip.fade_in,
                    clip.fade_out,
                ]
                .into_iter()
                .all(|seconds| {
                    seconds.is_finite()
                        && (0.0..=motion_encode::audio::MAX_SECONDS as f64).contains(&seconds)
                })
                || !clip.volume.is_finite()
                || !(0.0..=16.0).contains(&clip.volume)
            {
                return Err(invalid(format!(
                    "Audio draft {} has an empty path or invalid numeric fields",
                    index + 1
                )));
            }
        }
        Ok(())
    }
}

fn invalid(message: impl std::fmt::Display) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, message.to_string())
}

pub(crate) mod source_path {
    use std::path::{Path, PathBuf};

    use serde::{Deserialize, Deserializer, Serialize, Serializer};

    #[derive(Serialize, Deserialize)]
    #[serde(
        tag = "encoding",
        content = "value",
        rename_all = "snake_case",
        deny_unknown_fields
    )]
    enum StoredPath {
        Utf8(String),
        #[cfg(unix)]
        UnixBytes(Vec<u8>),
        #[cfg(windows)]
        WindowsWide(Vec<u16>),
    }

    pub fn serialize<S: Serializer>(path: &Path, serializer: S) -> Result<S::Ok, S::Error> {
        if let Some(value) = path.to_str() {
            return StoredPath::Utf8(value.to_owned()).serialize(serializer);
        }
        #[cfg(unix)]
        {
            use std::os::unix::ffi::OsStrExt;
            StoredPath::UnixBytes(path.as_os_str().as_bytes().to_vec()).serialize(serializer)
        }
        #[cfg(windows)]
        {
            use std::os::windows::ffi::OsStrExt;
            return StoredPath::WindowsWide(path.as_os_str().encode_wide().collect())
                .serialize(serializer);
        }
        #[cfg(not(any(unix, windows)))]
        Err(serde::ser::Error::custom(
            "Source path is not representable as UTF-8",
        ))
    }

    pub fn deserialize<'de, D: Deserializer<'de>>(deserializer: D) -> Result<PathBuf, D::Error> {
        Ok(match StoredPath::deserialize(deserializer)? {
            StoredPath::Utf8(value) => PathBuf::from(value),
            #[cfg(unix)]
            StoredPath::UnixBytes(value) => {
                use std::os::unix::ffi::OsStringExt;
                PathBuf::from(std::ffi::OsString::from_vec(value))
            }
            #[cfg(windows)]
            StoredPath::WindowsWide(value) => {
                use std::os::windows::ffi::OsStringExt;
                PathBuf::from(std::ffi::OsString::from_wide(&value))
            }
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::audio::settings::ClipSettings;

    #[test]
    fn drafts_round_trip_without_opening_missing_media() -> io::Result<()> {
        let directory = tempfile::tempdir()?;
        let file = directory.path().join("audio.json");
        let mut clip = ClipSettings::new(directory.path().join("missing.wav"), 4.0);
        clip.enabled = false;
        clip.source_in = 3.0;
        clip.source_out = 1.0;
        clip.start = 2.0;
        clip.volume = 0.125;
        clip.fade_in = 0.2;
        clip.fade_out = 0.4;
        let source = AudioForm {
            enabled: true,
            clips: vec![clip],
        };
        source.save(&file)?;
        let mut restored = AudioForm::default();
        restored.load(&file)?;
        assert_eq!(
            serde_json::to_value(&restored).unwrap(),
            serde_json::to_value(&source).unwrap()
        );
        Ok(())
    }

    #[test]
    fn finite_time_and_volume_values_round_trip_bit_exactly() -> io::Result<()> {
        let directory = tempfile::tempdir()?;
        let file = directory.path().join("audio.json");
        for value in [
            0.10000000000000002_f64,
            0.9999999999999999,
            12345.678901234567,
            86399.99999999999,
        ] {
            let mut clip = ClipSettings::new(directory.path().join("missing.wav"), value);
            clip.source_in = value;
            clip.start = value;
            clip.fade_in = value;
            clip.fade_out = value;
            clip.volume = 0.12345679;
            let settings = AudioForm {
                enabled: false,
                clips: vec![clip],
            };
            settings.save(&file)?;
            let mut restored = AudioForm::default();
            restored.load(&file)?;
            let clip = &restored.clips[0];
            for seconds in [
                clip.source_in,
                clip.source_out,
                clip.start,
                clip.fade_in,
                clip.fade_out,
            ] {
                assert_eq!(seconds.to_bits(), value.to_bits());
            }
            assert_eq!(clip.volume.to_bits(), settings.clips[0].volume.to_bits());
        }
        Ok(())
    }

    #[test]
    fn failed_loads_and_saves_preserve_current_drafts_and_existing_files() -> io::Result<()> {
        let directory = tempfile::tempdir()?;
        let file = directory.path().join("audio.json");
        let mut settings = AudioForm {
            enabled: true,
            clips: vec![ClipSettings::new(directory.path().join("missing.wav"), 1.0)],
        };
        let original = serde_json::to_value(&settings).unwrap();
        for bytes in [
            b"{broken".as_slice(),
            br#"{"version":2,"audio":{"enabled":false,"clips":[]}}"#.as_slice(),
        ] {
            std::fs::write(&file, bytes)?;
            assert!(settings.load(&file).is_err());
            assert_eq!(serde_json::to_value(&settings).unwrap(), original);
        }
        std::fs::write(&file, b"keep existing configuration")?;
        settings.clips[0].volume = f32::NAN;
        assert!(settings.save(&file).is_err());
        assert_eq!(std::fs::read(&file)?, b"keep existing configuration");
        Ok(())
    }

    #[test]
    fn relative_sources_resolve_against_the_configuration_directory() -> io::Result<()> {
        let directory = tempfile::tempdir()?;
        let file = directory.path().join("audio.json");
        let document = Document {
            version: 1,
            audio: AudioForm {
                enabled: false,
                clips: vec![ClipSettings::new("voice.wav".into(), 1.0)],
            },
        };
        std::fs::write(&file, serde_json::to_vec(&document).unwrap())?;
        let mut settings = AudioForm::default();
        settings.load(&file)?;
        assert_eq!(settings.clips[0].path, directory.path().join("voice.wav"));
        Ok(())
    }

    #[test]
    fn document_schema_limits_and_numeric_bounds_are_checked_before_replacement() -> io::Result<()>
    {
        let directory = tempfile::tempdir()?;
        let file = directory.path().join("audio.json");
        let clip = ClipSettings::new("voice.wav".into(), 1.0);
        let valid = serde_json::to_value(Document {
            version: 1,
            audio: AudioForm {
                enabled: true,
                clips: vec![clip.clone()],
            },
        })
        .unwrap();
        let mut invalids = Vec::new();
        let mut unknown = valid.clone();
        unknown["unexpected"] = true.into();
        invalids.push(unknown);
        let mut too_many = valid.clone();
        too_many["audio"]["clips"] =
            serde_json::to_value(vec![clip; motion_encode::audio::MAX_CLIPS + 1]).unwrap();
        invalids.push(too_many);
        for field in [
            "source_in",
            "source_out",
            "start",
            "fade_in",
            "fade_out",
            "volume",
        ] {
            let mut out_of_range = valid.clone();
            out_of_range["audio"]["clips"][0][field] = (-1.0).into();
            invalids.push(out_of_range);
        }
        let mut empty_path = valid.clone();
        empty_path["audio"]["clips"][0]["path"]["value"] = "".into();
        invalids.push(empty_path);
        let mut settings = AudioForm::default();
        for document in invalids {
            std::fs::write(&file, serde_json::to_vec(&document).unwrap())?;
            assert!(settings.load(&file).is_err());
            assert!(!settings.enabled && settings.clips.is_empty());
        }
        let mut bytes = serde_json::to_vec(&valid).unwrap();
        bytes.resize(MAX_BYTES, b' ');
        std::fs::write(&file, &bytes)?;
        settings.load(&file)?;
        let before = serde_json::to_value(&settings).unwrap();
        bytes.push(b' ');
        std::fs::write(&file, &bytes)?;
        assert!(settings.load(&file).is_err());
        assert_eq!(serde_json::to_value(&settings).unwrap(), before);
        Ok(())
    }

    #[test]
    fn saving_over_a_media_source_is_rejected_without_changing_its_bytes() -> io::Result<()> {
        let directory = tempfile::tempdir()?;
        let file = directory.path().join("source.json");
        std::fs::write(&file, b"original media")?;
        let settings = AudioForm {
            enabled: false,
            clips: vec![ClipSettings::new(file.clone(), 1.0)],
        };
        assert!(settings.save(&file).is_err());
        assert_eq!(std::fs::read(&file)?, b"original media");
        #[cfg(unix)]
        {
            let alias = directory.path().join("alias.json");
            std::os::unix::fs::symlink(&file, &alias)?;
            assert!(settings.save(&alias).is_err());
            assert_eq!(std::fs::read(&file)?, b"original media");
        }
        Ok(())
    }

    #[cfg(unix)]
    #[test]
    fn non_utf8_sources_round_trip_without_loss() -> io::Result<()> {
        use std::ffi::OsString;
        use std::os::unix::ffi::OsStringExt;

        let directory = tempfile::tempdir()?;
        let file = directory.path().join("audio.json");
        let path = directory
            .path()
            .join(OsString::from_vec(b"voice-\xff.wav".to_vec()));
        let settings = AudioForm {
            enabled: true,
            clips: vec![ClipSettings::new(path.clone(), 1.0)],
        };
        settings.save(&file)?;
        let mut restored = AudioForm::default();
        restored.load(&file)?;
        assert_eq!(restored.clips[0].path, path);
        Ok(())
    }
}
