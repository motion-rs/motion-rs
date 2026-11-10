//! Standalone audio export to WAV (RIFF or RF64), independent of video encoding.
//!
//! Other containers (FLAC, Opus, WebM audio) are not offered: the native FFmpeg bridge only
//! exposes the AAC encoder, so they would need new bindings.

use super::pcm::{MixedAudio, ResampleQuality, ceil_frames, invalid};
use motion_core::{AudioMetadata, AudioSource, Time, Timeline};
use std::{io::Write, ops::Range, path::PathBuf};

/// Sample encoding of the written WAV file.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum AudioFormat {
    /// 16-bit signed integer PCM; clips at full scale. Optional TPDF dither.
    Wav16,
    /// 24-bit signed integer PCM; clips at full scale.
    #[default]
    Wav24,
    /// 32-bit IEEE float; written unclipped.
    WavF32,
}

impl AudioFormat {
    fn bytes(self) -> u64 {
        match self {
            Self::Wav16 => 2,
            Self::Wav24 => 3,
            Self::WavF32 => 4,
        }
    }
}

/// Output channel layout; stereo mixes are averaged for mono.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum AudioChannels {
    /// One channel, `(left + right) / 2`.
    Mono,
    /// Two channels.
    #[default]
    Stereo,
}

impl AudioChannels {
    fn count(self) -> u16 {
        match self {
            Self::Mono => 1,
            Self::Stereo => 2,
        }
    }
}

/// Request for [`export_audio`].
#[derive(Clone, Debug)]
pub struct AudioExportOptions {
    /// Destination, committed only after the whole file is written.
    pub path: PathBuf,
    /// Sample encoding.
    pub format: AudioFormat,
    /// Output rate in 8000..=192000 Hz; common choices are 44100, 48000 and 96000.
    pub sample_rate: u32,
    /// Output channels.
    pub channels: AudioChannels,
    /// Timeline interval; `None` exports the whole timeline.
    pub range: Option<Range<Time>>,
    /// Resampler for sources whose rate differs from `sample_rate`.
    pub resampler: ResampleQuality,
    /// Deterministic TPDF dither when quantizing to 16 bits.
    pub dither: bool,
    /// Title, artist, album, comment, copyright, date, genre or software tags (WAV `LIST INFO`).
    /// Chapters are not supported by WAV and are rejected.
    pub metadata: AudioMetadata,
    /// Replaces an existing destination at commit.
    pub overwrite: bool,
}

impl AudioExportOptions {
    /// 24-bit stereo 48 kHz WAV at `path`.
    pub fn new(path: impl Into<PathBuf>) -> Self {
        Self {
            path: path.into(),
            format: AudioFormat::default(),
            sample_rate: 48_000,
            channels: AudioChannels::default(),
            range: None,
            resampler: ResampleQuality::default(),
            dither: false,
            metadata: AudioMetadata::default(),
            overwrite: false,
        }
    }

    /// Selects the sample encoding.
    pub fn format(mut self, format: AudioFormat) -> Self {
        self.format = format;
        self
    }

    /// Selects the output rate.
    pub fn sample_rate(mut self, rate: u32) -> Self {
        self.sample_rate = rate;
        self
    }

    /// Selects mono or stereo output.
    pub fn channels(mut self, channels: AudioChannels) -> Self {
        self.channels = channels;
        self
    }

    /// Selects a timeline interval.
    pub fn range(mut self, range: Range<Time>) -> Self {
        self.range = Some(range);
        self
    }

    /// Selects the resampler.
    pub fn resampler(mut self, resampler: ResampleQuality) -> Self {
        self.resampler = resampler;
        self
    }

    /// Enables 16-bit TPDF dither.
    pub fn dither(mut self, dither: bool) -> Self {
        self.dither = dither;
        self
    }

    /// Sets container tags.
    pub fn metadata(mut self, metadata: AudioMetadata) -> Self {
        self.metadata = metadata;
        self
    }

    /// Controls replacement of an existing destination.
    pub fn overwrite(mut self, overwrite: bool) -> Self {
        self.overwrite = overwrite;
        self
    }

    /// Checks all fields without touching the file system.
    pub fn validate(&self) -> motion_core::Result {
        if !(8000..=192_000).contains(&self.sample_rate) {
            return Err(invalid("sample_rate", "Require 8000..=192000 Hz").into());
        }
        self.metadata.validate()?;
        if !self.metadata.chapters.is_empty() {
            return Err(invalid("metadata", "WAV export does not support chapters").into());
        }
        for key in self.metadata.tags.keys() {
            if info_id(key).is_none() {
                return Err(invalid("metadata", "Unsupported WAV tag").into());
            }
        }
        Ok(())
    }
}

/// Outcome of a committed audio export.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct AudioExportReport {
    /// Written frames per channel.
    pub frames: u64,
    /// Final file size.
    pub bytes: u64,
    /// Whether the 64-bit RF64 header was required.
    pub rf64: bool,
    /// Samples clamped to full scale by an integer format.
    pub clipped_samples: u64,
}

fn info_id(key: &str) -> Option<&'static [u8; 4]> {
    Some(match key.to_ascii_lowercase().as_str() {
        "title" => b"INAM",
        "artist" => b"IART",
        "album" => b"IPRD",
        "comment" => b"ICMT",
        "copyright" => b"ICOP",
        "date" => b"ICRD",
        "genre" => b"IGNR",
        "software" => b"ISFT",
        _ => return None,
    })
}

fn info_chunk(metadata: &AudioMetadata) -> Vec<u8> {
    let mut body = b"INFO".to_vec();
    let mut entries: Vec<_> = metadata
        .tags
        .iter()
        .filter_map(|(key, value)| Some((info_id(key)?, value)))
        .collect();
    entries.sort();
    for (id, value) in entries {
        let size = value.len() + 1;
        body.extend_from_slice(id);
        body.extend_from_slice(&(size as u32).to_le_bytes());
        body.extend_from_slice(value.as_bytes());
        body.push(0);
        if size % 2 == 1 {
            body.push(0);
        }
    }
    if body.len() == 4 {
        return Vec::new();
    }
    let mut chunk = b"LIST".to_vec();
    chunk.extend_from_slice(&(body.len() as u32).to_le_bytes());
    chunk.extend(body);
    chunk
}

/// Builds the header up to and including the `data` chunk header.
/// Returns the header and whether it is RF64. Data larger than the 32-bit RIFF limit uses RF64.
pub(crate) fn wav_header(
    format: AudioFormat,
    channels: u16,
    rate: u32,
    frames: u64,
    metadata: &AudioMetadata,
) -> motion_core::Result<(Vec<u8>, bool)> {
    let width = format.bytes();
    let data = frames
        .checked_mul(u64::from(channels))
        .and_then(|samples| samples.checked_mul(width))
        .ok_or_else(|| invalid("wav", "PCM size overflows 64 bits"))?;
    let list = info_chunk(metadata);
    let float = format == AudioFormat::WavF32;
    let fixed = 4 + 8 + 16 + if float { 12 } else { 0 } + list.len() as u64 + 8;
    let total = fixed + data + (data & 1);
    let rf64 = total > u64::from(u32::MAX) || data > u64::from(u32::MAX);
    let mut header = Vec::new();
    let extra = if rf64 { 36 } else { 0 };
    header.extend_from_slice(if rf64 { b"RF64" } else { b"RIFF" });
    header.extend_from_slice(
        &(if rf64 {
            u32::MAX
        } else {
            (total + extra) as u32
        })
        .to_le_bytes(),
    );
    header.extend_from_slice(b"WAVE");
    if rf64 {
        header.extend_from_slice(b"ds64");
        header.extend_from_slice(&28u32.to_le_bytes());
        header.extend_from_slice(&(total + extra).to_le_bytes());
        header.extend_from_slice(&data.to_le_bytes());
        header.extend_from_slice(&frames.to_le_bytes());
        header.extend_from_slice(&0u32.to_le_bytes());
    }
    header.extend_from_slice(b"fmt ");
    header.extend_from_slice(&16u32.to_le_bytes());
    header.extend_from_slice(&(if float { 3u16 } else { 1 }).to_le_bytes());
    header.extend_from_slice(&channels.to_le_bytes());
    header.extend_from_slice(&rate.to_le_bytes());
    let block = u32::from(channels) * width as u32;
    header.extend_from_slice(&rate.saturating_mul(block).to_le_bytes());
    header.extend_from_slice(&(block as u16).to_le_bytes());
    header.extend_from_slice(&((width * 8) as u16).to_le_bytes());
    if float {
        header.extend_from_slice(b"fact");
        header.extend_from_slice(&4u32.to_le_bytes());
        header.extend_from_slice(&(frames.min(u64::from(u32::MAX)) as u32).to_le_bytes());
    }
    header.extend_from_slice(&list);
    header.extend_from_slice(b"data");
    header.extend_from_slice(&(if rf64 { u32::MAX } else { data as u32 }).to_le_bytes());
    Ok((header, rf64))
}

fn mix_hash(mut value: u64) -> u64 {
    value = value.wrapping_add(0x9e37_79b9_7f4a_7c15);
    value = (value ^ (value >> 30)).wrapping_mul(0xbf58_476d_1ce4_e5b9);
    value = (value ^ (value >> 27)).wrapping_mul(0x94d0_49bb_1331_11eb);
    value ^ (value >> 31)
}

/// Uniform value in [-0.5, 0.5) from a sample index and stream tag.
fn uniform(index: u64, stream: u64) -> f64 {
    (mix_hash(index.wrapping_mul(2).wrapping_add(stream)) >> 11) as f64 / (1u64 << 53) as f64 - 0.5
}

/// Writes the mixed timeline audio as a WAV file and returns the written size.
/// Existing destinations are kept unless `overwrite` is set; failures leave no partial file.
pub fn export_audio(
    timeline: &Timeline,
    options: &AudioExportOptions,
) -> motion_core::Result<AudioExportReport> {
    options.validate()?;
    let clips = timeline.soundtrack().clips();
    if clips.is_empty() {
        return Err(invalid("export", "Timeline has no audio to export").into());
    }
    let range = options
        .range
        .clone()
        .unwrap_or(Time::ZERO..timeline.duration());
    if range.start < Time::ZERO || range.end <= range.start {
        return Err(invalid("range", "Audio export range must be nonempty").into());
    }
    if !options.overwrite && options.path.try_exists()? {
        return Err(std::io::Error::new(
            std::io::ErrorKind::AlreadyExists,
            "Export destination exists",
        )
        .into());
    }
    if options.path.try_exists()? {
        let destination = options.path.canonicalize()?;
        for clip in clips {
            if let Some(path) = clip.parameters().source.file_path()
                && path.canonicalize()? == destination
            {
                return Err(invalid(
                    "path",
                    "Export destination must not replace an audio source",
                )
                .into());
            }
        }
    }
    let mix = MixedAudio::from_window_with(
        timeline,
        range.clone(),
        options.sample_rate,
        options.resampler,
    )?;
    let frames = ceil_frames(range.end - range.start, options.sample_rate)?;
    let channels = options.channels.count();
    let (header, rf64) = wav_header(
        options.format,
        channels,
        options.sample_rate,
        frames,
        &options.metadata,
    )?;
    let parent = options
        .path
        .parent()
        .filter(|parent| !parent.as_os_str().is_empty())
        .unwrap_or(std::path::Path::new("."));
    let file = tempfile::Builder::new()
        .prefix(".motion-audio-")
        .tempfile_in(parent)?;
    let mut output = std::io::BufWriter::new(file);
    output.write_all(&header)?;
    let mut clipped = 0u64;
    let mut buffer = vec![0.0f32; 8192];
    let mut bytes = Vec::with_capacity(4096 * 8);
    let mut start = 0u64;
    while start < frames {
        let count = (frames - start).min(4096) as usize;
        mix.read(start, &mut buffer[..count * 2]);
        bytes.clear();
        for frame in 0..count {
            let pair = [buffer[frame * 2], buffer[frame * 2 + 1]];
            let mono = [(pair[0] + pair[1]) * 0.5];
            let samples: &[f32] = if channels == 1 { &mono } else { &pair };
            for (channel, value) in samples.iter().enumerate() {
                let index = (start + frame as u64) * u64::from(channels) + channel as u64;
                match options.format {
                    AudioFormat::WavF32 => bytes.extend_from_slice(&value.to_le_bytes()),
                    AudioFormat::Wav16 | AudioFormat::Wav24 => {
                        let scale = if options.format == AudioFormat::Wav16 {
                            32767.0
                        } else {
                            8_388_607.0
                        };
                        let noise = if options.dither && options.format == AudioFormat::Wav16 {
                            uniform(index, 0) + uniform(index, 1)
                        } else {
                            0.0
                        };
                        let wanted = libm::round(f64::from(*value) * scale + noise);
                        let quantized = wanted.clamp(-scale - 1.0, scale);
                        clipped += u64::from(wanted != quantized);
                        let quantized = quantized as i32;
                        let width = options.format.bytes() as usize;
                        bytes.extend_from_slice(&quantized.to_le_bytes()[..width]);
                    }
                }
            }
        }
        output.write_all(&bytes)?;
        start += count as u64;
    }
    let data = frames * u64::from(channels) * options.format.bytes();
    if data & 1 == 1 {
        output.write_all(&[0])?;
    }
    let file = output.into_inner().map_err(|error| error.into_error())?;
    file.as_file().sync_all()?;
    let size = file.as_file().metadata()?.len();
    if options.overwrite {
        file.persist(&options.path).map_err(|error| error.error)?;
    } else {
        file.persist_noclobber(&options.path)
            .map_err(|error| error.error)?;
    }
    Ok(AudioExportReport {
        frames,
        bytes: size,
        rf64,
        clipped_samples: clipped,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::audio::{AudioClip, PcmBuffer};
    use motion_core::{Size, TimeLiteral};

    fn timeline(level: f32) -> Timeline {
        let pcm = PcmBuffer::new(
            48_000,
            2,
            (0..4800).flat_map(|_| [level, -level / 2.0]).collect(),
        )
        .unwrap();
        Timeline::new(Size::new(8.0, 4.0), 30)
            .audio(AudioClip::source(pcm))
            .build()
            .unwrap()
    }

    #[test]
    fn writes_pcm16_stereo_with_tags_and_protects_destinations() -> motion_core::Result {
        let directory = tempfile::tempdir()?;
        let path = directory.path().join("out.wav");
        let mut metadata = AudioMetadata::default();
        metadata.tags.insert("Title".into(), "Demo".into());
        let options = AudioExportOptions::new(&path)
            .format(AudioFormat::Wav16)
            .metadata(metadata);
        let report = export_audio(&timeline(0.5), &options)?;
        assert_eq!(
            (report.frames, report.rf64, report.clipped_samples),
            (4800, false, 0)
        );
        let bytes = std::fs::read(&path)?;
        assert_eq!(bytes.len() as u64, report.bytes);
        assert_eq!(&bytes[..4], b"RIFF");
        assert_eq!(
            u32::from_le_bytes(bytes[4..8].try_into().unwrap()) as usize,
            bytes.len() - 8
        );
        assert!(bytes.windows(4).any(|window| window == b"INAM"));
        let tail = &bytes[bytes.len() - 4800 * 4..];
        assert_eq!(i16::from_le_bytes([tail[0], tail[1]]), 16384);
        assert_eq!(i16::from_le_bytes([tail[2], tail[3]]), -8192);
        assert!(export_audio(&timeline(0.5), &options).is_err());
        assert!(export_audio(&timeline(0.5), &options.clone().overwrite(true)).is_ok());
        assert_eq!(std::fs::read_dir(directory.path())?.count(), 1);
        Ok(())
    }

    #[test]
    fn float_mono_range_and_rate_and_integer_clipping() -> motion_core::Result {
        let directory = tempfile::tempdir()?;
        let path = directory.path().join("float.wav");
        let options = AudioExportOptions::new(&path)
            .format(AudioFormat::WavF32)
            .channels(AudioChannels::Mono)
            .sample_rate(44_100)
            .resampler(ResampleQuality::Sinc)
            .range(20.milliseconds()..70.milliseconds());
        let report = export_audio(&timeline(0.5), &options)?;
        assert_eq!(report.frames, 2205);
        let bytes = std::fs::read(&path)?;
        assert_eq!(u16::from_le_bytes([bytes[20], bytes[21]]), 3);
        assert_eq!(u16::from_le_bytes([bytes[22], bytes[23]]), 1);
        assert_eq!(
            u32::from_le_bytes(bytes[24..28].try_into().unwrap()),
            44_100
        );
        let mid = bytes.len() - 2205 * 4 + 1000 * 4;
        let value = f32::from_le_bytes(bytes[mid..mid + 4].try_into().unwrap());
        assert!((value - 0.125).abs() < 1e-3, "{value}");

        let loud = directory.path().join("loud.wav");
        let report = export_audio(
            &timeline(2.0),
            &AudioExportOptions::new(&loud).format(AudioFormat::Wav24),
        )?;
        assert_eq!(report.clipped_samples, 4800);
        Ok(())
    }

    #[test]
    fn dither_is_deterministic_and_rejections_are_explicit() -> motion_core::Result {
        let directory = tempfile::tempdir()?;
        let first = directory.path().join("a.wav");
        let second = directory.path().join("b.wav");
        for path in [&first, &second] {
            export_audio(
                &timeline(0.3),
                &AudioExportOptions::new(path)
                    .format(AudioFormat::Wav16)
                    .dither(true),
            )?;
        }
        assert_eq!(std::fs::read(&first)?, std::fs::read(&second)?);
        let bad = directory.path().join("bad.wav");
        assert!(
            export_audio(
                &timeline(0.3),
                &AudioExportOptions::new(&bad).sample_rate(7)
            )
            .is_err()
        );
        assert!(
            export_audio(
                &Timeline::new(Size::new(8.0, 4.0), 30).build()?,
                &AudioExportOptions::new(&bad)
            )
            .is_err()
        );
        let mut metadata = AudioMetadata::default();
        metadata.tags.insert("bogus".into(), "x".into());
        assert!(
            export_audio(
                &timeline(0.3),
                &AudioExportOptions::new(&bad).metadata(metadata)
            )
            .is_err()
        );
        assert!(!bad.exists());
        Ok(())
    }

    #[test]
    fn large_outputs_use_rf64_and_odd_data_is_padded() -> motion_core::Result {
        let none = AudioMetadata::default();
        let (small, rf64) = wav_header(AudioFormat::Wav24, 1, 48_000, 3, &none)?;
        assert!(!rf64);
        assert_eq!(&small[..4], b"RIFF");
        let riff = u32::from_le_bytes(small[4..8].try_into().unwrap()) as usize;
        assert_eq!(riff, small.len() - 8 + 9 + 1);
        let (big, rf64) = wav_header(AudioFormat::WavF32, 2, 48_000, 600_000_000, &none)?;
        assert!(rf64);
        assert_eq!(&big[..4], b"RF64");
        assert_eq!(&big[12..16], b"ds64");
        let data = u64::from_le_bytes(big[28..36].try_into().unwrap());
        assert_eq!(data, 600_000_000 * 8);
        assert_eq!(&big[big.len() - 8..big.len() - 4], b"data");
        assert!(wav_header(AudioFormat::WavF32, 2, 48_000, u64::MAX, &none).is_err());
        Ok(())
    }
}
