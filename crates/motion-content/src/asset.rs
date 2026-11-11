//! Typed resource access through the unified lazy owner.

use crate::ImageAsset;
#[cfg(feature = "text")]
use motion_core::LoadContext;
use motion_core::{Asset, Assets, Handle};
#[cfg(test)]
use std::sync::Arc;

/// Typed loading through shared lazy handles.
pub trait AssetsExt {
    /// Loads one typed resource without reading or decoding it.
    fn asset<A: Asset>(&self, path: &str) -> Handle<A>;

    /// Loads or resolves an immutable image.
    fn image(&self, path: &str) -> Handle<ImageAsset> {
        self.asset(path)
    }

    /// Loads or resolves immutable SVG content.
    #[cfg(feature = "svg")]
    fn svg(&self, path: &str) -> Handle<crate::SvgAsset> {
        self.asset(path)
    }

    /// Loads or resolves an immutable animation.
    #[cfg(feature = "image")]
    fn animated_image(&self, path: &str) -> Handle<crate::AnimatedImageAsset> {
        self.asset(path)
    }

    /// Loads or resolves one explicit font face.
    #[cfg(feature = "text")]
    fn font(&self, path: &str) -> Handle<crate::FontFace> {
        self.asset(path)
    }
}

impl AssetsExt for Assets {
    fn asset<A: Asset>(&self, path: &str) -> Handle<A> {
        self.load(path)
    }
}

#[cfg(feature = "text")]
impl Asset for crate::FontFace {
    const KIND: &'static str = "font";

    fn decode(bytes: &[u8], _: &LoadContext) -> motion_core::Result<Self> {
        Ok(Self::from_bytes(bytes.to_vec(), 0)?)
    }

    fn retained_bytes(&self) -> usize {
        self.byte_len()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[cfg(feature = "svg")]
    #[test]
    fn source_drawables_use_the_same_owner_as_the_scene_callback() -> motion_core::Result {
        use motion_core::{Node, NodeModifiers, Size, Source};
        use motion_render::{Composition, RenderContext};
        let assets = Assets::new(".")?;
        assets.preload(
            "logo.png",
            Arc::new(ImageAsset::from_rgba8(2, 2, vec![255; 16])?),
        )?;
        let svg = assets.load::<crate::SvgAsset>(Source::Bytes {
            key: "parsed-svg".into(),
            data: Arc::from(&br##"<svg xmlns="http://www.w3.org/2000/svg" width="2" height="2"><rect width="2" height="2" fill="#00ff00"/></svg>"##[..]),
        }).get()?;
        assets.preload("diagram.svg", svg)?;
        let composition = Composition::new(
            8,
            4,
            motion_render::FrameClock::new(motion_core::fps!(30), 150),
        )
        .with_options(
            motion_render::RenderOptions::default()
                .context(RenderContext::new().with_assets(assets.clone())),
        )?;
        let frame = composition.render_frame(0, Size::new(8.0, 4.0), |context| {
            assert!(context.assets().store().same_store(assets.store()));
            for node in [
                Node::from(crate::Image::open("logo.png")),
                Node::from(crate::Svg::open("diagram.svg")),
            ] {
                assert_eq!(
                    context.measure(&node).unwrap(),
                    motion_core::Rect::new(0.0, 0.0, 2.0, 2.0)
                );
            }
            Node::group((
                crate::Image::open("logo.png"),
                crate::Svg::open("diagram.svg").translate(4.0, 0.0),
            ))
        })?;
        assert_eq!(frame.packed_rgb()[0], 0xffffff);
        assert_eq!(frame.packed_rgb()[4], 0x00ff00);
        let override_assets = Assets::new(".")?;
        override_assets.preload(
            "logo.png",
            Arc::new(ImageAsset::from_rgba8(1, 1, vec![255, 0, 0, 255])?),
        )?;
        let frame = composition
            .configure(|options| options.resources(override_assets.clone()))
            .render_frame(0, Size::new(8.0, 4.0), |context| {
                assert!(context.assets().store().same_store(override_assets.store()));
                crate::Image::open("logo.png").into()
            })?;
        assert_eq!(frame.packed_rgb()[0], 0xff0000);
        Ok(())
    }

    #[test]
    fn source_image_missing_files_follow_strict_and_placeholder_policies() -> motion_core::Result {
        use motion_core::Size;
        use motion_render::Composition;
        let directory = tempfile::tempdir()?;
        let assets = Assets::new(directory.path())?;
        let composition = Composition::new(
            2,
            2,
            motion_render::FrameClock::new(motion_core::fps!(30), 150),
        )
        .with_options(motion_render::RenderOptions::default().resources(assets))?;
        let build = |_: &motion_core::SceneContext<'_>| {
            crate::Image::open("absent.png").size(2.0, 2.0).into()
        };
        let error = composition
            .render_frame(0, Size::new(2.0, 2.0), build)
            .err()
            .unwrap();
        assert_eq!(error.kind(), motion_core::ErrorKind::Asset);
        assert_eq!(error.frame(), Some(0));
        let frame = composition
            .configure(|options| options.allow_missing_assets(true))
            .render_frame(0, Size::new(2.0, 2.0), build)?;
        assert_eq!(frame.packed_rgb()[0], 0xff00ff);
        Ok(())
    }

    #[cfg(feature = "text")]
    #[test]
    fn invalid_font_does_not_change_resources() {
        let assets = Assets::default();
        assert!(crate::FontFace::from_bytes(Vec::new(), 0).is_err());
        assert_eq!(assets.store().retained_bytes(), 0);
    }

    #[test]
    fn identifier_limit_bounds_tiny_shared_assets() -> motion_core::Result {
        let pixel = Arc::new(ImageAsset::from_rgba8(1, 1, vec![255; 4])?);
        let bytes = pixel.retained_bytes();
        let assets = Assets::new(".")?;
        for index in 0..4096 {
            assets.preload(format!("pixel-{index}"), pixel.clone())?;
        }
        assert!(assets.preload("overflow", pixel).is_err());
        assert_eq!(assets.store().retained_bytes(), 4096 * bytes);
        Ok(())
    }

    #[test]
    fn preloads_respect_allowance_and_snapshot_lifetimes() -> motion_core::Result {
        let image = Arc::new(ImageAsset::from_rgba8(2, 2, vec![255; 16])?);
        let bytes = image.retained_bytes();
        let pool = motion_core::ResourceStore::with_limits(motion_core::ResourceLimits {
            max_entries: 4096,
            max_bytes: bytes,
        })?;
        let assets = Assets::new(".")?.with_store(pool.clone());
        assets.preload("sun", image.clone())?;
        assert_eq!(pool.retained_bytes(), bytes);
        assert!(assets.preload("sun", image.clone()).is_err());
        assert!(assets.preload("moon", image.clone()).is_err());
        assert!(Arc::ptr_eq(&image, &assets.image("sun").get()?));
        let snapshot = assets.snapshot();
        assert!(assets.remove::<ImageAsset>("sun"));
        assert_eq!(pool.retained_bytes(), bytes);
        assert!(Arc::ptr_eq(&image, &snapshot.image("sun").get()?));
        drop(snapshot);
        assert_eq!(pool.retained_bytes(), 0);
        assert!(assets.remove::<ImageAsset>("moon"));
        assets.preload("moon", image)?;
        Ok(())
    }
}
