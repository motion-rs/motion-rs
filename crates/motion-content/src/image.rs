//! Immutable static and animated image resources.
#[cfg(all(test, feature = "image"))]
use image::{DynamicImage, ImageFormat};
use std::io;
#[cfg(all(test, feature = "image"))]
use std::{io::Cursor, sync::Arc};

#[cfg(feature = "image")]
mod animated;
mod static_image;
#[cfg(all(test, feature = "svg"))]
mod svg;

#[cfg(feature = "image")]
pub use animated::{AnimatedImage, AnimatedImageAsset, ImageLoopMode};
pub use static_image::{Fit, Image, ImageAsset, ImageLevel, Sampling};

#[cfg(feature = "image")]
const MAX_ENCODED: usize = 64 * 1024 * 1024;

const MAX_PIXELS: usize = 16_777_216;

fn pixel_count(width: u32, height: u32) -> io::Result<usize> {
    if width == 0 || height == 0 || width > 16384 || height > 16384 {
        return Err(invalid("Image dimensions must be in 1..=16384"));
    }
    let count = u64::from(width) * u64::from(height);
    if count > MAX_PIXELS as u64 {
        return Err(invalid("Image exceeds 16777216 source pixels"));
    }
    Ok(count as usize)
}

fn invalid(message: &str) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidInput, message)
}

#[cfg(all(test, feature = "image"))]
mod tests {
    use super::*;
    use crate::asset::AssetsExt;
    use motion_core::NodeModifiers;
    use motion_core::{FrameRate, Node, Size};
    #[cfg(feature = "svg")]
    use motion_render::RenderPrepare;
    use motion_render::{Composition, Renderer};

    #[test]
    #[cfg(feature = "svg")]
    fn svg_bytes_decode_once_preserve_alpha_and_share_pixels() -> motion_core::Result {
        let mut bytes = br##"<svg xmlns="http://www.w3.org/2000/svg" width="4" height="2">
            <path d="M0 0H2V2H0Z" fill="#ff0000" opacity="0.5"/>
        </svg>"##
            .to_vec();
        let asset = crate::SvgAsset::from_bytes(&bytes)?;
        bytes.fill(0);
        assert_eq!(asset.size(), motion_core::Size::new(4.0, 2.0));
        let frame = asset
            .tree
            .node()
            .compile(
                asset.size(),
                motion_core::PixelSize::new(4, 2),
                motion_core::ValidationLimits::default(),
            )?
            .prepare(
                motion_core::ValidationLimits::default(),
                motion_core::RasterLimits::default(),
            )?
            .render_linear(motion_core::RasterLimits::default())?;
        assert_eq!(frame.pixel(0, 0), Some([0.5, 0.0, 0.0, 0.5]));
        assert_eq!(frame.pixel(2, 0), Some([0.0; 4]));
        assert!(Arc::ptr_eq(&asset.tree, &asset.clone().tree));
        Ok(())
    }

    #[test]
    fn png_roundtrip_file_independence_and_shared_pixels() -> motion_core::Result {
        let rgba = vec![255, 0, 0, 255, 0, 255, 0, 0];
        let mut encoded = Cursor::new(Vec::new());
        DynamicImage::ImageRgba8(image::RgbaImage::from_raw(2, 1, rgba.clone()).unwrap())
            .write_to(&mut encoded, ImageFormat::Png)
            .map_err(crate::image_error)?;
        let asset = ImageAsset::from_bytes(encoded.get_ref())?;
        assert_eq!((asset.width(), asset.height()), (2, 1));
        assert_eq!(asset.levels()[0].pixel(0, 0), Some([1.0, 0.0, 0.0, 1.0]));
        assert_eq!(asset.levels()[0].pixel(1, 0), Some([0.0; 4]));
        let cloned = asset.clone();
        assert!(std::ptr::eq(asset.levels(), cloned.levels()));
        let returned = std::thread::spawn(move || cloned).join().unwrap();
        assert!(std::ptr::eq(asset.levels(), returned.levels()));
        let file = tempfile::NamedTempFile::new()?;
        std::fs::write(file.path(), encoded.into_inner())?;
        let loaded = ImageAsset::from_bytes(&std::fs::read(file.path())?)?;
        drop(file);
        assert_eq!(loaded, asset);
        Ok(())
    }

    #[test]
    fn jpeg_decodes_to_opaque_rgba() -> motion_core::Result {
        let mut encoded = Cursor::new(Vec::new());
        DynamicImage::ImageRgb8(image::RgbImage::from_pixel(2, 2, image::Rgb([120, 80, 40])))
            .write_to(&mut encoded, ImageFormat::Jpeg)
            .map_err(crate::image_error)?;
        let asset = ImageAsset::from_bytes(encoded.get_ref())?;
        assert_eq!((asset.width(), asset.height()), (2, 2));
        for row in 0..2 {
            for column in 0..2 {
                let pixel = asset.levels()[0].pixel(column, row).unwrap();
                assert_eq!(pixel[3], 1.0);
                assert!((motion_core::linear_to_srgb(pixel[0]) * 255.0 - 120.0).abs() <= 5.0);
            }
        }
        Ok(())
    }

    #[test]
    fn invalid_formats_dimensions_and_buffers_are_rejected() -> motion_core::Result {
        use image::ImageEncoder;
        for (width, height) in [
            (0, 1),
            (1, 0),
            (16385, 1),
            (4097, 4097),
            (u32::MAX, u32::MAX),
        ] {
            assert!(ImageAsset::from_rgba8(width, height, Vec::new()).is_err());
        }
        assert!(ImageAsset::from_rgba8(1, 1, vec![0; 3]).is_err());
        assert!(ImageAsset::from_bytes(b"not an image").is_err());
        let mut encoded = Vec::new();
        image::codecs::png::PngEncoder::new(&mut encoded)
            .write_image(
                &vec![0; 16385 * 4],
                16385,
                1,
                image::ExtendedColorType::Rgba8,
            )
            .map_err(crate::image_error)?;
        assert!(ImageAsset::from_bytes(&encoded).is_err());
        encoded.truncate(20);
        assert!(ImageAsset::from_bytes(&encoded).is_err());
        Ok(())
    }

    #[test]
    fn sequence_sampling_shares_pixels_and_supports_remapped_random_access() -> motion_core::Result
    {
        let images = (0..4)
            .map(|index| ImageAsset::from_rgba8(1, 1, vec![index * 60, 0, 0, 255]).map(Arc::new))
            .collect::<motion_core::Result<Vec<_>>>()?;
        let rate = motion_core::FrameRate::NTSC;
        let sequence = AnimatedImageAsset::from_sequence(images.clone(), rate)?;
        let clone = sequence.clone();
        assert!(std::ptr::eq(sequence.frames(), clone.frames()));
        assert_eq!(sequence.size(), motion_core::PixelSize::new(1, 1));
        assert_eq!(sequence.frames().len(), 4);
        for index in [3, 0, 2, 1, 19, 3] {
            let selected = sequence
                .frame_at(
                    motion_core::Duration::from_frame(index, rate),
                    ImageLoopMode::Once,
                )
                .unwrap();
            assert!(Arc::ptr_eq(selected, &images[index.min(3) as usize]));
        }
        let reversed =
            AnimatedImageAsset::from_sequence(images[1..4].iter().rev().cloned().collect(), rate)?;
        for (index, expected) in [(0, 3), (1, 3), (2, 2), (4, 1), (19, 1), (0, 3)] {
            let selected = reversed
                .frame_at(
                    motion_core::Duration::from_frame(index, rate) / 2.0,
                    ImageLoopMode::Once,
                )
                .unwrap();
            assert!(Arc::ptr_eq(selected, &images[expected]));
        }
        let freeze = AnimatedImageAsset::from_sequence(vec![images[2].clone()], rate)?;
        assert!(Arc::ptr_eq(
            freeze
                .frame_at(
                    motion_core::Duration::from_frame(19, rate),
                    ImageLoopMode::Loop
                )
                .unwrap(),
            &images[2]
        ));
        let joined = std::thread::spawn(move || clone).join().unwrap();
        assert!(std::ptr::eq(sequence.frames(), joined.frames()));
        Ok(())
    }

    #[test]
    fn sequence_open_preserves_explicit_order_and_survives_file_removal() -> motion_core::Result {
        let directory = tempfile::tempdir()?;
        let png_path = directory.path().join("99.dat");
        let jpeg_path = directory.path().join("01.dat");
        let mut encoded = Cursor::new(Vec::new());
        DynamicImage::ImageRgba8(image::RgbaImage::from_pixel(
            2,
            2,
            image::Rgba([255, 0, 0, 128]),
        ))
        .write_to(&mut encoded, ImageFormat::Png)
        .map_err(crate::image_error)?;
        std::fs::write(&png_path, encoded.into_inner())?;
        let mut encoded = Cursor::new(Vec::new());
        DynamicImage::ImageRgb8(image::RgbImage::from_pixel(2, 2, image::Rgb([0, 180, 0])))
            .write_to(&mut encoded, ImageFormat::Jpeg)
            .map_err(crate::image_error)?;
        std::fs::write(&jpeg_path, encoded.into_inner())?;
        let assets = motion_core::Assets::new(directory.path())?;
        let sequence = AnimatedImageAsset::from_sequence(
            vec![assets.image("99.dat").get()?, assets.image("01.dat").get()?],
            motion_core::FrameRate::from(30),
        )?;
        assert!(assets.image("missing").get().is_err());
        directory.close()?;
        assert!(
            (sequence.frames()[0].levels()[0].pixel(0, 0).unwrap()[3] - 128.0 / 255.0).abs()
                < 0.00025
        );
        let jpeg = sequence.frames()[1].levels()[0].pixel(0, 0).unwrap();
        assert_eq!(jpeg[3], 1.0);
        assert!((motion_core::linear_to_srgb(jpeg[1]) * 255.0 - 180.0).abs() <= 5.0);
        Ok(())
    }

    #[test]
    fn sequences_preserve_original_trim_rate_and_boundary_tables() -> motion_core::Result {
        use motion_core::{
            Animation, Easing, FrameRate, Keyframes, SceneCache, SceneContext, Size, Time,
            TimeLiteral,
        };
        let rate = FrameRate::from(24);
        let images = (0..120)
            .map(|index| ImageAsset::from_rgba8(1, 1, vec![index, 0, 0, 255]).map(Arc::new))
            .collect::<motion_core::Result<Vec<_>>>()?;
        let sequence = |indices: Vec<usize>| {
            AnimatedImageAsset::from_sequence(
                indices
                    .into_iter()
                    .map(|index| images[index].clone())
                    .collect(),
                rate,
            )
        };
        for (indices, speed, expected) in [
            ((20..40).collect(), 1.0, [20, 21, 22, 23, 24, 25, 26]),
            ((20..40).collect(), 0.5, [20, 20, 21, 21, 22, 22, 23]),
            ((20..40).collect(), 2.0, [20, 22, 24, 26, 28, 30, 32]),
            ((20..40).collect(), 1.5, [20, 21, 23, 24, 26, 27, 29]),
            ((20..40).rev().collect(), 1.0, [39, 38, 37, 36, 35, 34, 33]),
            (vec![27], 0.0, [27; 7]),
        ] {
            let asset = sequence(indices)?;
            for index in [6, 0, 5, 1, 4, 2, 3, 0] {
                assert!(Arc::ptr_eq(
                    asset
                        .frame_at(
                            motion_core::Duration::from_frame(index, rate) * speed,
                            ImageLoopMode::Once
                        )
                        .unwrap(),
                    &images[expected[index as usize]]
                ));
            }
        }
        for (indices, mode, speed, expected) in [
            (
                (10..14).collect(),
                ImageLoopMode::Once,
                1.0,
                [10, 11, 12, 13, 13, 13, 13, 13, 13],
            ),
            (
                (10..14).collect(),
                ImageLoopMode::Loop,
                1.0,
                [10, 11, 12, 13, 10, 11, 12, 13, 10],
            ),
            (
                (10..14).rev().collect(),
                ImageLoopMode::Loop,
                1.0,
                [13, 12, 11, 10, 13, 12, 11, 10, 13],
            ),
            (
                (10..14).collect(),
                ImageLoopMode::PingPong,
                1.0,
                [10, 11, 12, 13, 12, 11, 10, 11, 12],
            ),
            (
                (10..14).rev().collect(),
                ImageLoopMode::PingPong,
                1.0,
                [13, 12, 11, 10, 11, 12, 13, 12, 11],
            ),
            (
                (10..14).collect(),
                ImageLoopMode::PingPong,
                0.5,
                [10, 10, 11, 11, 12, 12, 13, 13, 12],
            ),
            (
                (10..14).collect(),
                ImageLoopMode::PingPong,
                1.5,
                [10, 11, 13, 12, 10, 11, 13, 12, 10],
            ),
            (
                (10..14).collect(),
                ImageLoopMode::Loop,
                2.5,
                [10, 12, 11, 13, 12, 10, 13, 11, 10],
            ),
        ] {
            let asset = sequence(indices)?;
            for index in [8, 0, 7, 1, 6, 2, 5, 3, 4, 0] {
                assert!(Arc::ptr_eq(
                    asset
                        .frame_at(motion_core::Duration::from_frame(index, rate) * speed, mode)
                        .unwrap(),
                    &images[expected[index as usize]]
                ));
            }
        }
        let track = Keyframes::new(0.0).to(2.seconds(), 48.0, Easing::Linear);
        let cache = SceneCache::new(0);
        let source = SceneContext::new(
            Time::from_frame(24, rate),
            rate,
            Size::new(123.0, 456.0),
            &cache,
        )
        .with_clip_duration(motion_core::Duration::from_frame(120, rate));
        assert_eq!(source.sample(&track), 24.0);
        assert_eq!(source.time().as_seconds(), 1.0);
        assert_eq!(
            source
                .shifted(motion_core::Duration::from_frame(20, rate))
                .time()
                .to_frame(rate),
            4
        );
        assert_eq!(source.size(), Size::new(123.0, 456.0));
        assert_eq!(source.frame_rate(), rate);
        assert_eq!(source.clip_duration(), Some(5.seconds()));
        assert_eq!(track.sample(source.time().since_origin()), 24.0);
        Ok(())
    }

    #[test]
    fn sequence_rejects_empty_mixed_and_excessive_assets_without_large_allocations()
    -> motion_core::Result {
        let rate = motion_core::FrameRate::from(30);
        assert!(AnimatedImageAsset::from_sequence(Vec::new(), rate).is_err());
        let small = Arc::new(ImageAsset::from_rgba8(1, 1, vec![0; 4])?);
        let wide = Arc::new(ImageAsset::from_rgba8(2, 1, vec![0; 8])?);
        assert!(AnimatedImageAsset::from_sequence(vec![small.clone(), wide], rate).is_err());
        assert!(AnimatedImageAsset::from_sequence(vec![small.clone(); 4096], rate).is_ok());
        assert!(AnimatedImageAsset::from_sequence(vec![small.clone(); 4097], rate).is_err());
        assert!(
            AnimatedImageAsset::from_frames(vec![(small, motion_core::Duration::from_ticks(-1))])
                .is_err()
        );
        Ok(())
    }

    #[test]
    #[ignore = "requires native GPU when enabled; no fonts or window"]
    fn sequence_rendering_matches_manual_images_after_seeking() -> motion_core::Result {
        let images = (0..4)
            .map(|index| {
                ImageAsset::from_rgba8(1, 1, vec![index * 60, 30, 180, 128])
                    .map(std::sync::Arc::new)
            })
            .collect::<motion_core::Result<Vec<_>>>()?;
        let sequence = std::sync::Arc::new(AnimatedImageAsset::from_sequence(
            images[1..4].iter().rev().cloned().collect(),
            FrameRate::from(30),
        )?);
        for backend in [
            Renderer::Cpu,
            #[cfg(feature = "gpu")]
            Renderer::Gpu,
        ] {
            let composition = Composition::new(8, 8, motion_render::FrameClock::new(30, 20))
                .with_options(motion_render::RenderOptions::default().backend(backend.into()))?;
            let mut actual = composition.frame_renderer(Size::new(8.0, 8.0))?;
            let mut manual = composition.frame_renderer(Size::new(8.0, 8.0))?;
            for (mode, expected) in [
                (ImageLoopMode::Once, [3, 2, 1, 3, 1, 1, 1, 1, 3]),
                (ImageLoopMode::Loop, [3, 2, 3, 3, 1, 3, 2, 1, 3]),
                (ImageLoopMode::PingPong, [3, 2, 2, 3, 1, 2, 3, 2, 3]),
            ] {
                for (position, index) in [0, 2, 19, 1, 4, 6, 8, 10, 0].into_iter().enumerate() {
                    let sampled = actual.render_frame(index, |context| {
                        Node::from(
                            AnimatedImage::from_asset(sequence.clone())
                                .animation_time(context.elapsed() / 2.0)
                                .loop_mode(mode)
                                .size(4.0, 4.0),
                        )
                        .translate(2.0, 2.0)
                    })?;
                    let reference = manual.render_frame(index, |_| {
                        Node::from(
                            Image::from_asset(images[expected[position]].clone()).size(4.0, 4.0),
                        )
                        .translate(2.0, 2.0)
                    })?;
                    assert!(
                        sampled.pixels() == reference.pixels(),
                        "sequence mismatch at {index} in {mode:?}"
                    );
                    assert_ne!(sampled.pixels()[3 * 8 + 3], 0x00ff_ffff);
                    assert_eq!(sampled.pixels()[0], 0x00ff_ffff);
                }
            }
        }
        Ok(())
    }
}
