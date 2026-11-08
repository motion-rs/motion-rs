//! Tests for rasterizing SVG documents into image bitmaps.

use super::pixel_count;

#[cfg(test)]
mod tests {
    use crate::image::MAX_ENCODED;

    fn raster(bytes: &[u8]) -> motion_core::Result<motion_render::Bitmap> {
        let asset = crate::SvgAsset::from_bytes(bytes)?;
        raster_asset(
            asset.clone(),
            asset.size().width.ceil() as u32,
            asset.size().height.ceil() as u32,
        )
    }

    fn raster_to(
        bytes: &[u8],
        width: u32,
        height: u32,
    ) -> motion_core::Result<motion_render::Bitmap> {
        raster_asset(crate::SvgAsset::from_bytes(bytes)?, width, height)
    }

    fn raster_asset(
        asset: crate::SvgAsset,
        width: u32,
        height: u32,
    ) -> motion_core::Result<motion_render::Bitmap> {
        super::pixel_count(width, height)?;
        let frame = motion_render::Composition::new(width as usize, height as usize)
            .blend_space(motion_core::BlendSpace::Srgb)
            .frame_renderer(asset.size())?
            .render_output_at(
                motion_core::Time::ZERO,
                None,
                motion_render::PixelFormat::SrgbRgba8 {
                    premultiplied: false,
                },
                |_| asset.tree.node(),
            )?;
        Ok(motion_render::Bitmap::from_frame(frame)?)
    }

    const MARK: &[u8] = br#"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 8 4">
        <rect width="8" height="4" fill="blue"/>
    </svg>"#;

    #[test]
    fn intrinsic_viewbox_bom_and_file_content_are_supported() -> motion_core::Result {
        let mut bytes =
            b"\xef\xbb\xbf<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n<!-- icon -->".to_vec();
        bytes.extend_from_slice(MARK);
        let asset = raster(&bytes)?;
        assert_eq!(asset.size(), motion_core::PixelSize::new(8, 4));
        assert_eq!(&asset.bytes()[..4], &[0, 0, 255, 255]);
        let file = tempfile::Builder::new().suffix(".bin").tempfile()?;
        std::fs::write(file.path(), bytes)?;
        let loaded = raster(&std::fs::read(file.path())?)?;
        drop(file);
        assert_eq!(loaded.bytes(), asset.bytes());
        Ok(())
    }

    #[test]
    fn explicit_viewport_preserves_aspect_and_checks_limits() -> motion_core::Result {
        let asset = raster_to(MARK, 8, 8)?;
        assert_eq!(asset.size(), motion_core::PixelSize::new(8, 8));
        assert!(asset.bytes()[..8 * 2 * 4].iter().all(|byte| *byte == 0));
        assert!(asset.bytes()[8 * 6 * 4..].iter().all(|byte| *byte == 0));
        assert_eq!(&asset.bytes()[8 * 2 * 4..8 * 2 * 4 + 4], &[0, 0, 255, 255]);
        for (width, height) in [(0, 1), (1, 0), (16385, 1), (4097, 4097), (u32::MAX, 1)] {
            assert!(raster_to(MARK, width, height).is_err());
        }
        for source in [
            "<svg xmlns='http://www.w3.org/2000/svg' width='0' height='1'/>",
            "<svg xmlns='http://www.w3.org/2000/svg' width='16385' height='1'/>",
            "<svg xmlns='http://www.w3.org/2000/svg' width='4097' height='4097'/>",
        ] {
            assert!(raster(source.as_bytes()).is_err());
        }
        assert!(raster_to(&vec![b' '; MAX_ENCODED + 1], 1, 1).is_err());
        Ok(())
    }

    #[test]
    fn gradients_masks_internal_references_and_filters_render() -> motion_core::Result {
        let asset = raster(br##"<svg xmlns="http://www.w3.org/2000/svg" width="8" height="4">
            <defs>
                <linearGradient id="shade"><stop stop-color="red"/><stop offset="1" stop-color="blue"/></linearGradient>
                <mask id="mask"><rect width="4" height="4" fill="white"/></mask>
                <rect id="art" width="8" height="4" fill="url(#shade)"/>
            </defs>
            <g mask="url(#mask)"><use href="#art"/></g>
        </svg>"##)?;
        assert!(asset.bytes()[0] > asset.bytes()[2]);
        assert!(asset.bytes()[3 * 4 + 2] > asset.bytes()[2]);
        assert_eq!(&asset.bytes()[4 * 4..8 * 4], &[0; 16]);
        let blurred = raster(br##"<svg xmlns="http://www.w3.org/2000/svg" width="12" height="12">
            <defs><filter id="soft" x="-100%" y="-100%" width="300%" height="300%"><feGaussianBlur stdDeviation="1"/></filter></defs>
            <rect x="4" y="4" width="4" height="4" fill="red" filter="url(#soft)"/>
        </svg>"##)?;
        assert!(blurred.bytes()[(3 * 12 + 5) * 4 + 3] > 0);
        assert!(blurred.bytes()[(5 * 12 + 5) * 4 + 3] > blurred.bytes()[(3 * 12 + 5) * 4 + 3]);
        Ok(())
    }

    #[test]
    fn embedded_png_is_supported_and_bad_data_is_rejected() -> motion_core::Result {
        let mut encoded = std::io::Cursor::new(Vec::new());
        image::DynamicImage::ImageRgba8(image::RgbaImage::from_pixel(
            1,
            1,
            image::Rgba([255, 0, 0, 128]),
        ))
        .write_to(&mut encoded, image::ImageFormat::Png)
        .map_err(crate::image_error)?;
        let escaped: String = encoded
            .get_ref()
            .iter()
            .map(|byte| format!("%{byte:02X}"))
            .collect();
        let source = format!(
            "<svg xmlns='http://www.w3.org/2000/svg' width='1' height='1'><image width='1' height='1' href='data:image/png,{escaped}'/></svg>"
        );
        let asset = raster(source.as_bytes())?;
        assert_eq!(asset.bytes(), [255, 0, 0, 128]);
        for href in [
            "data:image/png,invalid",
            "data:image/svg+xml,%3Csvg",
            "data:image/gif,GIF89a",
        ] {
            let source = format!(
                "<svg xmlns='http://www.w3.org/2000/svg' width='1' height='1'><image width='1' height='1' href='{href}'/></svg>"
            );
            assert!(raster(source.as_bytes()).is_err(), "{href}");
        }
        Ok(())
    }

    #[test]
    fn external_references_dtd_scripts_animation_and_invalid_xml_are_rejected() {
        for body in [
            "<image width='1' height='1' href='local.png'/>",
            "<image width='1' height='1' href='https://example.com/image.png'/>",
            "<use href='file:///tmp/external.svg#shape'/>",
            "<script>alert(1)</script>",
            "<animate attributeName='opacity' from='0' to='1' dur='1s'/>",
            "<foreignObject width='1' height='1'/>",
        ] {
            let source = format!(
                "<svg xmlns='http://www.w3.org/2000/svg' width='1' height='1'>{body}</svg>"
            );
            assert!(raster(source.as_bytes()).is_err(), "{body}");
        }
        assert!(raster(b"<!DOCTYPE svg [<!ENTITY size '1'>]><svg xmlns='http://www.w3.org/2000/svg' width='&size;' height='1'/>").is_err());
        for bytes in [
            b"<svg".as_slice(),
            b"<html/>",
            b"<svg>\xff</svg>",
            &[0x1f, 0x8b, 0, 0],
        ] {
            assert!(raster(bytes).is_err());
        }
    }

    #[test]
    fn excessive_xml_nesting_and_node_counts_are_rejected() {
        let nested = format!(
            "<svg xmlns='http://www.w3.org/2000/svg' width='1' height='1'>{}{}</svg>",
            "<g>".repeat(130),
            "</g>".repeat(130)
        );
        assert!(raster(nested.as_bytes()).is_err());
        let nodes = format!(
            "<svg xmlns='http://www.w3.org/2000/svg' width='1' height='1'>{}</svg>",
            "<g/>".repeat(100_001)
        );
        assert!(raster(nodes.as_bytes()).is_err());
    }

    #[test]
    #[ignore = "requires explicit MOTION_TEST_FONT; no system discovery"]
    fn svg_text_uses_explicit_frozen_fonts_at_load_time() -> motion_core::Result {
        let mut fonts = crate::FontLibrary::new();
        fonts.load_file(std::env::var("MOTION_TEST_FONT")?)?;
        let asset = crate::SvgAsset::from_bytes_with_fonts(
            br#"<svg xmlns="http://www.w3.org/2000/svg" width="64" height="32">
            <text x="2" y="24" font-family="Arial" font-size="24">SVG</text>
        </svg>"#,
            &fonts,
        )?;
        assert!(
            raster_asset(asset, 64, 32)?
                .bytes()
                .as_chunks::<4>()
                .0
                .iter()
                .any(|pixel| pixel[3] > 0)
        );
        Ok(())
    }

    #[cfg(feature = "gpu")]
    #[test]
    #[ignore = "requires a native GPU"]
    fn svg_image_sampling_matches_software_and_replays() -> motion_core::Result {
        use crate::{Fit, Image, ImageAsset};
        use motion_core::{
            BlendSpace, Color, Node, NodeModifiers, Pivot, Point, SceneContext, Size, Time,
        };
        use motion_render::{Composition, PixelFormat, Renderer};
        let vector = crate::SvgAsset::from_bytes(br##"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 24 12">
            <defs><linearGradient id="fill"><stop stop-color="red"/><stop offset="1" stop-color="teal"/></linearGradient></defs>
            <rect x="1" y="1" width="22" height="10" rx="3" fill="url(#fill)" opacity="0.7"/>
        </svg>"##)?;
        let raster = Composition::new(48, 24)
            .blend_space(BlendSpace::Srgb)
            .frame_renderer(vector.size())?
            .render_output_at(
                Time::ZERO,
                None,
                PixelFormat::SrgbRgba8 {
                    premultiplied: false,
                },
                |_| vector.tree.node(),
            )?;
        let asset = std::sync::Arc::new(ImageAsset::from_rgba8(
            48,
            24,
            raster.planes()[0].bytes().to_vec(),
        )?);
        for fit in [Fit::Contain, Fit::Cover, Fit::Fill] {
            let mut cpu = Composition::new(48, 36)
                .background(Color::from_hex(0x23272c))
                .frame_renderer(Size::new(48.0, 36.0))?;
            let mut gpu = Composition::new(48, 36)
                .background(Color::from_hex(0x23272c))
                .renderer(Renderer::Gpu)
                .frame_renderer(Size::new(48.0, 36.0))?;
            let build = |context: &SceneContext<'_>| {
                Node::group([Node::from(
                    Image::from_asset(asset.clone()).size(32.0, 26.0).fit(fit),
                )
                .translate(2.0, 2.0)])
                .pivot(Pivot::Point(Point::ORIGIN))
                .rotate(7.0)
                .opacity(0.6)
                .translate(
                    (context.time().to_frame(context.frame_rate()) % 3) as f64,
                    2.0,
                )
            };
            let first = gpu.render_frame(17, build)?;
            for index in [0, 45, 17] {
                let expected = cpu.render_frame(index, build)?;
                let actual = gpu.render_frame(index, build)?;
                for (expected, actual) in expected.pixels().iter().zip(actual.pixels()) {
                    for shift in [0, 8, 16] {
                        assert!(((expected >> shift) & 255).abs_diff((actual >> shift) & 255) <= 1);
                    }
                }
                if index == 17 {
                    assert_eq!(first.pixels(), actual.pixels());
                }
            }
        }
        Ok(())
    }
}
