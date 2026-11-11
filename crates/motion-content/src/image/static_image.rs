//! Half-float static image assets and resource-backed drawables.
//!
//! ```
//! use motion_content::prelude::*;
//! use motion_core::prelude::*;
//! use motion_core::{
//!     DisplayList, RasterLimits, ResourcePool, ValidatedDisplayList, MAX_PATH_VERBS,
//!     MAX_STROKE_VERBS, MAX_SVG_PATH_BYTES, sample_offsets,
//! };
//! use motion_render::{Composition, RenderPrepare, RenderOptions, Renderer, render_still};
//! use std::sync::Arc;
//! let asset = Arc::new(ImageAsset::from_rgba8(
//!     2,
//!     1,
//!     vec![255, 0, 0, 255, 0, 255, 0, 0],
//! )?);
//! let picture = Image::from_asset(asset)
//!     .size(8.0, 4.0)
//!     .fit(Fit::Cover)
//!     .corner_radius(1.0)
//!     .sampling(Sampling::Trilinear);
//! let frame =
//!     Composition::new(8, 4, motion_render::FrameClock::new(30, 150)).render_frame(0, Size::new(8.0, 4.0), |_| picture.clone().into())?;
//! assert_eq!(frame.size().width, 8);
//! # Ok::<(), motion_core::Error>(())
//! ```

#[cfg(feature = "image")]
use std::io::Cursor;
use std::{io, sync::Arc};

use half::f16;
#[cfg(feature = "image")]
use image::{DynamicImage, ImageDecoder, ImageFormat, ImageReader, Limits};

pub use motion_core::pixels::Sampling;

/// Centered image placement in a local viewport.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Fit {
    /// Stretch independently along each axis.
    Fill,
    /// Preserve aspect ratio with transparent letterboxing.
    #[default]
    Contain,
    /// Preserve aspect ratio and crop to fill the viewport.
    Cover,
    /// Keep source pixel dimensions and crop centrally.
    None,
}

#[derive(Clone, Debug)]
enum Input {
    Source(motion_core::Source),
    Lazy(motion_core::Handle<ImageAsset>),
    Loaded(Arc<ImageAsset>),
}

/// A static image drawable with explicit viewport/fit/corner/sampling.
/// Missing handles draw a magenta placeholder and report through the native
/// frame resource diagnostics.
#[derive(Clone, Debug)]
pub struct Image {
    input: Input,
    size: Option<motion_core::Size>,
    fit: Fit,
    radius: f64,
    sampling: Sampling,
}

impl Image {
    /// Captures a path resolved by the session resource owner during compilation.
    pub fn open(source: impl Into<motion_core::Source>) -> Self {
        Self {
            input: Input::Source(source.into()),
            size: None,
            fit: Fit::Contain,
            radius: 0.0,
            sampling: Sampling::Trilinear,
        }
    }

    /// Captures a lazy resource handle without reading or decoding it.
    pub fn new(handle: motion_core::Handle<ImageAsset>) -> Self {
        Self {
            input: Input::Lazy(handle),
            size: None,
            fit: Fit::Contain,
            radius: 0.0,
            sampling: Sampling::Trilinear,
        }
    }

    /// Captures already decoded image data without copying mip pixels.
    pub fn from_asset(image: Arc<ImageAsset>) -> Self {
        Self {
            input: Input::Loaded(image),
            size: None,
            fit: Fit::Contain,
            radius: 0.0,
            sampling: Sampling::Trilinear,
        }
    }

    /// Sets finite nonnegative local viewport dimensions, independent of source size.
    pub fn size(mut self, width: f64, height: f64) -> Self {
        self.size = Some(motion_core::Size::new(width, height));
        self
    }

    /// Selects centered Fill, Contain, Cover or original-size placement.
    pub fn fit(mut self, fit: Fit) -> Self {
        self.fit = fit;
        self
    }

    /// Clips to uniformly rounded viewport corners; radius clamps to viewport size.
    pub fn corner_radius(mut self, radius: f64) -> Self {
        self.radius = radius;
        self
    }

    /// Selects shared nearest, bilinear or transform-derived trilinear sampling.
    pub fn sampling(mut self, sampling: Sampling) -> Self {
        self.sampling = sampling;
        self
    }

    fn image(&self, context: &motion_core::DrawContext) -> motion_core::Result<Arc<ImageAsset>> {
        match &self.input {
            Input::Source(source) => context.assets().load::<ImageAsset>(source.clone()).get(),
            Input::Lazy(handle) => handle.get(),
            Input::Loaded(image) => Ok(image.clone()),
        }
    }

    fn layout_bounds(
        &self,
        context: &motion_core::DrawContext,
    ) -> Result<motion_core::Rect, motion_core::ValidationError> {
        if !self.radius.is_finite() || self.radius < 0.0 {
            return Err(motion_core::ValidationError::invalid(
                "image",
                "corner_radius",
                "Image corner radius must be finite and nonnegative",
            ));
        }
        let size = self.size.unwrap_or_else(|| {
            self.image(context)
                .map(|image| {
                    motion_core::Size::new(f64::from(image.width()), f64::from(image.height()))
                })
                .unwrap_or(motion_core::Size::new(64.0, 64.0))
        });
        Ok(size.to_rect())
    }
}

impl motion_core::Drawable for Image {
    fn bounds(
        &self,
        context: &motion_core::DrawContext,
    ) -> Result<motion_core::Rect, motion_core::ValidationError> {
        self.layout_bounds(context)
    }

    fn draw(
        &self,
        context: &motion_core::DrawContext,
        canvas: &mut motion_core::Canvas<'_>,
    ) -> Result<(), motion_core::ValidationError> {
        let bounds = self.layout_bounds(context)?;
        let shape = motion_core::RoundedRect::new(bounds, self.radius)?;
        match self.image(context) {
            Ok(image) => {
                if bounds.width() == 0.0 || bounds.height() == 0.0 {
                    return Ok(());
                }
                let horizontal = bounds.width() / f64::from(image.width());
                let vertical = bounds.height() / f64::from(image.height());
                let (horizontal, vertical) = match self.fit {
                    Fit::Fill => (horizontal, vertical),
                    Fit::Contain => (horizontal.min(vertical), horizontal.min(vertical)),
                    Fit::Cover => (horizontal.max(vertical), horizontal.max(vertical)),
                    Fit::None => (1.0, 1.0),
                };
                let width = f64::from(image.width()) * horizontal;
                let height = f64::from(image.height()) * vertical;
                let transform = motion_core::Transform::scale_non_uniform(horizontal, vertical)
                    .then(motion_core::Transform::translate(
                        (bounds.width() - width) * 0.5,
                        (bounds.height() - height) * 0.5,
                    ));
                let mut pattern = motion_core::ImagePattern::from_asset(image)
                    .transform(transform)
                    .extend(motion_core::GradientExtend::Pad)
                    .sampling(self.sampling);
                pattern.decal = true;
                canvas.fill(&shape, pattern, motion_core::FillRule::NonZero)
            }
            Err(_) => canvas.fill(
                &shape,
                motion_core::Color::rgb(255, 0, 255),
                motion_core::FillRule::NonZero,
            ),
        }
    }
}

/// One immutable image mip level in linear-premultiplied RGBA16F.
#[derive(Clone, Debug, PartialEq)]
pub struct ImageLevel {
    width: u32,
    height: u32,
    pixels: Arc<[[f16; 4]]>,
}

impl ImageLevel {
    /// Mip width in texels.
    pub fn width(&self) -> u32 {
        self.width
    }

    /// Mip height in texels.
    pub fn height(&self) -> u32 {
        self.height
    }

    /// Checked linear-premultiplied texel, expanded from shared half storage.
    pub fn pixel(&self, column: u32, row: u32) -> Option<[f32; 4]> {
        (column < self.width && row < self.height)
            .then(|| self.pixels[(row * self.width + column) as usize].map(f16::to_f32))
    }
}

/// static PNG/JPEG/WebP data with EXIF-corrected linear RGBA16F mipmaps.
/// Clones share every level. Encoded profiles are interpreted as sRGB; ICC/HDR
/// profile transforms and animated frame selection belong to later contracts.
#[derive(Clone, Debug, PartialEq)]
pub struct ImageAsset {
    levels: Arc<[ImageLevel]>,
}

impl motion_core::pixels::ImagePixels for ImageAsset {
    fn width(&self) -> u32 {
        self.width()
    }

    fn height(&self) -> u32 {
        self.height()
    }

    fn level_count(&self) -> usize {
        self.levels.len()
    }

    fn level_size(&self, level: usize) -> (u32, u32) {
        (self.levels[level].width(), self.levels[level].height())
    }

    fn texel(&self, level: usize, column: usize, row: usize) -> [f32; 4] {
        self.levels[level]
            .pixel(column as u32, row as u32)
            .expect("bounded image texel")
    }

    fn extend_level(&self, level: usize, out: &mut Vec<f32>) {
        let pixels = &self.levels[level].pixels;
        out.reserve(pixels.len() * 4);
        for pixel in pixels.iter() {
            out.extend(pixel.map(f16::to_f32));
        }
    }

    fn content_eq(&self, other: &dyn motion_core::pixels::ImagePixels) -> bool {
        other
            .as_any()
            .downcast_ref::<Self>()
            .is_some_and(|other| self == other)
    }

    fn as_any(&self) -> &dyn std::any::Any {
        self
    }
}

fn invalid(message: &str) -> motion_core::Error {
    io::Error::new(io::ErrorKind::InvalidInput, message).into()
}

impl ImageAsset {
    /// Reports unavailable decoding without performing I/O when codecs are disabled.
    #[cfg(not(feature = "image"))]
    pub fn from_bytes(_: &[u8]) -> motion_core::Result<Self> {
        Err(motion_core::Error::from(
            motion_core::ValidationError::unsupported(
                "image",
                "codec",
                "Image decoding requires the image feature",
            ),
        ))
    }

    /// Decodes a bounded static PNG/JPEG/WebP and applies EXIF orientation first.
    #[cfg(feature = "image")]
    pub fn from_bytes(bytes: &[u8]) -> motion_core::Result<Self> {
        if bytes.len() > super::MAX_ENCODED {
            return Err(invalid("Encoded image exceeds 64 MiB"));
        }
        let mut reader = ImageReader::new(Cursor::new(bytes)).with_guessed_format()?;
        if !matches!(
            reader.format(),
            Some(ImageFormat::Png | ImageFormat::Jpeg | ImageFormat::WebP)
        ) {
            return Err(invalid("images require static PNG, JPEG or WebP data"));
        }
        if reader.format() == Some(ImageFormat::Png)
            && image::codecs::png::PngDecoder::new(Cursor::new(bytes))
                .map_err(crate::image_error)?
                .is_apng()
                .map_err(crate::image_error)?
        {
            return Err(invalid("Animated PNG requires the animated-image contract"));
        }
        if reader.format() == Some(ImageFormat::WebP)
            && image::codecs::webp::WebPDecoder::new(Cursor::new(bytes))
                .map_err(crate::image_error)?
                .has_animation()
        {
            return Err(invalid(
                "Animated WebP requires the animated-image contract",
            ));
        }
        let mut limits = Limits::default();
        limits.max_image_width = Some(16384);
        limits.max_image_height = Some(16384);
        limits.max_alloc = Some(256 * 1024 * 1024);
        reader.limits(limits);
        let mut decoder = reader.into_decoder().map_err(crate::image_error)?;
        let (width, height) = decoder.dimensions();
        super::pixel_count(width, height)?;
        let orientation = decoder.orientation().map_err(crate::image_error)?;
        let mut decoded = DynamicImage::from_decoder(decoder).map_err(crate::image_error)?;
        decoded.apply_orientation(orientation);
        let rgba = decoded.into_rgba32f();
        let pixels = rgba
            .pixels()
            .map(|pixel| {
                let alpha = f64::from(pixel.0[3]);
                let rgb = [pixel.0[0], pixel.0[1], pixel.0[2]]
                    .map(|value| motion_core::color::srgb_to_linear(f64::from(value)));
                [rgb[0], rgb[1], rgb[2], alpha]
            })
            .collect();
        Self::from_rgba_float(rgba.width(), rgba.height(), pixels)
    }

    /// Converts straight encoded-sRGB RGBA8 without retaining or decoding it later.
    pub fn from_rgba8(width: u32, height: u32, pixels: Vec<u8>) -> motion_core::Result<Self> {
        if super::pixel_count(width, height)? * 4 != pixels.len() {
            return Err(invalid("RGBA8 byte count must equal width*height*4"));
        }
        let values = pixels
            .as_chunks::<4>()
            .0
            .iter()
            .map(|pixel| {
                [
                    motion_core::color::srgb_to_linear(f64::from(pixel[0]) / 255.0),
                    motion_core::color::srgb_to_linear(f64::from(pixel[1]) / 255.0),
                    motion_core::color::srgb_to_linear(f64::from(pixel[2]) / 255.0),
                    f64::from(pixel[3]) / 255.0,
                ]
            })
            .collect();
        Self::from_rgba_float(width, height, values)
    }

    /// Takes straight LINEAR f64 RGBA, then premultiplies and rounds to half.
    /// Extended RGB is allowed when premultiplied values fit RGBA16F; alpha must
    /// be in 0..=1. Zero or half-rounded-zero alpha clears hidden RGB.
    pub fn from_rgba_float(
        width: u32,
        height: u32,
        pixels: Vec<[f64; 4]>,
    ) -> motion_core::Result<Self> {
        if super::pixel_count(width, height)? != pixels.len() {
            return Err(invalid("RGBA float count must equal width*height"));
        }
        let mut base = Vec::with_capacity(pixels.len());
        for pixel in pixels {
            if !pixel[3].is_finite() || !(0.0..=1.0).contains(&pixel[3]) {
                return Err(invalid("Image alpha must be finite in 0..=1"));
            }
            if pixel[3] == 0.0 {
                base.push([f16::ZERO; 4]);
                continue;
            }
            let pixel = [
                pixel[0] * pixel[3],
                pixel[1] * pixel[3],
                pixel[2] * pixel[3],
                pixel[3],
            ];
            if pixel
                .iter()
                .any(|value| !value.is_finite() || value.abs() > 65504.0)
            {
                return Err(invalid(
                    "Premultiplied image values must fit finite RGBA16F",
                ));
            }
            let mut stored = pixel.map(f16::from_f64);
            if stored[3] == f16::ZERO {
                stored = [f16::ZERO; 4];
            }
            base.push(stored);
        }
        let mut levels = vec![ImageLevel {
            width,
            height,
            pixels: base.into(),
        }];
        let mut bytes = u64::from(width) * u64::from(height) * 8;
        while levels.last().unwrap().width > 1 || levels.last().unwrap().height > 1 {
            let previous = levels.last().unwrap();
            let width = previous.width.div_ceil(2);
            let height = previous.height.div_ceil(2);
            bytes = bytes
                .checked_add(u64::from(width) * u64::from(height) * 8)
                .filter(|bytes| *bytes <= 256 * 1024 * 1024)
                .ok_or_else(|| invalid("Image mip chain exceeds 256 MiB"))?;
            let mut pixels = Vec::with_capacity(width as usize * height as usize);
            for row in 0..height {
                for column in 0..width {
                    let left = f64::from(column) * f64::from(previous.width) / f64::from(width);
                    let right =
                        f64::from(column + 1) * f64::from(previous.width) / f64::from(width);
                    let top = f64::from(row) * f64::from(previous.height) / f64::from(height);
                    let bottom =
                        f64::from(row + 1) * f64::from(previous.height) / f64::from(height);
                    let mut sum = [0.0_f64; 4];
                    let mut count = 0.0;
                    for vertical in
                        libm::floor(top) as u32..(libm::ceil(bottom) as u32).min(previous.height)
                    {
                        for horizontal in
                            libm::floor(left) as u32..(libm::ceil(right) as u32).min(previous.width)
                        {
                            let pixel = previous.pixel(horizontal, vertical).unwrap();
                            let weight = (right.min(f64::from(horizontal + 1))
                                - left.max(f64::from(horizontal)))
                                * (bottom.min(f64::from(vertical + 1))
                                    - top.max(f64::from(vertical)));
                            for channel in 0..4 {
                                sum[channel] += f64::from(pixel[channel]) * weight;
                            }
                            count += weight;
                        }
                    }
                    let mut stored = sum.map(|value| f16::from_f64(value / count));
                    if stored[3] == f16::ZERO {
                        stored = [f16::ZERO; 4];
                    }
                    pixels.push(stored);
                }
            }
            levels.push(ImageLevel {
                width,
                height,
                pixels: pixels.into(),
            });
        }
        Ok(Self {
            levels: levels.into(),
        })
    }

    /// Source width after orientation.
    pub fn width(&self) -> u32 {
        self.levels[0].width
    }

    /// Source height after orientation.
    pub fn height(&self) -> u32 {
        self.levels[0].height
    }

    /// Immutable mip chain, including the base and final 1x1 level.
    pub fn levels(&self) -> &[ImageLevel] {
        &self.levels
    }

    /// Logical shared pixel payload, excluding allocation overhead.
    pub fn retained_bytes(&self) -> usize {
        self.levels.iter().map(|level| level.pixels.len() * 8).sum()
    }
}

impl motion_core::Asset for ImageAsset {
    const KIND: &'static str = "image";

    fn decode(bytes: &[u8], _: &motion_core::LoadContext) -> motion_core::Result<Self> {
        Self::from_bytes(bytes)
    }

    fn retained_bytes(&self) -> usize {
        self.retained_bytes()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[cfg(feature = "image")]
    use crate::asset::AssetsExt;
    use motion_render::RenderPrepare;

    #[test]
    #[cfg(feature = "gpu")]
    #[ignore = "requires native GPU; no font discovery, encoder or window"]
    fn images_match_gpu_for_all_fit_sampling_and_fractional_mips() -> motion_core::Result {
        use motion_core::{Node, NodeModifiers, PixelSize, RasterLimits, Size, ValidationLimits};
        let limits = RasterLimits::default();
        let mut gpu = motion_render::gpu::Gpu::new()?;
        let info = gpu.info().clone();
        let pixels = (0..9)
            .flat_map(|row| {
                (0..17).map(move |column| {
                    [
                        column as f64 / 16.0,
                        row as f64 / 8.0,
                        if column % 2 == 0 { 1.5 } else { 0.1 },
                        if row % 3 == 0 { 0.0 } else { 0.8 },
                    ]
                })
            })
            .collect();
        let asset = Arc::new(ImageAsset::from_rgba_float(17, 9, pixels)?);
        let mut maximum = 0.0_f32;
        let mut cases = 0;
        for size in [2.3, 5.5, 12.0] {
            for fit in [Fit::Fill, Fit::Contain, Fit::Cover, Fit::None] {
                for sampling in [Sampling::Nearest, Sampling::Bilinear, Sampling::Trilinear] {
                    let image = Image::from_asset(asset.clone())
                        .size(size, size * 0.8)
                        .fit(fit)
                        .corner_radius(size * 0.15)
                        .sampling(sampling);
                    let prepared = Node::from(image)
                        .rotate(0.13)
                        .translate(1.1, 2.1)
                        .compile(
                            Size::new(16.0, 16.0),
                            PixelSize::new(16, 16),
                            ValidationLimits::default(),
                        )?
                        .prepare(ValidationLimits::default(), limits)?;
                    let cpu = prepared.render_linear(limits)?;
                    let output = gpu.render_linear(
                        &prepared,
                        motion_render::gpu::Plan::new(&prepared, limits)?,
                    )?;
                    for row in 0..16 {
                        for column in 0..16 {
                            for (first, second) in cpu
                                .linear_pixel(column, row)
                                .unwrap()
                                .into_iter()
                                .zip(output.linear_pixel(column, row).unwrap())
                            {
                                maximum = maximum.max((first - second).abs());
                            }
                        }
                    }
                    let cpu = cpu.to_format(
                        motion_render::PixelFormat::SrgbRgba8 {
                            premultiplied: false,
                        },
                        limits,
                    )?;
                    let output = output.to_format(
                        motion_render::PixelFormat::SrgbRgba8 {
                            premultiplied: false,
                        },
                        limits,
                    )?;
                    for (first, second) in cpu.bytes().iter().zip(output.bytes()) {
                        assert!(first.abs_diff(*second) <= 1);
                    }
                    let replay = gpu
                        .render_linear(
                            &prepared,
                            motion_render::gpu::Plan::new(&prepared, limits)?,
                        )?
                        .to_format(
                            motion_render::PixelFormat::SrgbRgba8 {
                                premultiplied: false,
                            },
                            limits,
                        )?;
                    assert_eq!(output, replay);
                    cases += 1;
                }
            }
        }
        assert!(maximum <= 5e-6, "image linear difference {maximum}");
        eprintln!(
            "_image_gpu adapter={} backend={:?} cases={cases} source=RGBA16F odd_mips=shared max_linear_difference={maximum} rgba_tolerance=1 replay=exact",
            info.name, info.backend
        );
        Ok(())
    }

    #[test]
    #[cfg(feature = "image")]
    fn decoded_image_handles_survive_source_file_removal() -> motion_core::Result {
        use image::ImageEncoder;
        let directory = tempfile::tempdir()?;
        let path = directory.path().join("pixel.png");
        let mut encoded = Vec::new();
        image::codecs::png::PngEncoder::new(&mut encoded)
            .write_image(&[255, 0, 0, 255], 1, 1, image::ExtendedColorType::Rgba8)
            .map_err(crate::image_error)?;
        std::fs::write(&path, encoded)?;
        let assets = motion_core::Assets::new(directory.path())?;
        let handle = assets.image("pixel.png");
        let loaded = handle.get()?;
        std::fs::remove_file(path)?;
        assert!(Arc::ptr_eq(&loaded, &handle.get()?));
        assert_eq!(
            render(Image::new(handle).size(1.0, 1.0), 1)?.linear_pixel(0, 0),
            Some([1.0, 0.0, 0.0, 1.0])
        );
        Ok(())
    }

    fn render(image: Image, size: u32) -> motion_core::Result<motion_render::FrameBuffer> {
        Ok(motion_core::Node::from(image)
            .compile(
                motion_core::Size::new(f64::from(size), f64::from(size)),
                motion_core::PixelSize::new(size, size),
                motion_core::ValidationLimits::default(),
            )?
            .prepare(
                motion_core::ValidationLimits::default(),
                motion_core::RasterLimits::default(),
            )?
            .render_linear(motion_core::RasterLimits::default())?)
    }

    #[test]
    #[cfg(feature = "image")]
    fn static_decoders_preserve_png_precision_webp_and_exif_orientation() -> motion_core::Result {
        use image::ImageEncoder;
        let mut png = Cursor::new(Vec::new());
        let image = image::ImageBuffer::from_vec(1, 1, vec![32768_u16, 0, 0, 65535]).unwrap();
        DynamicImage::ImageRgba16(image)
            .write_to(&mut png, ImageFormat::Png)
            .map_err(crate::image_error)?;
        let decoded = ImageAsset::from_bytes(png.get_ref())?;
        let expected = motion_core::color::srgb_to_linear(32768.0 / 65535.0);
        assert!((f64::from(decoded.levels()[0].pixel(0, 0).unwrap()[0]) - expected).abs() < 0.0002);
        let exif = b"II\x2a\0\x08\0\0\0\x01\0\x12\x01\x03\0\x01\0\0\0\x06\0\0\0\0\0\0\0".to_vec();
        let mut webp = Vec::new();
        let mut encoder = image::codecs::webp::WebPEncoder::new_lossless(&mut webp);
        encoder
            .set_exif_metadata(exif.clone())
            .map_err(io::Error::other)?;
        encoder
            .encode(
                &[255, 0, 0, 255, 0, 255, 0, 255],
                2,
                1,
                image::ExtendedColorType::Rgba8,
            )
            .map_err(crate::image_error)?;
        let decoded = ImageAsset::from_bytes(&webp)?;
        assert_eq!((decoded.width(), decoded.height()), (1, 2));
        assert_eq!(decoded.levels()[0].pixel(0, 0), Some([1.0, 0.0, 0.0, 1.0]));
        assert_eq!(decoded.levels()[0].pixel(0, 1), Some([0.0, 1.0, 0.0, 1.0]));
        let mut jpeg = Vec::new();
        let mut encoder = image::codecs::jpeg::JpegEncoder::new(&mut jpeg);
        encoder.set_exif_metadata(exif).map_err(io::Error::other)?;
        encoder
            .encode(
                &[
                    255, 0, 0, 0, 255, 0, 0, 0, 255, 255, 0, 0, 0, 255, 0, 0, 0, 255,
                ],
                3,
                2,
                image::ExtendedColorType::Rgb8,
            )
            .map_err(crate::image_error)?;
        let decoded = ImageAsset::from_bytes(&jpeg)?;
        assert_eq!((decoded.width(), decoded.height()), (2, 3));
        Ok(())
    }

    #[test]
    fn image_fit_rounding_and_mip_minification_have_independent_pixels() -> motion_core::Result {
        let asset = Arc::new(ImageAsset::from_rgba8(
            3,
            1,
            vec![255, 0, 0, 255, 0, 255, 0, 255, 0, 0, 255, 255],
        )?);
        let image = Image::from_asset(asset)
            .size(8.0, 8.0)
            .sampling(Sampling::Nearest);
        let contain = render(image.clone().fit(Fit::Contain), 8)?;
        assert_eq!(contain.linear_pixel(4, 0), Some([0.0; 4]));
        assert_eq!(contain.linear_pixel(4, 3), Some([0.0, 1.0, 0.0, 1.0]));
        let cover = render(image.clone().fit(Fit::Cover), 8)?;
        assert_eq!(cover.linear_pixel(0, 0), Some([0.0, 1.0, 0.0, 1.0]));
        let fill = render(image.clone().fit(Fit::Fill), 8)?;
        assert_eq!(fill.linear_pixel(0, 0), Some([1.0, 0.0, 0.0, 1.0]));
        let original = render(image.clone().fit(Fit::None), 8)?;
        assert_eq!(original.linear_pixel(0, 0), Some([0.0; 4]));
        assert_eq!(original.linear_pixel(3, 3), Some([0.0, 1.0, 0.0, 1.0]));
        let rounded = render(image.fit(Fit::Fill).corner_radius(4.0), 8)?;
        assert_eq!(rounded.linear_pixel(0, 0), Some([0.0; 4]));
        assert!(rounded.linear_pixel(4, 4).unwrap()[3] > 0.9);
        let mut pixels = vec![[0.0, 0.0, 0.0, 1.0]; 64];
        pixels[0] = [1.0; 4];
        let asset = Arc::new(ImageAsset::from_rgba_float(8, 8, pixels)?);
        let nearest = render(
            Image::from_asset(asset.clone())
                .size(1.0, 1.0)
                .sampling(Sampling::Nearest),
            1,
        )?;
        let trilinear = render(
            Image::from_asset(asset)
                .size(1.0, 1.0)
                .sampling(Sampling::Trilinear),
            1,
        )?;
        assert_eq!(nearest.linear_pixel(0, 0).unwrap()[0], 0.0);
        assert_eq!(trilinear.linear_pixel(0, 0).unwrap()[0], 1.0 / 64.0);
        Ok(())
    }

    #[test]
    fn half_images_build_linear_premultiplied_odd_mips_without_hidden_color() -> motion_core::Result
    {
        let image = ImageAsset::from_rgba8(2, 1, vec![255, 0, 0, 255, 0, 255, 0, 0])?;
        assert_eq!(image.levels()[1].pixel(0, 0), Some([0.5, 0.0, 0.0, 0.5]));
        assert_eq!(image.retained_bytes(), 24);
        let odd = ImageAsset::from_rgba8(3, 1, vec![255; 12])?;
        assert_eq!(
            odd.levels()
                .iter()
                .map(|level| (level.width(), level.height()))
                .collect::<Vec<_>>(),
            [(3, 1), (2, 1), (1, 1)]
        );
        assert_eq!(odd.levels().last().unwrap().pixel(0, 0), Some([1.0; 4]));
        let weighted =
            ImageAsset::from_rgba8(3, 1, vec![255, 0, 0, 255, 255, 0, 0, 255, 0, 0, 255, 255])?;
        let final_pixel = weighted.levels().last().unwrap().pixel(0, 0).unwrap();
        assert!((final_pixel[0] - 2.0 / 3.0).abs() < 0.0005);
        assert!((final_pixel[2] - 1.0 / 3.0).abs() < 0.0005);
        let transparent =
            ImageAsset::from_rgba_float(1, 1, vec![[f64::NAN, f64::INFINITY, 1.0, 0.0]])?;
        assert_eq!(transparent.levels()[0].pixel(0, 0), Some([0.0; 4]));
        assert!(ImageAsset::from_rgba_float(1, 1, vec![[1e9, 0.0, 0.0, 1.0]]).is_err());
        Ok(())
    }
}
