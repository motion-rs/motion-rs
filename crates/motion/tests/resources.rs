//! Integration tests for resource loading, errors and asset ownership.

mod resources {

    mod error {
        use motion::prelude::*;

        #[test]
        fn font_and_backend_error_diagnostics() {
            let directory = tempfile::tempdir().unwrap();
            std::fs::write(directory.path().join("invalid.ttf"), [0]).unwrap();
            let assets = motion::Assets::new(directory.path()).unwrap();
            let font = assets.font("invalid.ttf");
            let error = font.get().err().unwrap();
            assert_eq!(error.kind(), ErrorKind::Asset);
            assert!(
                matches!(&error, Error::Asset { kind: "font", source_name, .. } if source_name.ends_with("invalid.ttf"))
            );
            assert_eq!(
                error.downcast_ref::<ValidationError>().unwrap().kind,
                motion::render::ValidationErrorKind::InvalidValue
            );
            #[cfg(not(feature = "gpu"))]
            {
                let error = motion::render::Composition::new(1, 1)
                    .renderer(motion::Renderer::Gpu)
                    .frame_renderer(motion::Size::new(1.0, 1.0))
                    .err()
                    .unwrap();
                assert_eq!(error.kind(), ErrorKind::GpuUnavailable);
                assert_eq!(
                    error.downcast_ref::<ValidationError>().unwrap().field,
                    "renderer"
                );
            }
        }

        #[test]
        #[cfg(feature = "typst")]
        fn formula_error_diagnostics() {
            let error = motion::TypstCompiler::new()
                .compile("#undefined-function()")
                .err()
                .unwrap();
            assert_eq!(error.kind(), ErrorKind::Formula);
            assert!(error.to_string().contains("undefined-function"));
        }
    }

    mod resource {
        use motion::extend::*;
        use motion::prelude::*;
        use std::{io, sync::Arc};

        #[test]
        #[cfg(feature = "text")]
        fn exports_report_actual_explicit_fonts_without_cross_export_leaks() -> motion::Result {
            let directory = tempfile::tempdir()?;
            let face = FontFace::from_bytes(
                include_bytes!("../../../tests/fixtures/fonts/NotoSans-Regular.ttf").to_vec(),
                0,
            )?;
            let text = Text::from_font("Used font", &face, 12.0, Color::WHITE)?;
            let timeline = Timeline::new(Size::new(96.0, 32.0), 30)
                .track(|track| track.scene(100.milliseconds(), move |_| text.clone()));
            let mut options =
                ExportOptions::new(directory.path().join("text")).codec(Codec::PngSequence);
            options.workers = 2;
            let first = motion::export(&timeline, &options, &NoProgress)?;
            assert_eq!(first.fonts.len(), 1);
            assert!(first.fonts[0].name.as_ref().unwrap().contains("NotoSans"));
            assert_ne!(first.fonts[0].file_hash, 0);
            options.path = directory.path().join("replay");
            let second = motion::export(&timeline, &options, &NoProgress)?;
            assert_eq!(first.fonts, second.fonts);
            options.path = directory.path().join("empty");
            let empty = Timeline::new(Size::new(96.0, 32.0), 30)
                .track(|track| track.scene(100.milliseconds(), |_| Node::empty()));
            assert!(
                motion::export(&empty, &options, &NoProgress)?
                    .fonts
                    .is_empty()
            );
            Ok(())
        }

        #[test]
        fn font_snapshots_are_part_of_resource_identity() -> motion::Result {
            struct Environment(u64);

            impl Asset for Environment {
                const KIND: &'static str = "font-environment";

                fn decode(_: &[u8], context: &LoadContext) -> motion::Result<Self> {
                    Ok(Self(context.fonts().unwrap().snapshot_id()))
                }
            }
            let directory = tempfile::tempdir()?;
            let fonts = motion::FontLibrary::new();
            let first = Assets::new(directory.path())?.with_fonts(fonts.clone());
            let same = Assets::new(directory.path())?
                .with_pool(first.pool().clone())
                .with_fonts(fonts.clone());
            let different = Assets::new(directory.path())?
                .with_pool(first.pool().clone())
                .with_fonts(fonts.default_family("explicit-other"));
            let source = Source::Bytes {
                key: "same".into(),
                data: Arc::from(&b"same"[..]),
            };
            let left = first.load::<Environment>(source.clone()).get().unwrap();
            let equal = same.load::<Environment>(source.clone()).get().unwrap();
            let right = different.load::<Environment>(source).get().unwrap();
            assert!(Arc::ptr_eq(&left, &equal));
            assert!(!Arc::ptr_eq(&left, &right));
            assert_ne!(left.0, right.0);
            Ok(())
        }

        #[test]
        #[cfg(feature = "ffmpeg")]
        #[ignore = "requires native FFmpeg with libx264; no fonts or window"]
        fn missing_asset_export_preserves_destination_and_allow_missing_commits() -> motion::Result
        {
            use motion::{ExportOptions, Image, NoProgress, Node, Size};
            let directory = tempfile::tempdir()?;
            let destination = directory.path().join("output.mp4");
            std::fs::write(&destination, b"committed")?;
            let build = |_: &motion::SceneContext<'_>| {
                Node::from(Image::open("missing.png").size(8.0, 8.0))
            };
            let timeline = Timeline::try_new(Size::new(8.0, 8.0), 30)?
                .try_track(|track| track.try_push(build.with_duration(100.milliseconds())))?;
            let mut options = ExportOptions::new(&destination);
            options.overwrite = true;
            options.render.resources = Some(Assets::new(directory.path())?);
            let error = motion::export(&timeline, &options, &NoProgress)
                .err()
                .unwrap();
            assert_eq!(error.kind(), motion::ErrorKind::Asset);
            assert_eq!(error.frame(), Some(0));
            assert_eq!(std::fs::read(&destination)?, b"committed");
            assert_eq!(directory.path().read_dir()?.count(), 1);
            options.render.allow_missing_assets = true;
            motion::export(&timeline, &options, &NoProgress)?;
            assert_eq!(motion_ffmpeg::inspect::video_frames(&destination, 3)?, 3);
            assert_eq!(directory.path().read_dir()?.count(), 1);
            Ok(())
        }

        #[test]
        fn missing_images_are_magenta_but_strict_frames_fail_and_diagnostics_do_not_leak()
        -> motion::Result {
            use motion::render::Composition;
            use motion::{Color, Image, Node, Rect, Size};
            let directory = tempfile::tempdir()?;
            let assets = Assets::new(directory.path())?;
            let composition = Composition::new(8, 8).resources(assets.clone());
            let mut strict = composition.frame_renderer(Size::new(8.0, 8.0))?;
            let error = strict
                .render_frame(3, |_| Node::from(Image::open("missing.png").size(8.0, 8.0)))
                .err()
                .unwrap();
            assert_eq!(error.kind(), motion::ErrorKind::Asset);
            assert_eq!(error.frame(), Some(3));
            assert_eq!(
                error.downcast_ref::<io::Error>().unwrap().kind(),
                io::ErrorKind::NotFound
            );
            assert!(strict.last_stats().is_none());
            assert_eq!(strict.asset_diagnostics().len(), 1);
            let mut preview = composition
                .allow_missing_assets(true)
                .frame_renderer(Size::new(8.0, 8.0))?;
            let frame = preview
                .render_frame(4, |_| Node::from(Image::open("missing.png").size(8.0, 8.0)))?;
            assert!(frame.pixels().iter().all(|pixel| *pixel == 0xff00ff));
            assert_eq!(preview.asset_diagnostics()[0].frame(), Some(4));
            preview.render_frame(1, |_| {
                Node::from(Rect::new(0.0, 0.0, 8.0, 8.0).fill(Color::WHITE))
            })?;
            assert!(preview.asset_diagnostics().is_empty());
            let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                preview.render_frame(2, |_| {
                    let _error = assets.image("another-missing.png").get();
                    panic!("expected scene panic");
                })
            }));
            assert!(result.is_err());
            preview.render_frame(0, |_| Node::empty())?;
            assert!(preview.asset_diagnostics().is_empty());
            Ok(())
        }
    }
}
