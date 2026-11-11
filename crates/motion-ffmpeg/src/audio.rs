//! FFmpeg audio filter graphs that mix local sources and PCM callbacks into the encoder's
//! audio stream, plus the RAII wrappers for graph endpoints and option dictionaries.

use std::ffi::{CStr, CString};
use std::io;
use std::path::PathBuf;
use std::ptr;

use super::{check, ffi, path_string};

/// Output rate of the file mixer graph, and the AAC rate used for file-based audio.
pub(super) const MIXER_SAMPLE_RATE: u32 = 48_000;

/// Local sources and a sample-based graph with inputs `[1:a:0]`, `[2:a:0]`, etc.
/// The graph must have exactly one output named `[motion_audio]`.
pub struct AudioMix<'a> {
    /// Files are passed as dictionary values, never interpolated into the graph.
    pub sources: &'a [PathBuf],
    /// An internally constructed libavfilter graph, not a command line.
    pub filter_graph: &'a str,
}

/// Owned, bounded interleaved stereo PCM callback, without a temporary media file.
pub trait PcmSource: Send {
    /// Reads complete frames at an absolute source position, returning a frame count.
    /// Short reads are padded with silence; nonfinite values are rejected.
    fn read(&mut self, start: u64, output: &mut [f32]) -> io::Result<usize>;
}

/// Native AAC input at an explicitly selected supported sample rate.
pub struct AudioPcm {
    /// 44100, 48000 or 96000 Hz.
    pub sample_rate: u32,
    /// Owned source retained through encoder finalization.
    pub source: Box<dyn PcmSource>,
}

pub(super) enum AudioInput<'a> {
    File(AudioMix<'a>),
    Pcm(AudioPcm),
}

impl AudioInput<'_> {
    pub(super) fn sample_rate(&self) -> u32 {
        match self {
            Self::File(_) => MIXER_SAMPLE_RATE,
            Self::Pcm(input) => input.sample_rate,
        }
    }
}

pub(super) struct Audio {
    pub(super) codec: *mut ffi::AVCodecContext,
    pub(super) stream: *mut ffi::AVStream,
    mixer: Option<Mixer>,
    pcm: Option<AudioPcm>,
    frame: *mut ffi::AVFrame,
    total: u64,
    pub(super) samples: u64,
}

/// A bounded streaming 48 kHz stereo mixer shared by export and audio preview.
pub struct Mixer {
    graph: *mut ffi::AVFilterGraph,
    sink: *mut ffi::AVFilterContext,
    frame: *mut ffi::AVFrame,
    samples: u64,
    total: u64,
}

impl Audio {
    pub(super) fn new(
        format: *mut ffi::AVFormatContext,
        input: AudioInput<'_>,
        total: u64,
    ) -> io::Result<Self> {
        let rate = input.sample_rate();
        if ![44100, 48000, 96000].contains(&rate) || total == 0 || total > 86400 * u64::from(rate) {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "Invalid native PCM rate or duration",
            ));
        }
        let (mixer, pcm) = match input {
            AudioInput::File(mix) => (Some(Mixer::new(mix, total)?), None),
            AudioInput::Pcm(pcm) => (None, Some(pcm)),
        };
        let mut audio = Self {
            codec: ptr::null_mut(),
            stream: ptr::null_mut(),
            mixer,
            pcm,
            frame: ptr::null_mut(),
            total,
            samples: 0,
        };
        // SAFETY: `format` is the caller's live output context with `oformat` set by
        // `avformat_alloc_output_context2`. Codec context and stream are null-checked before any
        // field access, and every allocation is owned by `audio`, whose `Drop` frees it on early
        // return. The stream itself is owned and freed by `format`.
        unsafe {
            let codec = ffi::avcodec_find_encoder_by_name(c"aac".as_ptr());
            if codec.is_null() {
                return Err(io::Error::other("FFmpeg libraries do not provide AAC"));
            }
            audio.codec = ffi::avcodec_alloc_context3(codec);
            audio.stream = ffi::avformat_new_stream(format, ptr::null());
            if audio.codec.is_null() || audio.stream.is_null() {
                return Err(io::Error::other("Cannot allocate native audio resources"));
            }
            (*audio.codec).sample_rate = rate as i32;
            (*audio.codec).sample_fmt = ffi::AV_SAMPLE_FMT_FLTP;
            (*audio.codec).bit_rate = 192_000;
            (*audio.codec).time_base = ffi::AVRational {
                num: 1,
                den: rate as i32,
            };
            ffi::av_channel_layout_default(&mut (*audio.codec).ch_layout, 2);
            if (*(*format).oformat).flags & ffi::AVFMT_GLOBALHEADER as i32 != 0 {
                (*audio.codec).flags |= ffi::AV_CODEC_FLAG_GLOBAL_HEADER as i32;
            }
            check(
                ffi::avcodec_open2(audio.codec, codec, ptr::null_mut()),
                "Open AAC encoder",
            )?;
            (*audio.stream).time_base = (*audio.codec).time_base;
            check(
                ffi::avcodec_parameters_from_context((*audio.stream).codecpar, audio.codec),
                "Copy audio parameters",
            )?;
            if audio.pcm.is_some() {
                audio.frame = ffi::av_frame_alloc();
                if audio.frame.is_null() {
                    return Err(io::Error::other("Cannot allocate PCM frame"));
                }
            }
        }
        Ok(audio)
    }

    pub(super) fn next_frame(&mut self) -> io::Result<Option<*mut ffi::AVFrame>> {
        // SAFETY: `Audio::new` only returns after `self.codec` was allocated, null-checked and
        // opened, and the context lives until `Drop`.
        let count = unsafe { (*self.codec).frame_size };
        if count <= 0 {
            return Err(io::Error::other("AAC encoder has no frame size"));
        }
        if let Some(mixer) = &mut self.mixer {
            let frame = mixer.read_frame(count)?;
            self.samples = mixer.samples;
            return Ok(frame);
        }
        if self.samples >= self.total {
            return Ok(None);
        }
        let count = (count as u64).min(self.total - self.samples) as usize;
        let mut samples = vec![0.0; count * 2];
        let pcm = self
            .pcm
            .as_mut()
            .ok_or_else(|| io::Error::other("Missing PCM input"))?;
        let got = pcm.source.read(self.samples, &mut samples)?;
        if got > count || samples[..got * 2].iter().any(|sample| !sample.is_finite()) {
            return Err(io::Error::other(
                "Invalid PCM callback frame count or samples",
            ));
        }
        samples[got * 2..].fill(0.0);
        // SAFETY: PCM input implies `self.frame` was allocated and null-checked in `Audio::new`.
        // `av_frame_get_buffer` succeeded for FLTP stereo with `count` samples, so planes 0 and 1
        // each hold at least `count` aligned f32 values, and `samples` holds `count * 2` values.
        unsafe {
            ffi::av_frame_unref(self.frame);
            (*self.frame).format = ffi::AV_SAMPLE_FMT_FLTP;
            (*self.frame).sample_rate = pcm.sample_rate as i32;
            (*self.frame).nb_samples = count as i32;
            ffi::av_channel_layout_default(&mut (*self.frame).ch_layout, 2);
            check(
                ffi::av_frame_get_buffer(self.frame, 32),
                "Allocate PCM sample planes",
            )?;
            for channel in 0..2 {
                let plane = std::slice::from_raw_parts_mut(
                    (*self.frame).data[channel].cast::<f32>(),
                    count,
                );
                for (index, sample) in plane.iter_mut().enumerate() {
                    *sample = samples[index * 2 + channel];
                }
            }
            (*self.frame).pts = self.samples as i64;
            (*self.frame).duration = count as i64;
        }
        self.samples += count as u64;
        Ok(Some(self.frame))
    }
}

impl Mixer {
    /// Opens trusted local sources using a finite graph and at most 24 hours of output.
    pub fn new(mix: AudioMix<'_>, total: u64) -> io::Result<Self> {
        if mix.sources.is_empty()
            || mix.sources.len() > 32
            || total == 0
            || total > 86_400 * u64::from(MIXER_SAMPLE_RATE)
        {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "Invalid audio source count or duration",
            ));
        }
        let mut mixer = Self {
            graph: ptr::null_mut(),
            sink: ptr::null_mut(),
            frame: ptr::null_mut(),
            samples: 0,
            total,
        };
        // SAFETY: Both allocations are null-checked before `graph` is dereferenced; `mixer` owns
        // them from this point and frees them in `Drop` on any early return.
        unsafe {
            mixer.graph = ffi::avfilter_graph_alloc();
            mixer.frame = ffi::av_frame_alloc();
            if mixer.graph.is_null() || mixer.frame.is_null() {
                return Err(io::Error::other("Cannot allocate audio mixer"));
            }
            (*mixer.graph).nb_threads = 1;
        }
        mixer.connect(mix)?;
        Ok(mixer)
    }

    /// Reads up to 4096 stereo sample frames into caller-owned storage; zero means EOF.
    pub fn read(&mut self, output: &mut [[f32; 2]]) -> io::Result<usize> {
        if output.is_empty() || output.len() > 4096 {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "Mixer reads require 1..=4096 sample frames",
            ));
        }
        let Some(frame) = self.read_frame(output.len() as i32)? else {
            return Ok(0);
        };
        // SAFETY: `read_frame` returns `self.frame`, which stays allocated and is not touched
        // again until the next read. Planes are verified non-null FLTP stereo, and `read_frame`
        // bounds `nb_samples` to `1..=output.len()`, so each plane holds `count` f32 values that
        // also fit in `output`.
        unsafe {
            let frame = &*frame;
            if frame.format != ffi::AV_SAMPLE_FMT_FLTP
                || frame.ch_layout.nb_channels != 2
                || frame.data[0].is_null()
                || frame.data[1].is_null()
            {
                return Err(io::Error::other("Invalid mixed sample planes"));
            }
            let count = frame.nb_samples as usize;
            let left = std::slice::from_raw_parts(frame.data[0].cast::<f32>(), count);
            let right = std::slice::from_raw_parts(frame.data[1].cast::<f32>(), count);
            for (index, pair) in output[..count].iter_mut().enumerate() {
                *pair = [left[index], right[index]];
                if !pair.iter().all(|sample| sample.is_finite()) {
                    return Err(io::Error::other("Audio mixer produced non-finite samples"));
                }
            }
            Ok(count)
        }
    }

    /// Discards an exact number of output samples, checking cancellation between pulls.
    /// This is bounded-memory decoding, not constant-time random access.
    pub fn skip(&mut self, samples: u64, mut cancelled: impl FnMut() -> bool) -> io::Result<()> {
        let target = self
            .samples
            .checked_add(samples)
            .filter(|target| *target <= self.total)
            .ok_or_else(|| {
                io::Error::new(io::ErrorKind::InvalidInput, "Mixer seek exceeds duration")
            })?;
        while self.samples < target {
            if cancelled() {
                return Err(io::Error::new(
                    io::ErrorKind::Interrupted,
                    "Audio seek cancelled",
                ));
            }
            self.read_frame((target - self.samples).min(4096) as i32)?;
        }
        Ok(())
    }

    fn connect(&mut self, mix: AudioMix<'_>) -> io::Result<()> {
        let description = CString::new(mix.filter_graph)
            .map_err(|error| io::Error::new(io::ErrorKind::InvalidInput, error))?;
        let mut ends = GraphEnds {
            inputs: ptr::null_mut(),
            outputs: ptr::null_mut(),
        };
        // SAFETY: `self.graph` is the live graph allocated in `new`, and `description` outlives
        // the parse call. The in/out lists returned by the parser are owned by `ends` and freed on
        // drop; every list node and its `name` are null-checked before being read, and the filter
        // contexts they reference belong to `self.graph`. The sink is created in the same graph.
        unsafe {
            check(
                ffi::avfilter_graph_parse2(
                    self.graph,
                    description.as_ptr(),
                    &mut ends.inputs,
                    &mut ends.outputs,
                ),
                "Parse audio graph",
            )?;
            let sink = ffi::avfilter_get_by_name(c"abuffersink".as_ptr());
            if sink.is_null() {
                return Err(io::Error::other(
                    "FFmpeg libraries do not provide abuffersink",
                ));
            }
            check(
                ffi::avfilter_graph_create_filter(
                    &mut self.sink,
                    sink,
                    c"motion_sink".as_ptr(),
                    ptr::null(),
                    ptr::null_mut(),
                    self.graph,
                ),
                "Create audio sink",
            )?;
            let mut used = vec![false; mix.sources.len()];
            let mut input = ends.inputs;
            while !input.is_null() {
                if (*input).name.is_null() {
                    return Err(io::Error::other("Audio graph has an unnamed input"));
                }
                let label = CStr::from_ptr((*input).name).to_bytes();
                let index = (0..mix.sources.len())
                    .find(|index| label == format!("{}:a:0", index + 1).as_bytes())
                    .ok_or_else(|| io::Error::other("Unexpected audio graph input"))?;
                if used[index] {
                    return Err(io::Error::other("Duplicate audio graph input"));
                }
                used[index] = true;
                let source = self.source(index, &mix.sources[index]).map_err(|error| {
                    io::Error::new(
                        error.kind(),
                        format!(
                            "Audio clip {} ({}): {error}",
                            index + 1,
                            mix.sources[index].display()
                        ),
                    )
                })?;
                check(
                    ffi::avfilter_link(source, 0, (*input).filter_ctx, (*input).pad_idx as u32),
                    "Connect audio source",
                )?;
                input = (*input).next;
            }
            if used.contains(&false) {
                return Err(io::Error::other("Audio graph does not use every source"));
            }
            let output = ends.outputs;
            if output.is_null()
                || !(*output).next.is_null()
                || (*output).name.is_null()
                || CStr::from_ptr((*output).name) != c"motion_audio"
            {
                return Err(io::Error::other(
                    "Audio graph requires one motion_audio output",
                ));
            }
            check(
                ffi::avfilter_link((*output).filter_ctx, (*output).pad_idx as u32, self.sink, 0),
                "Connect audio sink",
            )?;
            check(
                ffi::avfilter_graph_config(self.graph, ptr::null_mut()),
                "Configure audio graph",
            )?;
            if ffi::av_buffersink_get_format(self.sink) != ffi::AV_SAMPLE_FMT_FLTP
                || ffi::av_buffersink_get_sample_rate(self.sink) != MIXER_SAMPLE_RATE as i32
                || ffi::av_buffersink_get_channels(self.sink) != 2
            {
                return Err(io::Error::other(
                    "Audio graph must output 48 kHz planar float stereo",
                ));
            }
        }
        Ok(())
    }

    fn source(
        &mut self,
        index: usize,
        path: &std::path::Path,
    ) -> io::Result<*mut ffi::AVFilterContext> {
        let path = std::fs::canonicalize(path)?;
        if !path.is_file() {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "Audio source must be a regular local file",
            ));
        }
        let filename = path_string(&path)?;
        let name = CString::new(format!("motion_source_{index}"))
            .map_err(|error| io::Error::new(io::ErrorKind::InvalidInput, error))?;
        // SAFETY: The filter definition and the allocated context are null-checked, and the
        // context is owned by the live `self.graph`. `options` owns its dictionary (freed on drop)
        // and all C strings outlive the calls that read them.
        unsafe {
            let filter = ffi::avfilter_get_by_name(c"amovie".as_ptr());
            if filter.is_null() {
                return Err(io::Error::other("FFmpeg libraries do not provide amovie"));
            }
            let source = ffi::avfilter_graph_alloc_filter(self.graph, filter, name.as_ptr());
            if source.is_null() {
                return Err(io::Error::other("Cannot allocate audio source"));
            }
            let mut options = Dictionary(ptr::null_mut());
            options.set(c"filename", &filename)?;
            options.set(c"streams", c"a:0")?;
            options.set(c"format_opts", c"protocol_whitelist=file")?;
            options.set(c"dec_threads", c"1")?;
            check(
                ffi::avfilter_init_dict(source, &mut options.0),
                "Open local audio source",
            )?;
            if ffi::av_dict_count(options.0) != 0 {
                return Err(io::Error::other("Unrecognized audio source options"));
            }
            Ok(source)
        }
    }

    fn read_frame(&mut self, requested: i32) -> io::Result<Option<*mut ffi::AVFrame>> {
        if self.samples == self.total {
            return Ok(None);
        }
        // SAFETY: `self.frame` and `self.sink` were allocated and null-checked in `new`/`connect`
        // and stay valid until `Drop`; the frame is only accessed after the sink filled it.
        unsafe {
            ffi::av_frame_unref(self.frame);
            let requested = (requested as u64).min(self.total - self.samples) as i32;
            check(
                ffi::av_buffersink_get_samples(self.sink, self.frame, requested),
                "Read mixed audio samples",
            )?;
            let count = (*self.frame).nb_samples;
            if count <= 0 || count > requested {
                return Err(io::Error::other("Unexpected mixed audio frame length"));
            }
            (*self.frame).pts = self.samples as i64;
            (*self.frame).duration = i64::from(count);
            self.samples += count as u64;
            Ok(Some(self.frame))
        }
    }
}

impl Drop for Audio {
    fn drop(&mut self) {
        // SAFETY: `Audio` exclusively owns both pointers; the free functions accept null and
        // reset the fields, so partially constructed values are released exactly once.
        unsafe {
            ffi::avcodec_free_context(&mut self.codec);
            ffi::av_frame_free(&mut self.frame);
        }
    }
}

impl Drop for Mixer {
    fn drop(&mut self) {
        // SAFETY: `Mixer` exclusively owns the graph (including its filter contexts) and frame;
        // the free functions accept null and reset the fields.
        unsafe {
            ffi::avfilter_graph_free(&mut self.graph);
            ffi::av_frame_free(&mut self.frame);
        }
    }
}

struct GraphEnds {
    inputs: *mut ffi::AVFilterInOut,
    outputs: *mut ffi::AVFilterInOut,
}

impl Drop for GraphEnds {
    fn drop(&mut self) {
        // SAFETY: The lists were returned by `avfilter_graph_parse2`, which transfers ownership to
        // the caller; `avfilter_inout_free` accepts null and resets the fields.
        unsafe {
            ffi::avfilter_inout_free(&mut self.inputs);
            ffi::avfilter_inout_free(&mut self.outputs);
        }
    }
}

struct Dictionary(*mut ffi::AVDictionary);

impl Dictionary {
    fn set(&mut self, key: &CStr, value: &CStr) -> io::Result<()> {
        check(
            // SAFETY: `self.0` is null or a dictionary owned by `self`; key and value are valid C
            // strings that FFmpeg copies (flags 0).
            unsafe { ffi::av_dict_set(&mut self.0, key.as_ptr(), value.as_ptr(), 0) },
            "Set audio source option",
        )
    }
}

impl Drop for Dictionary {
    fn drop(&mut self) {
        // SAFETY: `Dictionary` owns its pointer, which is null or allocated by `av_dict_set`.
        unsafe {
            ffi::av_dict_free(&mut self.0);
        }
    }
}
