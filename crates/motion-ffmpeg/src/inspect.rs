//! Read-only media inspection used by tests and tooling: stream info, container metadata,
//! decoded audio samples, video frame counts and 10-bit luma samples.

use std::io;
use std::path::Path;
use std::ptr;

use super::{TIMELINE_TICKS_PER_SECOND, again, check, ffi, path_string};

/// Stream metadata read directly from the container.
pub struct MediaInfo {
    /// Declared frame count of the first video stream.
    pub video_frames: i64,
    /// Average frame rate of the first video stream, or zero when unknown.
    pub video_fps: f64,
    /// Whether that stream contains H.264 video.
    pub h264: bool,
    /// Whether the first video stream is HEVC/H.265.
    pub h265: bool,
    /// First video's decoded component precision, or zero when unknown.
    pub video_bit_depth: u8,
    /// Whether video declares limited-range BT.709 primaries/transfer/matrix.
    pub bt709_limited: bool,
    /// FFmpeg color transfer identifier (1 BT.709, 13 sRGB), or 2 unspecified.
    pub color_transfer: u32,
    /// BT.709 primaries/matrix with limited range, independently of transfer.
    pub bt709_primaries_matrix_limited: bool,
    /// Width of the first video stream, or zero when absent.
    pub width: i32,
    /// Height of the first video stream, or zero when absent.
    pub height: i32,

    /// Whether the first audio stream contains AAC.
    pub aac: bool,
    /// Sample rate of the first audio stream, or zero when absent.
    pub sample_rate: i32,
    /// Channel count of the first audio stream, or zero when absent.
    pub channels: i32,
    /// Declared duration of the first audio stream in seconds, or zero when absent.
    pub audio_seconds: f64,
}

/// Reads local media stream metadata without decoding or launching a process.
pub fn media(path: &Path) -> io::Result<MediaInfo> {
    let input = Input::open(path)?;
    let mut result = MediaInfo {
        video_frames: 0,
        video_fps: 0.0,
        h264: false,
        h265: false,
        video_bit_depth: 0,
        bt709_limited: false,
        color_transfer: 2,
        bt709_primaries_matrix_limited: false,
        width: 0,
        height: 0,
        aac: false,
        sample_rate: 0,
        channels: 0,
        audio_seconds: 0.0,
    };
    let mut video_found = false;
    let mut audio_found = false;
    // SAFETY: `input` holds a context opened by `avformat_open_input` with stream info read;
    // indices stay below `nb_streams`, and each stream owns non-null `codecpar`.
    unsafe {
        for index in 0..(*input.0).nb_streams as usize {
            let stream = *(*input.0).streams.add(index);
            let parameters = &*(*stream).codecpar;
            if parameters.codec_type == ffi::AVMEDIA_TYPE_VIDEO && !video_found {
                result.video_frames = (*stream).nb_frames;
                result.h264 = parameters.codec_id == ffi::AV_CODEC_ID_H264;
                result.h265 = parameters.codec_id == ffi::AV_CODEC_ID_HEVC;
                result.color_transfer = parameters.color_trc;
                result.bt709_primaries_matrix_limited = parameters.color_primaries
                    == ffi::AVCOL_PRI_BT709
                    && parameters.color_space == ffi::AVCOL_SPC_BT709
                    && parameters.color_range == ffi::AVCOL_RANGE_MPEG;
                result.video_bit_depth = match parameters.format {
                    ffi::AV_PIX_FMT_YUV420P10LE | ffi::AV_PIX_FMT_YUVA444P10LE => 10,
                    ffi::AV_PIX_FMT_YUV420P
                    | ffi::AV_PIX_FMT_YUVA420P
                    | ffi::AV_PIX_FMT_RGB24
                    | ffi::AV_PIX_FMT_RGBA => 8,
                    _ => u8::try_from(parameters.bits_per_raw_sample).unwrap_or(0),
                };
                result.bt709_limited = parameters.color_primaries == ffi::AVCOL_PRI_BT709
                    && parameters.color_trc == ffi::AVCOL_TRC_BT709
                    && parameters.color_space == ffi::AVCOL_SPC_BT709
                    && parameters.color_range == ffi::AVCOL_RANGE_MPEG;
                result.width = parameters.width;
                result.height = parameters.height;
                let rate = (*stream).avg_frame_rate;
                if rate.num > 0 && rate.den > 0 {
                    result.video_fps = f64::from(rate.num) / f64::from(rate.den);
                }
                video_found = true;
            }
            if parameters.codec_type == ffi::AVMEDIA_TYPE_AUDIO && !audio_found {
                result.aac = parameters.codec_id == ffi::AV_CODEC_ID_AAC;
                result.sample_rate = parameters.sample_rate;
                result.channels = parameters.ch_layout.nb_channels;
                let time_base = (*stream).time_base;
                if (*stream).duration >= 0 && time_base.den > 0 {
                    result.audio_seconds = (*stream).duration as f64 * f64::from(time_base.num)
                        / f64::from(time_base.den);
                }
                audio_found = true;
            }
        }
    }
    Ok(result)
}

/// Reads bounded container tags and chapters without launching a process.
pub fn metadata(path: &Path) -> io::Result<crate::MediaMetadata> {
    let input = Input::open(path)?;
    let mut result = crate::MediaMetadata::default();
    // SAFETY: `input` holds an opened context. Dictionary entries returned by `av_dict_get` are
    // null-checked and have NUL-terminated key/value strings owned by the context. Chapter
    // indices stay below `nb_chapters`, whose table entries are allocated by the demuxer.
    unsafe {
        let mut entry = std::ptr::null_mut();
        loop {
            entry = ffi::av_dict_get(
                (*input.0).metadata,
                c"".as_ptr(),
                entry,
                ffi::AV_DICT_IGNORE_SUFFIX as i32,
            );
            if entry.is_null() {
                break;
            }
            if result.tags.len() >= 128 {
                return Err(io::Error::other("Too many inspected metadata tags"));
            }
            result.tags.push((
                std::ffi::CStr::from_ptr((*entry).key)
                    .to_string_lossy()
                    .into_owned(),
                std::ffi::CStr::from_ptr((*entry).value)
                    .to_string_lossy()
                    .into_owned(),
            ));
        }
        if (*input.0).nb_chapters > 4096 {
            return Err(io::Error::other("Too many inspected chapters"));
        }
        for index in 0..(*input.0).nb_chapters as usize {
            let chapter = &**(*input.0).chapters.add(index);
            if chapter.time_base.den <= 0 {
                return Err(io::Error::other("Invalid inspected chapter clock"));
            }
            let title = ffi::av_dict_get(chapter.metadata, c"title".as_ptr(), std::ptr::null(), 0);
            let title = if title.is_null() {
                String::new()
            } else {
                std::ffi::CStr::from_ptr((*title).value)
                    .to_string_lossy()
                    .into_owned()
            };
            let ticks = |value: i64| -> io::Result<i64> {
                i64::try_from(
                    i128::from(value)
                        * i128::from(chapter.time_base.num)
                        * i128::from(TIMELINE_TICKS_PER_SECOND)
                        / i128::from(chapter.time_base.den),
                )
                .map_err(io::Error::other)
            };
            result.chapters.push(crate::MediaChapter {
                start: ticks(chapter.start)?,
                end: ticks(chapter.end)?,
                title,
            });
        }
    }
    Ok(result)
}

/// Decodes an exported 48 kHz planar-float stereo track, bounded by `max_frames`.
/// Other decoded formats are rejected instead of being silently reinterpreted.
pub fn audio_samples(path: &Path, max_frames: usize) -> io::Result<Vec<[f32; 2]>> {
    if max_frames == 0 || max_frames > 32 * 1024 * 1024 {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "PCM inspection limit must be within 1..=33554432 stereo frames",
        ));
    }
    let mut samples = Vec::new();
    decode(path, ffi::AVMEDIA_TYPE_AUDIO, |frame| {
        if frame.format != ffi::AV_SAMPLE_FMT_FLTP
            || frame.sample_rate != 48_000
            || frame.ch_layout.nb_channels != 2
            || frame.nb_samples <= 0
        {
            return Err(io::Error::new(
                io::ErrorKind::Unsupported,
                "Inspection expects 48 kHz planar-float stereo",
            ));
        }
        let count = frame.nb_samples as usize;
        if count > max_frames.saturating_sub(samples.len()) {
            return Err(io::Error::other("Decoded audio exceeds inspection limit"));
        }
        if frame.data[0].is_null() || frame.data[1].is_null() {
            return Err(io::Error::other("Decoded audio has no sample planes"));
        }
        samples.try_reserve(count).map_err(io::Error::other)?;
        // SAFETY: The frame is FLTP stereo with non-null planes checked above, so planes 0 and 1
        // each hold `nb_samples` f32 values owned by the frame for the duration of the callback.
        unsafe {
            let left = std::slice::from_raw_parts(frame.data[0].cast::<f32>(), count);
            let right = std::slice::from_raw_parts(frame.data[1].cast::<f32>(), count);
            samples.extend(left.iter().zip(right).map(|(left, right)| [*left, *right]));
        }
        Ok(())
    })?;
    Ok(samples)
}

/// Counts decoded video frames without retaining pixels, failing beyond `max_frames`.
pub fn video_frames(path: &Path, max_frames: u32) -> io::Result<u32> {
    if max_frames == 0 {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "Video inspection limit must be positive",
        ));
    }
    let mut count = 0;
    decode(path, ffi::AVMEDIA_TYPE_VIDEO, |_| {
        if count == max_frames {
            return Err(io::Error::other("Decoded video exceeds inspection limit"));
        }
        count += 1;
        Ok(())
    })?;
    Ok(count)
}

/// Decodes bounded planar little-endian 10-bit video luma, honoring native byte strides.
pub fn video_luma10(path: &Path, max_samples: usize) -> io::Result<Vec<u16>> {
    if max_samples == 0 || max_samples > 32 * 1024 * 1024 {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "Require 1..=33554432 luma samples",
        ));
    }
    let mut samples = Vec::new();
    decode(path, ffi::AVMEDIA_TYPE_VIDEO, |frame| {
        if frame.format != ffi::AV_PIX_FMT_YUV420P10LE
            || frame.width <= 0
            || frame.height <= 0
            || frame.linesize[0] < frame.width * 2
            || frame.data[0].is_null()
        {
            return Err(io::Error::new(
                io::ErrorKind::Unsupported,
                "Require planar little-endian 10-bit YUV420",
            ));
        }
        let count = frame.width as usize * frame.height as usize;
        if count > max_samples.saturating_sub(samples.len()) {
            return Err(io::Error::other("Decoded luma exceeds limit"));
        }
        samples.try_reserve(count).map_err(io::Error::other)?;
        // SAFETY: The frame is YUV420P10LE with a non-null luma plane and
        // `linesize[0] >= width * 2`, so each of the `height` rows has `width * 2` readable bytes
        // starting at `row * linesize[0]`.
        unsafe {
            for row in 0..frame.height as usize {
                let bytes = std::slice::from_raw_parts(
                    frame.data[0].add(row * frame.linesize[0] as usize),
                    frame.width as usize * 2,
                );
                for bytes in bytes.as_chunks::<2>().0 {
                    samples.push(u16::from_le_bytes([bytes[0], bytes[1]]));
                }
            }
        }
        Ok(())
    })?;
    Ok(samples)
}

fn decode(
    path: &Path,
    media_type: ffi::AVMediaType,
    mut receive: impl FnMut(&ffi::AVFrame) -> io::Result<()>,
) -> io::Result<()> {
    let input = Input::open(path)?;
    let mut decoder = Decoder {
        codec: ptr::null_mut(),
        packet: ptr::null_mut(),
        frame: ptr::null_mut(),
    };
    // SAFETY: `input` holds an opened context and `index` is a checked, non-negative stream
    // index from `av_find_best_stream`. Decoder allocations are null-checked and owned by
    // `decoder`, which frees them on any return; packets are unreferenced after use.
    unsafe {
        let index = ffi::av_find_best_stream(input.0, media_type, -1, -1, ptr::null_mut(), 0);
        check(index, "Find exported media stream")?;
        let stream = *(*input.0).streams.add(index as usize);
        let codec = ffi::avcodec_find_decoder((*(*stream).codecpar).codec_id);
        if codec.is_null() {
            return Err(io::Error::other("Missing inspection decoder"));
        }
        decoder.codec = ffi::avcodec_alloc_context3(codec);
        decoder.packet = ffi::av_packet_alloc();
        decoder.frame = ffi::av_frame_alloc();
        if decoder.codec.is_null() || decoder.packet.is_null() || decoder.frame.is_null() {
            return Err(io::Error::other("Cannot allocate inspection decoder"));
        }
        check(
            ffi::avcodec_parameters_to_context(decoder.codec, (*stream).codecpar),
            "Copy inspection decoder parameters",
        )?;
        check(
            ffi::avcodec_open2(decoder.codec, codec, ptr::null_mut()),
            "Open inspection decoder",
        )?;
        loop {
            let result = ffi::av_read_frame(input.0, decoder.packet);
            if result == ffi::AVERROR_EOF {
                break;
            }
            check(result, "Read exported packet")?;
            if (*decoder.packet).stream_index == index {
                check(
                    ffi::avcodec_send_packet(decoder.codec, decoder.packet),
                    "Submit exported media packet",
                )?;
                decoder.receive(&mut receive)?;
            }
            ffi::av_packet_unref(decoder.packet);
        }
        check(
            ffi::avcodec_send_packet(decoder.codec, ptr::null()),
            "Flush inspection decoder",
        )?;
        decoder.receive(&mut receive)?;
    }
    Ok(())
}

struct Input(*mut ffi::AVFormatContext);

impl Input {
    fn open(path: &Path) -> io::Result<Self> {
        let path = path.canonicalize()?;
        if !path.is_file() {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "Inspection requires a regular local file",
            ));
        }
        let filename = path_string(&path)?;
        let mut input = Self(ptr::null_mut());
        // SAFETY: `filename` outlives the open call, `options` is freed after it, and on failure
        // `avformat_open_input` leaves `input.0` null; on success `input` owns the context.
        unsafe {
            let mut options = ptr::null_mut();
            let configured = ffi::av_dict_set(
                &mut options,
                c"protocol_whitelist".as_ptr(),
                c"file".as_ptr(),
                0,
            );
            let opened = if configured < 0 {
                configured
            } else {
                ffi::avformat_open_input(&mut input.0, filename.as_ptr(), ptr::null(), &mut options)
            };
            ffi::av_dict_free(&mut options);
            check(opened, "Open local media for inspection")?;
            check(
                ffi::avformat_find_stream_info(input.0, ptr::null_mut()),
                "Inspect media streams",
            )?;
        }
        Ok(input)
    }
}

impl Drop for Input {
    fn drop(&mut self) {
        // SAFETY: `Input` owns the context; `avformat_close_input` accepts null and resets it.
        unsafe {
            ffi::avformat_close_input(&mut self.0);
        }
    }
}

struct Decoder {
    codec: *mut ffi::AVCodecContext,
    packet: *mut ffi::AVPacket,
    frame: *mut ffi::AVFrame,
}

impl Decoder {
    fn receive(
        &mut self,
        receive: &mut impl FnMut(&ffi::AVFrame) -> io::Result<()>,
    ) -> io::Result<()> {
        // SAFETY: `decode` only calls this after opening `codec` and allocating `frame`; the
        // frame reference passed to `receive` is valid until it is unreferenced afterwards.
        unsafe {
            loop {
                let result = ffi::avcodec_receive_frame(self.codec, self.frame);
                if again(result) || result == ffi::AVERROR_EOF {
                    return Ok(());
                }
                check(result, "Decode exported media frame")?;
                receive(&*self.frame)?;
                ffi::av_frame_unref(self.frame);
            }
        }
    }
}

impl Drop for Decoder {
    fn drop(&mut self) {
        // SAFETY: `Decoder` exclusively owns its pointers; the free functions accept null and
        // reset the fields.
        unsafe {
            ffi::av_packet_free(&mut self.packet);
            ffi::av_frame_free(&mut self.frame);
            ffi::avcodec_free_context(&mut self.codec);
        }
    }
}
