//! Windowed, checked file decoding without whole-file PCM retention.

use motion_core::{AudioClip, AudioSource, Duration, Time};
use std::{
    path::{Path, PathBuf},
    sync::Arc,
};
use symphonia::core::{
    audio::SampleBuffer,
    codecs::{Decoder, DecoderOptions},
    formats::{FormatOptions, FormatReader, SeekMode, SeekTo},
    io::MediaSourceStream,
    meta::MetadataOptions,
    probe::Hint,
};

struct Reader {
    format: Box<dyn FormatReader>,
    decoder: Box<dyn Decoder>,
    id: u32,
    rate: u32,
    channels: u16,
    frames: Option<u64>,
}

fn reader(path: &Path) -> motion_core::Result<Reader> {
    let error = |cause| motion_core::Error::asset("audio", path.display().to_string(), cause);
    let file = std::fs::File::open(path).map_err(error)?;
    if !file.metadata()?.is_file() {
        return Err(super::pcm::invalid("file", "Audio source must be a regular file").into());
    }
    let stream = MediaSourceStream::new(Box::new(file), Default::default());
    let format = symphonia::default::get_probe()
        .format(
            &Hint::new(),
            stream,
            &FormatOptions {
                enable_gapless: true,
                ..Default::default()
            },
            &MetadataOptions::default(),
        )
        .map_err(|cause| motion_core::Error::asset("audio", path.display().to_string(), cause))?
        .format;
    let track = format
        .tracks()
        .iter()
        .find(|track| track.codec_params.sample_rate.is_some())
        .or_else(|| format.default_track())
        .ok_or_else(|| super::pcm::invalid("stream", "No audio track"))?;
    let id = track.id;
    let rate = track
        .codec_params
        .sample_rate
        .ok_or_else(|| super::pcm::invalid("format", "Audio sampling rate is unknown"))?;
    let channels = track
        .codec_params
        .channels
        .ok_or_else(|| super::pcm::invalid("format", "Audio channel layout is unknown"))?
        .count() as u16;
    if !(1..=192_000).contains(&rate) || !(1..=2).contains(&channels) {
        return Err(
            super::pcm::invalid("format", "Require mono/stereo audio at 1..=192000 Hz").into(),
        );
    }
    let frames = track.codec_params.n_frames;
    let decoder = symphonia::default::get_codecs()
        .make(&track.codec_params, &DecoderOptions::default())
        .map_err(|cause| motion_core::Error::asset("audio", path.display().to_string(), cause))?;
    Ok(Reader {
        format,
        decoder,
        id,
        rate,
        channels,
        frames,
    })
}

struct FileSource {
    path: PathBuf,
    rate: u32,
    channels: u16,
    frames: Option<u64>,
}

impl AudioSource for FileSource {
    fn file_path(&self) -> Option<&Path> {
        Some(&self.path)
    }

    fn sample_rate(&self) -> u32 {
        self.rate
    }

    fn channels(&self) -> u16 {
        self.channels
    }

    fn duration(&self) -> Option<Duration> {
        self.frames
            .and_then(|frames| {
                i64::try_from(
                    (u128::from(frames) * motion_core::TICKS_PER_SECOND as u128)
                        .div_ceil(u128::from(self.rate)),
                )
                .ok()
            })
            .map(Time::from_ticks)
    }

    fn read(&self, start: u64, output: &mut [f32]) -> usize {
        self.try_read(start, output)
            .expect("Use AudioSource::try_read for recoverable file decoding")
    }

    fn try_read(&self, start: u64, output: &mut [f32]) -> motion_core::Result<usize> {
        output.fill(0.0);
        let channels = usize::from(self.channels);
        let wanted = output.len() / channels;
        if wanted == 0 {
            return Ok(0);
        }
        let mut reader = reader(&self.path)?;
        let mut cursor = 0;
        if start > 0
            && let Ok(seek) = reader.format.seek(
                SeekMode::Accurate,
                SeekTo::TimeStamp {
                    ts: start,
                    track_id: reader.id,
                },
            )
        {
            cursor = seek.actual_ts;
            reader.decoder.reset();
        }
        let mut written = 0;
        loop {
            let packet = match reader.format.next_packet() {
                Ok(packet) => packet,
                Err(symphonia::core::errors::Error::IoError(error))
                    if error.kind() == std::io::ErrorKind::UnexpectedEof =>
                {
                    break;
                }
                Err(error) => {
                    return Err(motion_core::Error::asset(
                        "audio",
                        self.path.display().to_string(),
                        error,
                    ));
                }
            };
            if packet.track_id() != reader.id {
                continue;
            }
            let decoded = reader.decoder.decode(&packet).map_err(|error| {
                motion_core::Error::asset("audio", self.path.display().to_string(), error)
            })?;
            if decoded.spec().rate != self.rate || decoded.spec().channels.count() != channels {
                return Err(
                    super::pcm::invalid("format", "Audio format changed while streaming").into(),
                );
            }
            let count = decoded.frames();
            if decoded.capacity().saturating_mul(channels) > super::pcm::MAX_SAMPLES {
                return Err(super::pcm::invalid(
                    "packet",
                    "Decoded audio packet exceeds the bounded sample allowance",
                )
                .into());
            }
            let mut buffer = SampleBuffer::<f32>::new(decoded.capacity() as u64, *decoded.spec());
            buffer.copy_interleaved_ref(decoded);
            let skip = start.saturating_sub(cursor).min(count as u64) as usize;
            let count = (count - skip).min(wanted - written);
            let samples = &buffer.samples()[skip * channels..(skip + count) * channels];
            if samples.iter().any(|sample| !sample.is_finite()) {
                return Err(super::pcm::invalid("source", "Nonfinite decoded PCM").into());
            }
            output[written * channels..(written + count) * channels].copy_from_slice(samples);
            written += count;
            cursor += decoded_frames(&buffer, channels) as u64;
            if written == wanted {
                break;
            }
        }
        Ok(written)
    }
}

fn decoded_frames(buffer: &SampleBuffer<f32>, channels: usize) -> usize {
    buffer.samples().len() / channels
}

pub(super) fn resolve(clip: &AudioClip) -> motion_core::Result<AudioClip> {
    if !clip.unresolved_file() {
        return Ok(clip.clone());
    }
    let path = clip.parameters().source.file_path().expect("file source");
    let metadata = reader(path)?;
    let source = FileSource {
        path: path.to_path_buf(),
        rate: metadata.rate,
        channels: metadata.channels,
        frames: metadata.frames,
    };
    let resolved = clip.with_resolved_source(Arc::new(source));
    resolved.resolved_duration()?;
    Ok(resolved)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::MixedAudio;
    use motion_core::{Size, TimeLiteral, Timeline};
    use std::io::Write;

    #[test]
    fn files_stay_lazy_and_window_reads_match_prepared_pcm_fades() -> motion_core::Result {
        let directory = tempfile::tempdir()?;
        let path = directory.path().join("tone.wav");
        let clip = AudioClip::file(path.clone()).fade_in(100.milliseconds());
        let timeline = Timeline::new(Size::new(8.0, 4.0), 30).audio(clip);
        assert!(timeline.validate().is_ok());
        assert!(super::super::prepare_timeline(&timeline).is_err());
        let count = 8000_u32;
        let mut wav = std::fs::File::create(&path)?;
        wav.write_all(b"RIFF")?;
        wav.write_all(&(36 + count * 2).to_le_bytes())?;
        wav.write_all(b"WAVEfmt ")?;
        wav.write_all(&16_u32.to_le_bytes())?;
        wav.write_all(&1_u16.to_le_bytes())?;
        wav.write_all(&1_u16.to_le_bytes())?;
        wav.write_all(&count.to_le_bytes())?;
        wav.write_all(&(count * 2).to_le_bytes())?;
        wav.write_all(&2_u16.to_le_bytes())?;
        wav.write_all(&16_u16.to_le_bytes())?;
        wav.write_all(b"data")?;
        wav.write_all(&(count * 2).to_le_bytes())?;
        for index in 0..count {
            wav.write_all(&((index as i16 % 100) * 100).to_le_bytes())?;
        }
        drop(wav);
        let prepared = super::super::prepare_timeline(&timeline)?;
        assert_eq!(prepared.frame_count(), 30);
        let original = std::fs::read(&path)?;
        let mut options = crate::ExportOptions::new(&path).overwrite(true);
        options.codec = crate::Codec::default();
        assert!(crate::export(&prepared, &options, &crate::NoProgress).is_err());
        assert_eq!(std::fs::read(&path)?, original);
        let full = MixedAudio::from_timeline(&prepared, 8000)?;
        let window =
            MixedAudio::from_window(&prepared, 50.milliseconds()..150.milliseconds(), 8000)?;
        let mut expected = vec![0.0; 1600];
        let mut actual = vec![0.0; 1600];
        full.read(400, &mut expected);
        window.read(0, &mut actual);
        assert_eq!(expected, actual);
        std::fs::remove_file(path)?;
        assert!(MixedAudio::from_timeline(&prepared, 8000).is_err());
        Ok(())
    }
}
