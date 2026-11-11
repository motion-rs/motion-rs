use super::*;
use crate::asset::AssetsExt;
use motion_core::{Asset, NodeModifiers};
use motion_render::RenderPrepare;

fn pixels(asset: &SvgAsset) -> motion_core::Result<motion_render::LinearFrame> {
    Ok(asset
        .tree
        .node()
        .compile(
            asset.size(),
            motion_core::PixelSize::new(
                asset.size().width.ceil() as u32,
                asset.size().height.ceil() as u32,
            ),
            motion_core::ValidationLimits::default(),
        )?
        .prepare(
            motion_core::ValidationLimits::default(),
            motion_core::RasterLimits::default(),
        )?
        .render_linear(motion_core::RasterLimits::default())?)
}

#[test]
fn svg_paths_ids_viewbox_and_transforms_enter_vectors() -> motion_core::Result<()> {
    let asset=SvgAsset::from_bytes(br#"<svg xmlns="http://www.w3.org/2000/svg" width="8" height="4" viewBox="10 20 8 4"><g id="group" transform="translate(10 20)"><path id="blue" fill="blue" d="M0 0h8v4h-8z"/></g></svg>"#)?;
    assert_eq!(asset.view_box, Rect::new(10.0, 20.0, 18.0, 24.0));
    assert!(asset.tree.get("blue").is_some());
    let frame = asset
        .tree
        .node()
        .compile(
            asset.size(),
            motion_core::PixelSize::new(8, 4),
            motion_core::ValidationLimits::default(),
        )?
        .prepare(
            motion_core::ValidationLimits::default(),
            motion_core::RasterLimits::default(),
        )?
        .render_linear(motion_core::RasterLimits::default())?;
    assert_eq!(frame.pixel(2, 2), Some([0.0, 0.0, 1.0, 1.0]));
    assert!(SvgAsset::from_bytes(br#"<svg xmlns="http://www.w3.org/2000/svg"><image href="file:///tmp/other.png"/></svg>"#).is_err());
    Ok(())
}

#[test]
fn svg_gradients_strokes_clips_masks_filters_and_diagnostics_are() -> motion_core::Result<()> {
    let asset=SvgAsset::from_bytes(br##"<svg xmlns="http://www.w3.org/2000/svg" width="16" height="8"><defs><linearGradient id="g"><stop stop-color="red"/><stop offset="1" stop-color="blue"/></linearGradient><clipPath id="c"><rect width="14" height="8"/></clipPath><mask id="m" maskUnits="userSpaceOnUse" x="0" y="0" width="16" height="8"><rect width="8" height="8" fill="white"/></mask></defs><g clip-path="url(#c)" mask="url(#m)"><rect id="art" width="16" height="8" fill="url(#g)" stroke="green" stroke-width="1" paint-order="stroke fill"/></g></svg>"##)?;
    let frame = pixels(&asset)?;
    assert!(frame.pixel(1, 3).unwrap()[0] > frame.pixel(1, 3).unwrap()[2]);
    assert_eq!(frame.pixel(10, 3), Some([0.0; 4]));
    assert!(asset.tree.get("art").is_some());
    assert!(asset.diagnostics().is_empty());
    let blur=SvgAsset::from_bytes(br##"<svg xmlns="http://www.w3.org/2000/svg" width="12" height="12"><defs><filter id="b" x="-100%" y="-100%" width="300%" height="300%"><feGaussianBlur stdDeviation="1"/></filter></defs><rect x="4" y="4" width="4" height="4" fill="red" filter="url(#b)"/></svg>"##)?;
    assert!(blur.diagnostics().is_empty());
    let frame = pixels(&blur)?;
    assert!(frame.pixel(5, 3).unwrap()[3] > 0.0);
    assert!(frame.pixel(5, 5).unwrap()[3] > frame.pixel(5, 3).unwrap()[3]);
    let matrix=SvgAsset::from_bytes(br##"<svg xmlns="http://www.w3.org/2000/svg" width="4" height="4"><defs><filter id="f"><feColorMatrix values="0 0 1 0 0 0 1 0 0 0 1 0 0 0 0 0 0 0 1 0"/></filter></defs><rect width="4" height="4" fill="red" filter="url(#f)"/></svg>"##)?;
    assert_eq!(pixels(&matrix)?.pixel(2, 2), Some([0.0, 0.0, 1.0, 1.0]));
    let unsupported=SvgAsset::from_bytes(br##"<svg xmlns="http://www.w3.org/2000/svg" width="4" height="4"><defs><filter id="f"><feTurbulence/></filter></defs><rect width="4" height="4" fill="red" filter="url(#f)"/></svg>"##)?;
    assert_eq!(unsupported.diagnostics().len(), 1);
    assert_eq!(
        pixels(&unsupported)?.pixel(2, 2),
        Some([1.0, 0.0, 0.0, 1.0])
    );
    let clip=SvgAsset::from_bytes(br##"<svg xmlns="http://www.w3.org/2000/svg" width="12" height="4"><defs><clipPath id="c"><rect width="4" height="4"/><rect x="8" width="4" height="4" visibility="hidden"/></clipPath></defs><rect width="12" height="4" fill="red" clip-path="url(#c)"/></svg>"##)?;
    assert_eq!(pixels(&clip)?.pixel(2, 2), Some([1.0, 0.0, 0.0, 1.0]));
    assert_eq!(pixels(&clip)?.pixel(10, 2), Some([0.0; 4]));
    Ok(())
}

#[test]
fn svg_lazy_selection_writing_images_and_security_keep_original_resources()
-> motion_core::Result<()> {
    let missing = motion_core::Assets::new(".")?;
    let captured = Svg::new(missing.svg("missing-native-svg-diagnostic.svg")).translate(1.0, 1.0);
    let composition =
        motion_render::Composition::new(8, 8, motion_render::FrameClock::new(30, 150));
    let error = composition
        .render_frame(0, Size::new(8.0, 8.0), |_| captured.clone())
        .err()
        .expect("strict SVG resource failure");
    assert_eq!(error.kind(), motion_core::ErrorKind::Asset);
    assert_eq!(error.frame(), Some(0));
    let placeholder = composition
        .configure(|options| options.allow_missing_assets(true))
        .render_frame(0, Size::new(8.0, 8.0), |_| captured.clone())?;
    assert!(placeholder.pixels().contains(&0xff00ff));
    let directory = tempfile::tempdir()?;
    let path = directory.path().join("art.svg");
    std::fs::write(&path,br#"<svg xmlns="http://www.w3.org/2000/svg" width="8" height="4"><rect id="art" width="8" height="4" fill="blue"/></svg>"#)?;
    let assets = motion_core::Assets::new(directory.path())?;
    let handle = assets.svg("art.svg");
    let asset = handle.get()?;
    std::fs::remove_file(path)?;
    let view = Svg::new(handle)
        .fit(Size::new(8.0, 8.0))
        .select("#art", |node| node.opacity(0.5))
        .write(1.0);
    let node = view.try_node()?;
    let frame = node
        .compile(
            Size::new(8.0, 8.0),
            motion_core::PixelSize::new(8, 8),
            motion_core::ValidationLimits::default(),
        )?
        .prepare(
            motion_core::ValidationLimits::default(),
            motion_core::RasterLimits::default(),
        )?
        .render_linear(motion_core::RasterLimits::default())?;
    assert_eq!(frame.pixel(2, 0), Some([0.0; 4]));
    assert_eq!(frame.pixel(2, 3), Some([0.0, 0.0, 0.5, 0.5]));
    assert!(
        Svg::from_asset(asset)
            .select("missing", |node| node)
            .try_node()
            .is_err()
    );
    let mut bytes = Vec::new();

    use image::ImageEncoder;
    image::codecs::png::PngEncoder::new(&mut bytes)
        .write_image(&[255, 0, 0, 128], 1, 1, image::ExtendedColorType::Rgba8)
        .map_err(crate::image_error)?;
    let escaped = bytes
        .iter()
        .map(|byte| format!("%{byte:02X}"))
        .collect::<String>();
    let source = format!(
        "<svg xmlns='http://www.w3.org/2000/svg' width='1' height='1'><image width='1' height='1' href='data:image/png,{escaped}'/></svg>"
    );
    let image = SvgAsset::from_bytes(source.as_bytes())?;
    assert!(image.retained_bytes() > 0);
    assert_eq!(
        motion_render::FrameBuffer::from_linear_rgba8(
            &pixels(&image)?,
            motion_core::RasterLimits::default()
        )?
        .bytes(),
        [255, 0, 0, 128]
    );
    for body in [
        "<script/>",
        "<image href='file:///tmp/file.png'/>",
        "<foreignObject/>",
        "<animate/>",
        "<image href='data:image/png,bad'/>",
    ] {
        let source =
            format!("<svg xmlns='http://www.w3.org/2000/svg' width='4' height='4'>{body}</svg>");
        assert!(SvgAsset::from_bytes(source.as_bytes()).is_err(), "{body}");
    }
    assert!(SvgAsset::from_bytes(b"<!DOCTYPE svg [<!ENTITY x '1'>]><svg xmlns='http://www.w3.org/2000/svg' width='&x;' height='1'/>").is_err());
    Ok(())
}

#[test]
fn nested_svg_retains_vector_content_without_raster_fallback() -> motion_core::Result<()> {
    let nested = "<svg xmlns='http://www.w3.org/2000/svg' width='2' height='2'><rect width='2' height='2' fill='lime'/></svg>";
    let escaped = nested
        .as_bytes()
        .iter()
        .map(|byte| format!("%{byte:02X}"))
        .collect::<String>();
    let source = format!(
        "<svg xmlns='http://www.w3.org/2000/svg' width='2' height='2'><image width='2' height='2' href='data:image/svg+xml,{escaped}'/></svg>"
    );
    let asset = SvgAsset::from_bytes(source.as_bytes())?;
    assert!(
        asset
            .tree
            .paint_leaves()
            .iter()
            .all(|leaf| matches!(leaf.item, VectorItem::Path(_)))
    );
    assert_eq!(pixels(&asset)?.pixel(0, 0), Some([0.0, 1.0, 0.0, 1.0]));
    Ok(())
}

#[test]
fn svg_isolation_nested_ids_and_alpha_filter_inputs_preserve_semantics() -> motion_core::Result<()>
{
    let named=Arc::new(SvgAsset::from_bytes(br#"<svg xmlns="http://www.w3.org/2000/svg" id="canvas" width="4" height="4"><g id="gear"><rect width="4" height="4" fill="blue"/></g></svg>"#)?);
    assert!(named.tree.ids().contains_key("canvas"));
    assert!(named.tree.get("gear").is_some());
    let selected = Svg::from_asset(named)
        .select("#canvas", |node| node.opacity(0.5))
        .try_node()?
        .compile(
            Size::new(4.0, 4.0),
            motion_core::PixelSize::new(4, 4),
            motion_core::ValidationLimits::default(),
        )?
        .prepare(
            motion_core::ValidationLimits::default(),
            motion_core::RasterLimits::default(),
        )?
        .render_linear(motion_core::RasterLimits::default())?;
    assert_eq!(selected.pixel(1, 1), Some([0.0, 0.0, 0.5, 0.5]));
    let isolated=SvgAsset::from_bytes(br#"<svg xmlns="http://www.w3.org/2000/svg" width="4" height="4"><rect width="4" height="4" fill="blue"/><g style="isolation:isolate"><rect width="4" height="4" fill="red" style="mix-blend-mode:multiply"/></g></svg>"#)?;
    assert_eq!(pixels(&isolated)?.pixel(1, 1), Some([1.0, 0.0, 0.0, 1.0]));
    let alpha=SvgAsset::from_bytes(br##"<svg xmlns="http://www.w3.org/2000/svg" width="4" height="4"><defs><filter id="a"><feGaussianBlur in="SourceAlpha" stdDeviation="0"/></filter></defs><rect width="4" height="4" fill="red" opacity=".5" filter="url(#a)"/></svg>"##)?;
    assert!(alpha.diagnostics().is_empty());
    let pixel = pixels(&alpha)?.pixel(2, 2).unwrap();
    assert_eq!(&pixel[..3], &[0.0; 3]);
    assert!((pixel[3] - 0.5).abs() < 0.01);
    let nested = "<svg xmlns='http://www.w3.org/2000/svg' width='2' height='2'><rect id='inside' width='2' height='2' fill='lime'/></svg>";
    let escaped = nested
        .as_bytes()
        .iter()
        .map(|byte| format!("%{byte:02X}"))
        .collect::<String>();
    let source = format!(
        "<svg xmlns='http://www.w3.org/2000/svg' width='4' height='2'><image id='left' width='2' height='2' href='data:image/svg+xml,{escaped}'/><image id='right' x='2' width='2' height='2' href='data:image/svg+xml,{escaped}'/></svg>"
    );
    let asset = SvgAsset::from_bytes(source.as_bytes())?;
    assert!(asset.tree.get("left::inside").is_some());
    assert!(asset.tree.get("right::inside").is_some());
    Ok(())
}

#[test]
#[ignore = "requires explicit MOTION_TEST_FONT; no system discovery"]
fn svg_text_uses_shared_fonts_and_outlined_geometry() -> motion_core::Result<()> {
    let path = std::env::var_os("MOTION_TEST_FONT")
        .ok_or_else(|| std::io::Error::other("Set MOTION_TEST_FONT"))?;
    let mut fonts = crate::FontLibrary::new();
    fonts.load_file(path)?;
    let source=br#"<svg xmlns="http://www.w3.org/2000/svg" width="96" height="32"><text id="label" x="4" y="23" font-family="Arial" font-size="20" fill="blue">Hello</text></svg>"#;
    assert!(SvgAsset::from_bytes(source).is_err());
    let asset = SvgAsset::from_bytes_with_fonts(source, &fonts)?;
    assert!(asset.tree.get("label").is_some());
    assert!(pixels(&asset)?.pixels().iter().any(|pixel| pixel[3] > 0.5));
    assert!(
        asset
            .tree
            .paint_leaves()
            .iter()
            .all(|leaf| matches!(leaf.item, VectorItem::Path(_)))
    );
    Ok(())
}

#[cfg(feature = "gpu")]
#[test]
#[ignore = "requires real native GPU; no fonts, window or encoder"]
fn svg_effects_operations_and_focal_paints_match_cpu_gpu() -> motion_core::Result<()> {
    let scenes = [
        r##"<svg xmlns="http://www.w3.org/2000/svg" width="24" height="16"><defs><radialGradient id="g" fx="25%" fy="40%" gradientTransform="skewX(10)"><stop stop-color="yellow"/><stop offset="1" stop-color="blue"/></radialGradient></defs><rect id="art" x="2" y="2" width="20" height="12" fill="url(#g)"/></svg>"##,
        r##"<svg xmlns="http://www.w3.org/2000/svg" width="24" height="16"><defs><clipPath id="c"><circle cx="10" cy="8" r="7"/></clipPath><mask id="m" maskUnits="userSpaceOnUse" x="0" y="0" width="24" height="16" mask-type="alpha"><rect width="18" height="16" fill="white" opacity=".6"/></mask></defs><g id="art" clip-path="url(#c)" mask="url(#m)" opacity=".7"><rect width="24" height="16" fill="red"/><circle cx="12" cy="9" r="6" fill="blue"/></g></svg>"##,
        r##"<svg xmlns="http://www.w3.org/2000/svg" width="24" height="16"><defs><filter id="f" x="-100%" y="-100%" width="300%" height="300%"><feDropShadow dx="1" dy="2" stdDeviation="1" flood-color="green"/><feColorMatrix type="saturate" values=".5"/></filter></defs><path id="art" d="M4 3L20 3L12 12Z" fill="red" filter="url(#f)"/></svg>"##,
        r##"<svg xmlns="http://www.w3.org/2000/svg" width="24" height="16"><path id="art" d="M2 4Q12 18 22 4" fill="none" stroke="blue" stroke-width="2" stroke-linecap="round" stroke-dasharray="3 2"/></svg>"##,
    ];
    let limits = motion_core::RasterLimits::default();
    let mut gpu = motion_render::gpu::Gpu::new()?;
    let mut cases = 0;
    let mut maximum = 0u8;
    for source in scenes {
        let asset = Arc::new(SvgAsset::from_bytes(source.as_bytes())?);
        assert!(asset.diagnostics().is_empty(), "{:?}", asset.diagnostics());
        for progress in [0.0, 0.3, 0.85, 1.0, 0.3] {
            for space in [
                motion_core::BlendSpace::LinearRgb,
                motion_core::BlendSpace::Srgb,
            ] {
                let node = Svg::from_asset(asset.clone())
                    .select("art", |node| node.rotate(0.1).opacity(0.8))
                    .write(progress)
                    .fit(Size::new(28.0, 20.0))
                    .try_node()?;
                let prepared = node
                    .compile(
                        Size::new(28.0, 20.0),
                        motion_core::PixelSize::new(56, 40),
                        motion_core::ValidationLimits::default(),
                    )?
                    .prepare(motion_core::ValidationLimits::default(), limits)?
                    .blend_space(space);
                let cpu = prepared.render_linear(limits)?;
                let actual =
                    gpu.render(&prepared, motion_render::gpu::Plan::new(&prepared, limits)?)?;
                assert_eq!(
                    actual.pixels(),
                    gpu.render(&prepared, motion_render::gpu::Plan::new(&prepared, limits)?)?
                        .pixels()
                );
                let cpu = cpu.to_srgb_rgba8(false, limits)?;
                let actual = actual.to_srgb_rgba8(false, limits)?;
                for (cpu, actual) in cpu.bytes().iter().zip(actual.bytes()) {
                    maximum = maximum.max(cpu.abs_diff(*actual));
                }
                assert!(maximum <= 2, "SVG case={cases},max_byte={maximum}");
                cases += 1;
            }
        }
    }
    assert_eq!(cases, 40);
    println!("SVG cases={cases} max_byte={maximum} GPU replay=exact");
    Ok(())
}
