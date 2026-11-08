//! Video codec selection, encoder context configuration and frame/scaler allocation.

use std::ffi::CString;
use std::io;
use std::ptr;

use super::Encoder;
use crate::settings::{H265Profile, VideoCodec};
use crate::{check, ffi};

impl Encoder {
    pub(super) fn allocate_video(&mut self, path: &CString) -> io::Result<*const ffi::AVCodec> {
        // SAFETY: Every allocation is null-checked before its fields are written, and `self` owns
        // them so `Drop` frees them on early return. `oformat` is set by a successful
        // `avformat_alloc_output_context2`. The stream is owned by `format_context`.
        unsafe {
            check(
                ffi::avformat_alloc_output_context2(
                    &mut self.format_context,
                    ptr::null(),
                    self.codec.container().as_ptr(),
                    path.as_ptr(),
                ),
                "Allocate MP4 output",
            )?;
            let codec = ffi::avcodec_find_encoder_by_name(self.codec.encoder().as_ptr());
            if codec.is_null() {
                return Err(io::Error::other("FFmpeg libraries do not provide libx264"));
            }
            self.video_context = ffi::avcodec_alloc_context3(codec);
            self.frame = ffi::av_frame_alloc();
            self.rgb_frame = ffi::av_frame_alloc();
            self.packet = ffi::av_packet_alloc();
            self.stream = ffi::avformat_new_stream(self.format_context, ptr::null());
            if self.video_context.is_null()
                || self.frame.is_null()
                || self.rgb_frame.is_null()
                || self.packet.is_null()
                || self.stream.is_null()
            {
                return Err(io::Error::other("Cannot allocate native video resources"));
            }
            (*self.video_context).width = self.width as i32;
            (*self.video_context).height = self.height as i32;
            (*self.video_context).pix_fmt = self.codec.pixel_format();
            (*self.video_context).color_primaries = ffi::AVCOL_PRI_BT709;
            (*self.video_context).color_trc = if self.settings.gamma24 {
                ffi::AVCOL_TRC_BT709
            } else {
                ffi::AVCOL_TRC_IEC61966_2_1
            };
            (*self.video_context).colorspace = ffi::AVCOL_SPC_BT709;
            (*self.video_context).color_range = ffi::AVCOL_RANGE_MPEG;
            (*self.video_context).time_base = ffi::AVRational {
                num: self.frame_rate.1 as i32,
                den: self.frame_rate.0 as i32,
            };
            (*self.video_context).framerate = ffi::AVRational {
                num: self.frame_rate.0 as i32,
                den: self.frame_rate.1 as i32,
            };
            if (*(*self.format_context).oformat).flags & ffi::AVFMT_GLOBALHEADER as i32 != 0 {
                (*self.video_context).flags |= ffi::AV_CODEC_FLAG_GLOBAL_HEADER as i32;
            }
            Ok(codec)
        }
    }

    pub(super) fn configure_codec(&mut self, quality: &CString) -> io::Result<()> {
        // SAFETY: `allocate_video` allocated `video_context` for the selected encoder, so
        // `priv_data` holds that encoder's option struct. All option names and values are valid
        // C strings that outlive the `av_opt_set` calls, which copy them.
        unsafe {
            match self.codec {
                VideoCodec::H264 | VideoCodec::H265 { .. } => {
                    let preset = CString::new(self.settings.preset)
                        .map_err(|error| io::Error::new(io::ErrorKind::InvalidInput, error))?;
                    check(
                        ffi::av_opt_set(
                            (*self.video_context).priv_data,
                            c"preset".as_ptr(),
                            preset.as_ptr(),
                            0,
                        ),
                        "Set codec preset",
                    )?;
                    check(
                        ffi::av_opt_set(
                            (*self.video_context).priv_data,
                            c"crf".as_ptr(),
                            quality.as_ptr(),
                            0,
                        ),
                        "Set codec quality",
                    )?;
                    if matches!(
                        self.codec,
                        VideoCodec::H265 {
                            profile: H265Profile::Main10,
                        }
                    ) {
                        check(
                            ffi::av_opt_set(
                                (*self.video_context).priv_data,
                                c"profile".as_ptr(),
                                c"main10".as_ptr(),
                                0,
                            ),
                            "Set HEVC Main 10 profile",
                        )?;
                    }
                    if let Some(parameters) = &self.settings.x265_params {
                        let parameters = CString::new(parameters.as_str())
                            .map_err(|error| io::Error::new(io::ErrorKind::InvalidInput, error))?;
                        check(
                            ffi::av_opt_set(
                                (*self.video_context).priv_data,
                                c"x265-params".as_ptr(),
                                parameters.as_ptr(),
                                0,
                            ),
                            "Set x265 parameters",
                        )?;
                    }
                }
                VideoCodec::Vp9 { alpha } => {
                    (*self.video_context).bit_rate = 0;
                    check(
                        ffi::av_opt_set(
                            (*self.video_context).priv_data,
                            c"crf".as_ptr(),
                            quality.as_ptr(),
                            0,
                        ),
                        "Set VP9 quality",
                    )?;
                    if alpha {
                        check(
                            ffi::av_opt_set(
                                (*self.video_context).priv_data,
                                c"auto-alt-ref".as_ptr(),
                                c"0".as_ptr(),
                                0,
                            ),
                            "Disable VP9 alternate reference for alpha",
                        )?;
                    }
                }
                VideoCodec::ProRes4444 => {
                    check(
                        ffi::av_opt_set(
                            (*self.video_context).priv_data,
                            c"profile".as_ptr(),
                            c"4".as_ptr(),
                            0,
                        ),
                        "Set ProRes 4444 profile",
                    )?;
                    check(
                        ffi::av_opt_set(
                            (*self.video_context).priv_data,
                            c"alpha_bits".as_ptr(),
                            c"16".as_ptr(),
                            0,
                        ),
                        "Set ProRes alpha depth",
                    )?;
                }
            }
        }
        Ok(())
    }

    pub(super) fn allocate_video_frames(&mut self, codec: *const ffi::AVCodec) -> io::Result<()> {
        // SAFETY: `codec`, `video_context`, `stream`, `frame` and `rgb_frame` were allocated and
        // null-checked by `allocate_video`. Frame buffers are allocated from the dimensions set
        // here, and the scaler is null-checked before configuration; `Drop` frees it.
        unsafe {
            check(
                ffi::avcodec_open2(self.video_context, codec, ptr::null_mut()),
                "Open H.264 encoder",
            )?;
            (*self.stream).time_base = (*self.video_context).time_base;
            (*self.stream).avg_frame_rate = (*self.video_context).framerate;
            check(
                ffi::avcodec_parameters_from_context((*self.stream).codecpar, self.video_context),
                "Copy video parameters",
            )?;
            (*self.frame).format = self.codec.pixel_format();
            (*self.frame).width = self.width as i32;
            (*self.frame).height = self.height as i32;
            (*self.frame).color_primaries = (*self.video_context).color_primaries;
            (*self.frame).color_trc = (*self.video_context).color_trc;
            (*self.frame).colorspace = (*self.video_context).colorspace;
            (*self.frame).color_range = (*self.video_context).color_range;
            check(
                ffi::av_frame_get_buffer(self.frame, 32),
                "Allocate YUV frame",
            )?;
            (*self.rgb_frame).format = if self.codec.alpha() {
                ffi::AV_PIX_FMT_RGBA
            } else {
                ffi::AV_PIX_FMT_RGB24
            };
            (*self.rgb_frame).width = self.width as i32;
            (*self.rgb_frame).height = self.height as i32;
            check(
                ffi::av_frame_get_buffer(self.rgb_frame, 32),
                "Allocate aligned RGB frame",
            )?;
            self.scale_context = ffi::sws_getContext(
                self.width as i32,
                self.height as i32,
                if self.codec.alpha() {
                    ffi::AV_PIX_FMT_RGBA
                } else {
                    ffi::AV_PIX_FMT_RGB24
                },
                self.width as i32,
                self.height as i32,
                self.codec.pixel_format(),
                ffi::SWS_BICUBIC as i32,
                ptr::null_mut(),
                ptr::null_mut(),
                ptr::null(),
            );
            if self.scale_context.is_null() {
                return Err(io::Error::other("Cannot create RGB/YUV converter"));
            }
            let coefficients = ffi::sws_getCoefficients(ffi::SWS_CS_ITU709 as i32);
            check(
                ffi::sws_setColorspaceDetails(
                    self.scale_context,
                    coefficients,
                    1,
                    coefficients,
                    0,
                    0,
                    1 << 16,
                    1 << 16,
                ),
                "Set BT.709 RGB/YUV matrix",
            )?;
        }
        Ok(())
    }
}
