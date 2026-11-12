//! Immutable animations with independent integer-tick frame lookup.

use crate::{Image, ImageAsset};

use image::{AnimationDecoder, ImageDecoder};
use motion_core::{Duration, FrameRate, ValidationError};
use std::{
    io::{Cursor, Read},
    sync::Arc,
};

const MAX_FRAMES: usize = 4096;

const MAX_BYTES: usize = 256 * 1024 * 1024;

fn invalid(field: &str, message: &str) -> ValidationError {
    ValidationError::invalid("animated_image", field, message)
}

/// Time mapping applied to a decoded animation, without modifying its frame data.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum ImageLoopMode {
    /// Honor encoded play count; hold the last frame afterwards.
    #[default]
    File,
    /// Play once and hold the final frame.
    Once,
    /// Repeat indefinitely.
    Loop,
    /// Reverse interior frames without duplicating endpoint durations.
    PingPong,
}

/// Fully composed immutable GIF/APNG frames and half-open display intervals.
/// Up to 4096 frames, 24 hours and 256 MiB mip data; no seek cursor.
#[derive(Clone, Debug)]
pub struct AnimatedImageAsset {
    frames: Arc<[Arc<ImageAsset>]>,
    ends: Arc<[Duration]>,
    plays: Option<u32>,
    bytes: usize,
}

impl AnimatedImageAsset {
    /// Freezes explicitly timed equal-sized images. Zero durations are instantaneous
    /// and never selected inside the cycle; total duration must be positive.
    pub fn from_frames(frames: Vec<(Arc<ImageAsset>, Duration)>) -> Result<Self, ValidationError> {
        if frames.is_empty() || frames.len() > MAX_FRAMES {
            return Err(invalid("frames", "Animation requires 1..=4096 frames"));
        }
        let size = (frames[0].0.width(), frames[0].0.height());
        let mut total = 0u64;
        let mut bytes = frames.len()
            * (std::mem::size_of::<Arc<ImageAsset>>() + std::mem::size_of::<Duration>());
        let mut seen = std::collections::BTreeSet::new();
        let mut ends = Vec::with_capacity(frames.len());
        let mut images = Vec::with_capacity(frames.len());
        for (frame, duration) in frames {
            if (frame.width(), frame.height()) != size {
                return Err(invalid("frames", "Animation dimensions must match"));
            }
            total = total
                .checked_add(duration.ticks())
                .ok_or_else(|| invalid("duration", "Animation time overflow"))?;
            if total > 86400 * motion_core::TICKS_PER_SECOND as u64 {
                return Err(invalid("duration", "Animation exceeds 24 hours"));
            }
            if seen.insert(Arc::as_ptr(&frame) as usize) {
                bytes = bytes
                    .checked_add(frame.retained_bytes())
                    .ok_or_else(|| invalid("bytes", "Animation storage overflow"))?;
            }
            if bytes > MAX_BYTES {
                return Err(invalid("bytes", "Animation mip data exceeds 256 MiB"));
            }
            ends.push(Duration::from_ticks(total));
            images.push(frame);
        }
        if total == 0 {
            return Err(invalid(
                "duration",
                "Animation total duration must be positive",
            ));
        }
        Ok(Self {
            frames: images.into(),
            ends: ends.into(),
            plays: Some(1),
            bytes,
        })
    }

    /// Creates a uniform sequence using independent rational frame boundaries.
    pub fn from_sequence(
        frames: Vec<Arc<ImageAsset>>,
        rate: FrameRate,
    ) -> Result<Self, ValidationError> {
        if frames.len() > MAX_FRAMES
            || u64::from(rate.numerator())
                > motion_core::TICKS_PER_SECOND as u64 * u64::from(rate.denominator())
        {
            return Err(invalid("frames", "Invalid sequence count/frame rate"));
        }
        let frames = frames
            .into_iter()
            .enumerate()
            .map(|(index, frame)| {
                let start = Duration::checked_from_frame(index as u64, rate)
                    .ok_or_else(|| invalid("duration", "Sequence time overflow"))?;
                let end = Duration::checked_from_frame(index as u64 + 1, rate)
                    .ok_or_else(|| invalid("duration", "Sequence time overflow"))?;
                Ok((frame, end - start))
            })
            .collect::<Result<Vec<_>, ValidationError>>()?;
        Self::from_frames(frames)
    }

    /// Decodes GIF/APNG using library disposal/blend rules. Delays round once to
    /// nearest ticks; no browser-specific minimum display delay is invented.
    pub fn from_bytes(bytes: &[u8]) -> motion_core::Result<Self> {
        if bytes.len() > 64 * 1024 * 1024 {
            return Err(invalid("encoded_bytes", "Animation source exceeds 64 MiB").into());
        }
        let mut limits = image::Limits::default();
        limits.max_image_width = Some(16384);
        limits.max_image_height = Some(16384);
        limits.max_alloc = Some(MAX_BYTES as u64);
        let cursor = Cursor::new(bytes);
        match image::guess_format(bytes).map_err(crate::image_error)? {
            image::ImageFormat::Gif => {
                let mut metadata = gif::DecodeOptions::new();
                metadata.skip_frame_decoding(true);
                let mut metadata = metadata.read_info(Cursor::new(bytes)).map_err(|error| {
                    motion_core::Error::asset("animated image", "<memory>", error)
                })?;
                let mut count = 0;
                while metadata
                    .read_next_frame()
                    .map_err(|error| {
                        motion_core::Error::asset("animated image", "<memory>", error)
                    })?
                    .is_some()
                {
                    count += 1;
                    if count > MAX_FRAMES {
                        return Err(invalid("frames", "Animation exceeds 4096 frames").into());
                    }
                }
                let plays = match metadata.repeat() {
                    gif::Repeat::Infinite => None,
                    gif::Repeat::Finite(repeats) => Some(u32::from(repeats) + 1),
                };
                let mut decoder =
                    image::codecs::gif::GifDecoder::new(cursor).map_err(crate::image_error)?;
                decoder.set_limits(limits).map_err(crate::image_error)?;
                Self::check_dimensions(decoder.dimensions())?;
                Self::decode(decoder.into_frames(), plays)
            }
            image::ImageFormat::Png => {
                let decoder = image::codecs::png::PngDecoder::with_limits(cursor, limits)
                    .map_err(crate::image_error)?;
                Self::check_dimensions(decoder.dimensions())?;
                if !decoder.is_apng().map_err(crate::image_error)? {
                    return Err(invalid("format", "Static PNG is not an animated image").into());
                }
                let decoder = decoder.apng().map_err(crate::image_error)?;
                let plays = loop_count(decoder.loop_count());
                Self::decode(decoder.into_frames(), plays)
            }
            _ => Err(invalid("format", "Animated images require GIF or APNG").into()),
        }
    }

    fn check_dimensions((width, height): (u32, u32)) -> Result<(), ValidationError> {
        if width == 0 || height == 0 || u64::from(width) * u64::from(height) > 16_777_216 {
            Err(invalid(
                "size",
                "Animation canvas exceeds 16 million pixels",
            ))
        } else {
            Ok(())
        }
    }

    fn decode(frames: image::Frames<'_>, plays: Option<u32>) -> motion_core::Result<Self> {
        let mut values = Vec::new();
        let mut bytes = 0usize;
        for frame in frames {
            if values.len() == MAX_FRAMES {
                return Err(invalid("frames", "Animation exceeds 4096 frames").into());
            }
            let frame = frame.map_err(crate::image_error)?;
            let (numerator, denominator) = frame.delay().numer_denom_ms();
            let divisor = u128::from(denominator) * 1000;
            let ticks = (u128::from(numerator) * motion_core::TICKS_PER_SECOND as u128
                + divisor / 2)
                / divisor;
            let duration = Duration::from_ticks(
                u64::try_from(ticks)
                    .map_err(|error| motion_core::Error::Parameters(Box::new(error)))?,
            );
            let rgba = frame.into_buffer();
            let estimate = rgba.width() as usize * rgba.height() as usize * 16;
            if estimate > MAX_BYTES.saturating_sub(bytes) {
                return Err(invalid("bytes", "Animation decoded mip allowance exhausted").into());
            }
            let asset = Arc::new(ImageAsset::from_rgba8(
                rgba.width(),
                rgba.height(),
                rgba.into_raw(),
            )?);
            bytes += asset.retained_bytes();
            values.push((asset, duration));
        }
        let mut asset = Self::from_frames(values)?;
        asset.plays = plays;
        Ok(asset)
    }

    /// Explicit bounded local-file loading; rendering never reopens the source.
    pub fn open(path: impl AsRef<std::path::Path>) -> motion_core::Result<Self> {
        let path = path.as_ref();
        let metadata = std::fs::metadata(path)?;
        if !metadata.is_file() || metadata.len() > 64 * 1024 * 1024 {
            return Err(invalid("source", "Animation must be a regular file <=64 MiB").into());
        }
        let mut bytes = Vec::new();
        std::fs::File::open(path)?
            .take(64 * 1024 * 1024 + 1)
            .read_to_end(&mut bytes)?;
        Self::from_bytes(&bytes).map_err(|error| {
            motion_core::Error::asset("animated image", path.display().to_string(), error)
        })
    }

    /// Dimensions shared by every composed frame.
    pub fn size(&self) -> motion_core::PixelSize {
        motion_core::PixelSize::new(self.frames[0].width(), self.frames[0].height())
    }

    /// Immutable original composed frames.
    pub fn frames(&self) -> &[Arc<ImageAsset>] {
        &self.frames
    }

    /// One cycle's actual rounded duration, not a video-frame count.
    pub fn duration(&self) -> Duration {
        *self.ends.last().expect("nonempty frames")
    }

    /// File-declared play count; None means indefinite repetition.
    pub fn file_plays(&self) -> Option<u32> {
        self.plays
    }

    /// Original individual frame duration in ticks.
    pub fn frame_duration(&self, index: usize) -> Option<Duration> {
        self.ends.get(index).map(|end| {
            *end - index
                .checked_sub(1)
                .map(|previous| self.ends[previous])
                .unwrap_or(Duration::ZERO)
        })
    }

    /// Selects the frame at an elapsed time; finite play holds the last frame afterwards.
    pub fn frame_index_at(&self, time: Duration, mode: ImageLoopMode) -> usize {
        let total = self.duration().ticks();
        let mut local = time.ticks();
        match mode {
            ImageLoopMode::Once => {
                if local >= total {
                    return self.frames.len() - 1;
                }
            }
            ImageLoopMode::File => {
                if self
                    .plays
                    .is_some_and(|plays| i128::from(local) >= i128::from(total) * i128::from(plays))
                {
                    return self.frames.len() - 1;
                }
                local %= total;
            }
            ImageLoopMode::Loop => local %= total,
            ImageLoopMode::PingPong => {
                let first = self.ends[0].ticks();
                let last = self
                    .frame_duration(self.frames.len() - 1)
                    .expect("last frame")
                    .ticks();
                let interior = total.saturating_sub(first + last);
                local %= total + interior;
                if local >= total {
                    local = total - last - 1 - (local - total);
                }
            }
        }
        self.ends
            .partition_point(|end| end.ticks() <= local)
            .min(self.frames.len() - 1)
    }

    /// Samples without I/O, allocation, decoding or state mutation.
    pub fn frame_at(&self, time: Duration, mode: ImageLoopMode) -> &Arc<ImageAsset> {
        &self.frames[self.frame_index_at(time, mode)]
    }
}

fn loop_count(count: image::metadata::LoopCount) -> Option<u32> {
    match count {
        image::metadata::LoopCount::Infinite => None,
        image::metadata::LoopCount::Finite(count) => Some(count.get()),
    }
}

impl motion_core::Asset for AnimatedImageAsset {
    const KIND: &'static str = "animated image";

    fn decode(bytes: &[u8], _: &motion_core::LoadContext) -> motion_core::Result<Self> {
        Self::from_bytes(bytes)
    }

    fn retained_bytes(&self) -> usize {
        self.bytes
    }
}

#[derive(Clone, Debug)]
enum Input {
    Lazy(motion_core::Handle<AnimatedImageAsset>),
    Loaded(Arc<AnimatedImageAsset>),
}

/// animated image using the static image fit/mip/paint pipeline.
#[derive(Clone, Debug)]
pub struct AnimatedImage {
    input: Input,
    time: Duration,
    mode: ImageLoopMode,
    size: Option<motion_core::Size>,
    fit: crate::Fit,
    radius: f64,
    sampling: crate::Sampling,
}

impl AnimatedImage {
    /// Captures a lazy resource without decoding.
    pub fn new(handle: motion_core::Handle<AnimatedImageAsset>) -> Self {
        Self::with_input(Input::Lazy(handle))
    }

    /// Captures immutable already-decoded sequence data.
    pub fn from_asset(asset: Arc<AnimatedImageAsset>) -> Self {
        Self::with_input(Input::Loaded(asset))
    }

    fn with_input(input: Input) -> Self {
        Self {
            input,
            time: Duration::ZERO,
            mode: ImageLoopMode::File,
            size: None,
            fit: crate::Fit::Contain,
            radius: 0.0,
            sampling: crate::Sampling::Trilinear,
        }
    }

    /// Sets source query time independently of output frame rate.
    pub fn animation_time(mut self, time: Duration) -> Self {
        self.time = time;
        self
    }

    /// Overrides encoded looping metadata.
    pub fn loop_mode(mut self, mode: ImageLoopMode) -> Self {
        self.mode = mode;
        self
    }

    /// Sets the stable local viewport.
    pub fn size(mut self, width: f64, height: f64) -> Self {
        self.size = Some(motion_core::Size::new(width, height));
        self
    }

    /// Selects centered fitting.
    pub fn fit(mut self, fit: crate::Fit) -> Self {
        self.fit = fit;
        self
    }

    /// Selects rounded viewport clipping.
    pub fn corner_radius(mut self, radius: f64) -> Self {
        self.radius = radius;
        self
    }

    /// Selects shared texture reconstruction.
    pub fn sampling(mut self, sampling: crate::Sampling) -> Self {
        self.sampling = sampling;
        self
    }

    fn asset(&self) -> motion_core::Result<Arc<AnimatedImageAsset>> {
        match &self.input {
            Input::Lazy(handle) => handle.get(),
            Input::Loaded(asset) => Ok(asset.clone()),
        }
    }

    fn picture(&self, image: Arc<ImageAsset>) -> Image {
        let mut image = Image::from_asset(image)
            .fit(self.fit)
            .corner_radius(self.radius)
            .sampling(self.sampling);
        if let Some(size) = self.size {
            image = image.size(size.width, size.height);
        }
        image
    }
}

impl motion_core::Drawable for AnimatedImage {
    fn bounds(
        &self,
        context: &motion_core::DrawContext,
    ) -> Result<motion_core::Rect, ValidationError> {
        if !self.radius.is_finite() || self.radius < 0.0 {
            return Err(invalid(
                "corner_radius",
                "Animated image corner radius must be finite and nonnegative",
            ));
        }
        match self.asset() {
            Ok(asset) => {
                motion_core::Drawable::bounds(&self.picture(asset.frames[0].clone()), context)
            }
            Err(_) => Ok(self
                .size
                .unwrap_or(motion_core::Size::new(64.0, 64.0))
                .to_rect()),
        }
    }

    fn draw(
        &self,
        context: &motion_core::DrawContext,
        canvas: &mut motion_core::Canvas<'_>,
    ) -> Result<(), ValidationError> {
        match self.asset() {
            Ok(asset) => motion_core::Drawable::draw(
                &self.picture(asset.frame_at(self.time, self.mode).clone()),
                context,
                canvas,
            ),
            Err(_) => canvas.fill(
                &motion_core::Drawable::bounds(self, context)?,
                motion_core::Color::rgb(255, 0, 255),
                motion_core::FillRule::NonZero,
            ),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::asset::AssetsExt;
    use motion_core::{Asset, TimeLiteral};
    use motion_render::RenderPrepare;

    fn image(color: [u8; 4]) -> Arc<ImageAsset> {
        Arc::new(ImageAsset::from_rgba8(2, 1, color.repeat(2)).unwrap())
    }

    #[test]
    fn variable_duration_sequences_have_exact_boundaries_loops_and_concurrent_seek()
    -> motion_core::Result<()> {
        let asset = Arc::new(AnimatedImageAsset::from_frames(vec![
            (image([255, 0, 0, 255]), 100.milliseconds()),
            (image([0, 255, 0, 255]), 300.milliseconds()),
            (image([0, 0, 255, 255]), 200.milliseconds()),
        ])?);
        for (time, index) in [
            (0, 0),
            (99, 0),
            (100, 1),
            (399, 1),
            (400, 2),
            (599, 2),
            (600, 0),
            (999, 1),
        ] {
            assert_eq!(
                asset.frame_index_at(time.milliseconds(), ImageLoopMode::Loop),
                index
            );
        }
        assert_eq!(
            asset.frame_index_at(600.milliseconds(), ImageLoopMode::Once),
            2
        );
        assert_eq!(
            asset.frame_index_at(899.milliseconds(), ImageLoopMode::PingPong),
            1
        );
        assert_eq!(
            asset.frame_index_at(900.milliseconds(), ImageLoopMode::PingPong),
            0
        );
        for _ in 0..3 {
            let shared = asset.clone();
            std::thread::spawn(move || {
                for tick in (0..1000).rev() {
                    assert_eq!(
                        shared.frame_index_at(tick.milliseconds(), ImageLoopMode::Loop),
                        shared.frame_index_at((tick + 1200).milliseconds(), ImageLoopMode::Loop)
                    );
                }
            })
            .join()
            .unwrap();
        }
        let sequence = AnimatedImageAsset::from_sequence(asset.frames.to_vec(), FrameRate::NTSC)?;
        assert_eq!(
            sequence.duration(),
            Duration::from_frame(3, FrameRate::NTSC)
        );
        assert!(AnimatedImageAsset::from_frames(vec![(image([0; 4]), Duration::ZERO)]).is_err());
        Ok(())
    }

    #[test]
    fn gif_and_apng_decode_composited_frames_delays_and_lazy_pixels() -> motion_core::Result<()> {
        let mut gif = Vec::new();
        {
            let mut encoder = image::codecs::gif::GifEncoder::new(&mut gif);
            encoder
                .set_repeat(image::codecs::gif::Repeat::Finite(2))
                .map_err(crate::image_error)?;
            for (color, delay) in [
                ([255, 0, 0, 255], 100),
                ([0, 255, 0, 255], 300),
                ([0, 0, 255, 255], 200),
            ] {
                encoder
                    .encode_frame(image::Frame::from_parts(
                        image::RgbaImage::from_pixel(2, 1, image::Rgba(color)),
                        0,
                        0,
                        image::Delay::from_numer_denom_ms(delay, 1),
                    ))
                    .map_err(crate::image_error)?;
            }
        }
        let animation = AnimatedImageAsset::from_bytes(&gif)?;
        assert_eq!(animation.duration(), 600.milliseconds());
        assert_eq!(animation.file_plays(), Some(3));
        let mut once = Vec::new();
        {
            let mut encoder = image::codecs::gif::GifEncoder::new(&mut once);
            encoder
                .encode_frame(image::Frame::from_parts(
                    image::RgbaImage::from_pixel(2, 1, image::Rgba([255, 0, 0, 255])),
                    0,
                    0,
                    image::Delay::from_numer_denom_ms(100, 1),
                ))
                .map_err(crate::image_error)?;
        }
        assert_eq!(AnimatedImageAsset::from_bytes(&once)?.file_plays(), Some(1));
        let mut apng = Vec::new();
        {
            let mut encoder = png::Encoder::new(&mut apng, 2, 1);
            encoder.set_color(png::ColorType::Rgba);
            encoder.set_depth(png::BitDepth::Eight);
            encoder
                .set_animated(2, 3)
                .map_err(|error| motion_core::Error::Parameters(Box::new(error)))?;
            let mut writer = encoder
                .write_header()
                .map_err(|error| motion_core::Error::Parameters(Box::new(error)))?;
            writer
                .set_frame_delay(1, 10)
                .map_err(|error| motion_core::Error::Parameters(Box::new(error)))?;
            writer
                .write_image_data(&[255, 0, 0, 255, 255, 0, 0, 255])
                .map_err(|error| motion_core::Error::Parameters(Box::new(error)))?;
            writer
                .set_frame_delay(3, 10)
                .map_err(|error| motion_core::Error::Parameters(Box::new(error)))?;
            writer
                .set_blend_op(png::BlendOp::Over)
                .map_err(|error| motion_core::Error::Parameters(Box::new(error)))?;
            writer
                .write_image_data(&[0, 0, 0, 0, 0, 0, 255, 255])
                .map_err(|error| motion_core::Error::Parameters(Box::new(error)))?;
        }
        let decoded = AnimatedImageAsset::from_bytes(&apng)?;
        assert_eq!(decoded.duration(), 400.milliseconds());
        assert_eq!(decoded.file_plays(), Some(3));
        assert!(decoded.retained_bytes() > 0);
        let directory = tempfile::tempdir()?;
        let path = directory.path().join("animation.png");
        std::fs::write(&path, apng)?;
        let resources = motion_core::Assets::new(directory.path())?;
        let handle = resources.animated_image("animation.png");
        let asset = handle.get()?;
        std::fs::remove_file(path)?;
        let picture = AnimatedImage::new(handle)
            .animation_time(100.milliseconds())
            .sampling(crate::Sampling::Nearest);
        let list = motion_core::Node::from(picture).compile(
            motion_core::Size::new(2.0, 1.0),
            motion_core::PixelSize::new(2, 1),
            motion_core::ValidationLimits::default(),
        )?;
        let frame = list
            .prepare(
                motion_core::ValidationLimits::default(),
                motion_core::RasterLimits::default(),
            )?
            .render_linear(motion_core::RasterLimits::default())?;
        assert_eq!(
            frame
                .to_format(
                    motion_render::PixelFormat::SrgbRgba8 {
                        premultiplied: false
                    },
                    motion_core::RasterLimits::default()
                )?
                .bytes(),
            &[255, 0, 0, 255, 0, 0, 255, 255]
        );
        assert!(Arc::strong_count(&asset) >= 1);
        Ok(())
    }

    #[cfg(feature = "gpu")]
    #[test]
    #[ignore = "requires an actual native GPU adapter; no fonts or encoders"]
    fn animated_outputs_match_cpu_across_loops_and_exact_ticks() -> motion_core::Result<()> {
        use motion_core::NodeModifiers;
        let asset = Arc::new(AnimatedImageAsset::from_frames(vec![
            (image([255, 0, 0, 128]), 100.milliseconds()),
            (image([0, 255, 0, 255]), 300.milliseconds()),
            (image([0, 0, 255, 192]), 200.milliseconds()),
        ])?);
        let composition = motion_render::Composition::new(
            24,
            16,
            motion_render::FrameClock::new(motion_core::fps!(10), 20),
        );
        let design = motion_core::Size::new(12.0, 8.0);
        let mut cpu = composition.clone().frame_renderer(design)?;
        let mut gpu = composition
            .configure(|options| options.backend(motion_render::BackendPreference::gpu()))
            .frame_renderer(design)?;
        let mut requests = 0;
        for mode in [
            ImageLoopMode::Once,
            ImageLoopMode::File,
            ImageLoopMode::Loop,
            ImageLoopMode::PingPong,
        ] {
            for format in [
                motion_render::PixelFormat::SrgbRgba8 {
                    premultiplied: false,
                },
                motion_render::PixelFormat::LinearRgba16Float,
                motion_render::PixelFormat::Yuv420 { bit_depth: 8 },
                motion_render::PixelFormat::Yuv420 { bit_depth: 10 },
            ] {
                for milliseconds in [600, 0, 99, 100, 399, 400, 599, 1800, 100] {
                    let build = |context: &motion_core::SceneContext<'_>| {
                        motion_core::Node::from(
                            AnimatedImage::from_asset(asset.clone())
                                .animation_time(context.elapsed())
                                .loop_mode(mode)
                                .size(10.0, 6.0)
                                .fit(crate::Fit::Fill)
                                .corner_radius(1.0),
                        )
                        .translate(0.75, 0.5)
                        .opacity(0.8)
                    };
                    let background = if matches!(format, motion_render::PixelFormat::Yuv420 { .. })
                    {
                        Some(motion_core::Color::BLACK)
                    } else {
                        None
                    };
                    let expected = cpu.render_output_at(
                        milliseconds.milliseconds().after_origin(),
                        background,
                        format,
                        build,
                    )?;
                    let actual = gpu.render_output_at(
                        milliseconds.milliseconds().after_origin(),
                        background,
                        format,
                        build,
                    )?;
                    for (reference, result) in expected.planes().iter().zip(actual.planes()) {
                        if format == motion_render::PixelFormat::LinearRgba16Float {
                            for (reference, result) in reference
                                .bytes()
                                .as_chunks::<2>()
                                .0
                                .iter()
                                .zip(result.bytes().as_chunks::<2>().0)
                            {
                                let reference =
                                    half::f16::from_bits(u16::from_le_bytes(*reference)).to_f32();
                                let result =
                                    half::f16::from_bits(u16::from_le_bytes(*result)).to_f32();
                                assert!((reference - result).abs() <= 0.001);
                            }
                        } else if format == (motion_render::PixelFormat::Yuv420 { bit_depth: 10 }) {
                            for (reference, result) in reference
                                .bytes()
                                .as_chunks::<2>()
                                .0
                                .iter()
                                .zip(result.bytes().as_chunks::<2>().0)
                            {
                                assert!(
                                    u16::from_le_bytes(*reference)
                                        .abs_diff(u16::from_le_bytes(*result))
                                        <= 1
                                );
                            }
                        } else {
                            let mismatch = reference
                                .bytes()
                                .iter()
                                .zip(result.bytes())
                                .enumerate()
                                .find(|(_, (reference, result))| reference.abs_diff(**result) > 1);
                            assert!(
                                mismatch.is_none(),
                                "mode={mode:?},format={format:?},time={milliseconds},mismatch={mismatch:?}"
                            );
                        }
                    }
                    assert_eq!(
                        actual,
                        gpu.render_output_at(
                            milliseconds.milliseconds().after_origin(),
                            background,
                            format,
                            build
                        )?
                    );
                    requests += 1;
                }
            }
        }
        assert_eq!(requests, 144);
        Ok(())
    }
}
