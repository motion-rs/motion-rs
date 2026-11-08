//! Container setup with metadata and chapters, packet draining and audio interleaving.

use std::ffi::CString;
use std::io;
use std::ptr;

use super::Encoder;
use crate::audio::{Audio, AudioInput, MIXER_SAMPLE_RATE};
use crate::settings::VideoCodec;
use crate::{TIMELINE_TICKS_PER_SECOND, again, check, ffi};

impl Encoder {
    pub(super) fn audio_samples(&self, frames: u32) -> io::Result<u64> {
        let rate = self
            .audio
            .as_ref()
            // SAFETY: `Audio::new` returns only with an allocated, opened codec context.
            .map_or(MIXER_SAMPLE_RATE, |audio| unsafe {
                (*audio.codec).sample_rate as u32
            });
        let samples = (u128::from(frames) * u128::from(rate) * u128::from(self.frame_rate.1))
            .div_ceil(u128::from(self.frame_rate.0));
        u64::try_from(samples).map_err(|_| {
            io::Error::new(
                io::ErrorKind::InvalidInput,
                "Video duration exceeds the audio sample range",
            )
        })
    }

    pub(super) fn open_container(
        &mut self,
        path: &CString,
        audio: Option<AudioInput<'_>>,
    ) -> io::Result<()> {
        // SAFETY: `format_context` is the live output context from `allocate_video`. Stream
        // indices stay below `nb_streams`. The chapter table is allocated with `av_calloc` for
        // `chapters.len()` entries and handed to the context before filling, and `nb_chapters`
        // only counts entries already stored, so `avformat_free_context` frees exactly those.
        // C strings outlive the calls, and `options` is freed after writing the header.
        unsafe {
            if let Some(mix) = audio {
                let total = (u128::from(self.expected_frames)
                    * u128::from(mix.sample_rate())
                    * u128::from(self.frame_rate.1))
                .div_ceil(u128::from(self.frame_rate.0));
                let total = u64::try_from(total).map_err(|_| {
                    io::Error::new(io::ErrorKind::InvalidInput, "Audio sample count overflow")
                })?;
                self.audio = Some(Audio::new(self.format_context, mix, total)?);
            }
            for (key, value) in &self.settings.metadata.tags {
                let key = CString::new(key.as_str()).map_err(io::Error::other)?;
                let value = CString::new(value.as_str()).map_err(io::Error::other)?;
                check(
                    ffi::av_dict_set(
                        &mut (*self.format_context).metadata,
                        key.as_ptr(),
                        value.as_ptr(),
                        0,
                    ),
                    "Set container tag",
                )?;
                if key.as_c_str() == c"language" {
                    for index in 0..(*self.format_context).nb_streams as usize {
                        let stream = *(*self.format_context).streams.add(index);
                        check(
                            ffi::av_dict_set(
                                &mut (*stream).metadata,
                                key.as_ptr(),
                                value.as_ptr(),
                                0,
                            ),
                            "Set stream language",
                        )?;
                    }
                }
            }
            let chapters = &self.settings.metadata.chapters;
            if !chapters.is_empty() {
                let table =
                    ffi::av_calloc(chapters.len(), std::mem::size_of::<*mut ffi::AVChapter>())
                        .cast::<*mut ffi::AVChapter>();
                if table.is_null() {
                    return Err(io::Error::other("Cannot allocate chapter table"));
                }
                (*self.format_context).chapters = table;
                for (index, chapter) in chapters.iter().enumerate() {
                    if i128::from(chapter.end) * i128::from(self.frame_rate.0)
                        > i128::from(self.expected_frames)
                            * i128::from(TIMELINE_TICKS_PER_SECOND)
                            * i128::from(self.frame_rate.1)
                    {
                        return Err(io::Error::new(
                            io::ErrorKind::InvalidInput,
                            "Chapter exceeds output duration",
                        ));
                    }
                    let entry = ffi::av_calloc(1, std::mem::size_of::<ffi::AVChapter>())
                        .cast::<ffi::AVChapter>();
                    if entry.is_null() {
                        return Err(io::Error::other("Cannot allocate chapter"));
                    }
                    *table.add(index) = entry;
                    (*self.format_context).nb_chapters += 1;
                    (*entry).id = index as i64;
                    (*entry).time_base = ffi::AVRational {
                        num: 1,
                        den: TIMELINE_TICKS_PER_SECOND,
                    };
                    (*entry).start = chapter.start;
                    (*entry).end = chapter.end;
                    let title = CString::new(chapter.title.as_str()).map_err(io::Error::other)?;
                    check(
                        ffi::av_dict_set(
                            &mut (*entry).metadata,
                            c"title".as_ptr(),
                            title.as_ptr(),
                            0,
                        ),
                        "Set chapter title",
                    )?;
                }
            }
            check(
                ffi::avio_open(
                    &mut (*self.format_context).pb,
                    path.as_ptr(),
                    ffi::AVIO_FLAG_WRITE as i32,
                ),
                "Open local MP4 file",
            )?;
            let mut options = ptr::null_mut();
            let configured = if matches!(self.codec, VideoCodec::Vp9 { .. }) {
                0
            } else {
                ffi::av_dict_set(
                    &mut options,
                    c"movflags".as_ptr(),
                    if self.settings.metadata.tags.is_empty() {
                        c"+faststart".as_ptr()
                    } else {
                        c"+faststart+use_metadata_tags".as_ptr()
                    },
                    0,
                )
            };
            let configured = if configured < 0 || matches!(self.codec, VideoCodec::Vp9 { .. }) {
                configured
            } else {
                ffi::av_dict_set(
                    &mut options,
                    c"movie_timescale".as_ptr(),
                    // Matches `TIMELINE_TICKS_PER_SECOND`.
                    c"705600000".as_ptr(),
                    0,
                )
            };
            let result = if configured < 0 {
                configured
            } else {
                ffi::avformat_write_header(self.format_context, &mut options)
            };
            ffi::av_dict_free(&mut options);
            check(result, "Write MP4 header")?;
        }
        Ok(())
    }

    pub(super) fn drain(
        &mut self,
        codec: *mut ffi::AVCodecContext,
        stream: *mut ffi::AVStream,
    ) -> io::Result<()> {
        // SAFETY: `codec` and `stream` are the opened context and its stream from this encoder
        // (video or `self.audio`), both alive for the call. `packet` and `format_context` are
        // owned by `self`, and the packet is unreferenced after each mux.
        unsafe {
            loop {
                let result = ffi::avcodec_receive_packet(codec, self.packet);
                if again(result) || result == ffi::AVERROR_EOF {
                    return Ok(());
                }
                check(result, "Receive encoded packet")?;
                ffi::av_packet_rescale_ts(self.packet, (*codec).time_base, (*stream).time_base);
                (*self.packet).stream_index = (*stream).index;
                let result = ffi::av_interleaved_write_frame(self.format_context, self.packet);
                ffi::av_packet_unref(self.packet);
                check(result, "Mux encoded packet")?;
            }
        }
    }

    pub(super) fn write_audio_until(
        &mut self,
        samples: u64,
        check_cancelled: &mut impl FnMut() -> io::Result<()>,
    ) -> io::Result<()> {
        loop {
            let Some(audio) = self.audio.as_mut() else {
                return Ok(());
            };
            if audio.samples >= samples {
                return Ok(());
            }
            check_cancelled()?;
            let Some(frame) = audio.next_frame()? else {
                return Ok(());
            };
            let codec = audio.codec;
            let stream = audio.stream;
            // SAFETY: `codec` is the opened AAC context owned by `self.audio`; `frame` is that
            // audio's frame, filled by `next_frame` and still owned by it.
            unsafe {
                check(ffi::avcodec_send_frame(codec, frame), "Submit audio frame")?;
            }
            self.drain(codec, stream)?;
        }
    }
}
