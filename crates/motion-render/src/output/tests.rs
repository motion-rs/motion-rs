use super::*;
use crate::raster::prepare::RenderPrepare;
use motion_core::Shape;
use motion_core::ValidationErrorKind;
use motion_core::{Color, Node, NodeModifiers, Rect, Size, ValidationLimits};

#[test]
fn yuv420_storage_validates_depth_planes_strides_and_limits() {
    let size = PixelSize::new(8, 2);
    let limits = RasterLimits::default();
    for bit_depth in [8, 10] {
        let sample_bytes = if bit_depth == 8 { 1 } else { 2 };
        let planes = [
            vec![64; 16 * sample_bytes],
            vec![128; 4 * sample_bytes],
            vec![240; 4 * sample_bytes],
        ];
        let frame =
            FrameBuffer::from_yuv420_storage(size, planes.clone(), bit_depth, limits).unwrap();
        assert_eq!(frame.format(), PixelFormat::Yuv420 { bit_depth });
        assert_eq!(frame.byte_len(), 24 * sample_bytes);
        for (index, plane) in frame.planes().iter().enumerate() {
            assert_eq!(plane.bytes(), planes[index]);
            assert_eq!(plane.width(), if index == 0 { 8 } else { 4 });
            assert_eq!(plane.height(), if index == 0 { 2 } else { 1 });
            assert_eq!(plane.stride(), plane.width() as usize * sample_bytes);
        }
        let exact = RasterLimits {
            max_bytes: frame.retained_bytes(),
            max_work: 16,
            max_pixels: 16,
        };
        assert_eq!(
            FrameBuffer::from_yuv420_storage(size, planes.clone(), bit_depth, exact).unwrap(),
            frame
        );
        assert!(
            FrameBuffer::from_yuv420_storage(
                size,
                planes.clone(),
                bit_depth,
                RasterLimits {
                    max_bytes: exact.max_bytes - 1,
                    ..exact
                }
            )
            .is_err()
        );
        for index in 0..3 {
            for extra in [false, true] {
                let mut invalid_planes = planes.clone();
                if extra {
                    invalid_planes[index].push(0);
                } else {
                    invalid_planes[index].pop();
                }
                assert!(
                    FrameBuffer::from_yuv420_storage(size, invalid_planes, bit_depth, limits)
                        .is_err()
                );
            }
        }
        assert!(
            FrameBuffer::from_yuv420_storage(
                PixelSize::new(7, 2),
                planes.clone(),
                bit_depth,
                limits
            )
            .is_err()
        );
        assert!(
            FrameBuffer::from_yuv420_storage(
                PixelSize::new(8, 1),
                planes.clone(),
                bit_depth,
                limits
            )
            .is_err()
        );
        if bit_depth == 8 {
            let [y, u, v] = planes;
            assert_eq!(
                FrameBuffer::from_yuv420(size, y, u, v, limits).unwrap(),
                frame
            );
        }
    }
    for bit_depth in [0, 9, 12, 16] {
        assert!(
            FrameBuffer::from_yuv420_storage(
                size,
                [vec![0; 16], vec![0; 4], vec![0; 4]],
                bit_depth,
                limits
            )
            .is_err()
        );
    }
}

#[test]
fn rgba8_storage_checks_preserve_public_transparent_normalization() {
    let size = PixelSize::new(2, 1);
    let limits = RasterLimits::default();
    let frame = FrameBuffer::from_rgba8(size, vec![1, 2, 3, 0, 9, 10, 11, 128], limits).unwrap();
    assert_eq!(frame.planes()[0].bytes(), &[0, 0, 0, 0, 9, 10, 11, 128]);
    for premultiplied in [false, true] {
        let actual = FrameBuffer::from_rgba8_storage(
            size,
            vec![1, 2, 3, 0, 9, 10, 11, 128],
            premultiplied,
            limits,
        )
        .unwrap();
        assert_eq!(actual.planes()[0].bytes(), &[1, 2, 3, 0, 9, 10, 11, 128]);
        assert_eq!(actual.format(), PixelFormat::SrgbRgba8 { premultiplied });
    }
    let exact = RasterLimits {
        max_bytes: frame.retained_bytes(),
        max_work: 2,
        max_pixels: 2,
    };
    assert_eq!(
        FrameBuffer::from_rgba8(size, vec![0; 8], exact)
            .unwrap()
            .size(),
        size
    );
    for (bytes, limits, kind) in [
        (vec![0; 7], limits, ValidationErrorKind::InvalidValue),
        (vec![0; 9], limits, ValidationErrorKind::InvalidValue),
        (
            vec![0; 8],
            RasterLimits {
                max_bytes: exact.max_bytes - 1,
                ..exact
            },
            ValidationErrorKind::StructuralLimit,
        ),
        (
            vec![0; 8],
            RasterLimits {
                max_work: 1,
                ..exact
            },
            ValidationErrorKind::StructuralLimit,
        ),
        (
            vec![0; 8],
            RasterLimits {
                max_pixels: 1,
                ..exact
            },
            ValidationErrorKind::StructuralLimit,
        ),
    ] {
        assert_eq!(
            FrameBuffer::from_rgba8(size, bytes.clone(), limits)
                .unwrap_err()
                .kind,
            kind
        );
        assert_eq!(
            FrameBuffer::from_rgba8_storage(size, bytes, true, limits)
                .unwrap_err()
                .kind,
            kind
        );
    }
}

#[test]
fn constant_bands_match_full_linear_bytes_and_bound_storage() {
    for size in [
        PixelSize::new(8, 2),
        PixelSize::new(18, 34),
        PixelSize::new(32, 64),
    ] {
        for pixel in [
            [0.0; 4],
            [0.125, 0.25, 0.375, 0.5],
            [0.25, 0.5, 0.75, 1.0],
            [4.0, -0.5, 0.0, 1.0],
        ] {
            for format in [
                PixelFormat::SrgbRgba8 {
                    premultiplied: false,
                },
                PixelFormat::SrgbRgba8 {
                    premultiplied: true,
                },
                PixelFormat::LinearRgba16Float,
                PixelFormat::Yuv420 { bit_depth: 8 },
                PixelFormat::Yuv420 { bit_depth: 10 },
            ] {
                let limits = RasterLimits::default();
                let reference =
                    FrameBuffer::from_linear(&LinearFrame::constant(size, pixel), format, limits);
                let actual = FrameBuffer::from_constant(size, pixel, format, limits);
                match (actual, reference) {
                    (Ok((actual, peak)), Ok(expected)) => {
                        assert_eq!(actual, expected);
                        let exact = RasterLimits {
                            max_bytes: peak,
                            ..limits
                        };
                        assert_eq!(
                            FrameBuffer::from_constant(size, pixel, format, exact)
                                .unwrap()
                                .0,
                            expected
                        );
                        assert!(
                            FrameBuffer::from_constant(
                                size,
                                pixel,
                                format,
                                RasterLimits {
                                    max_bytes: peak - 1,
                                    ..limits
                                }
                            )
                            .is_err()
                        );
                    }
                    (Err(actual), Err(expected)) => {
                        assert_eq!((actual.kind, actual.field), (expected.kind, expected.field))
                    }
                    other => panic!("Constant conversion mismatch: {other:?}"),
                }
            }
        }
    }
}

fn linear(color: Color, alpha: f64) -> LinearFrame {
    Node::from(Rect::new(0.0, 0.0, 2.0, 2.0).fill(color))
        .opacity(alpha)
        .compile(
            Size::new(2.0, 2.0),
            PixelSize::new(2, 2),
            ValidationLimits::default(),
        )
        .unwrap()
        .prepare(ValidationLimits::default(), RasterLimits::default())
        .unwrap()
        .render_linear(RasterLimits::default())
        .unwrap()
}

#[test]
fn output_formats_preserve_transparency_half_and_bt709_boundaries() -> motion_core::Result {
    let limits = RasterLimits::default();
    let red = linear(Color::rgb(255, 0, 0), 0.5);
    let bitmap = Bitmap::from_linear(&red, limits)?;
    assert_eq!(&bitmap.bytes()[..4], &[255, 0, 0, 128]);
    let half = FrameBuffer::from_linear(&red, PixelFormat::LinearRgba16Float, limits)?;
    assert_eq!(&half.planes()[0].bytes()[..8], &[0, 56, 0, 0, 0, 0, 0, 56]);
    assert!(FrameBuffer::from_linear(&red, PixelFormat::Yuv420 { bit_depth: 8 }, limits).is_err());
    for (color, expected) in [
        (Color::BLACK, [16, 128, 128]),
        (Color::WHITE, [235, 128, 128]),
        (Color::rgb(255, 0, 0), [63, 102, 240]),
    ] {
        let frame = FrameBuffer::from_linear(
            &linear(color, 1.0),
            PixelFormat::Yuv420 { bit_depth: 8 },
            limits,
        )?;
        assert_eq!(
            frame
                .planes()
                .iter()
                .map(|plane| plane.bytes()[0])
                .collect::<Vec<_>>(),
            expected
        );
    }
    let ten = FrameBuffer::from_linear(
        &linear(Color::WHITE, 1.0),
        PixelFormat::Yuv420 { bit_depth: 10 },
        limits,
    )?;
    assert_eq!(&ten.planes()[0].bytes()[..2], &940u16.to_le_bytes());
    let gray = linear(Color::rgb(128, 128, 128), 1.0);
    let encoded = FrameBuffer::from_linear(&gray, PixelFormat::Yuv420 { bit_depth: 8 }, limits)?;
    assert_eq!(
        encoded
            .planes()
            .iter()
            .map(|plane| plane.bytes()[0])
            .collect::<Vec<_>>(),
        [126, 128, 128]
    );
    let mut short = limits;
    short.max_work = 11;
    assert!(FrameBuffer::from_linear(&gray, PixelFormat::Yuv420 { bit_depth: 8 }, short).is_err());
    let mut target = FrameBuffer::new(PixelSize::new(2, 2), limits)?;
    let original = target.clone();
    assert!(
        FrameBuffer::write_linear(
            &red,
            RenderTarget::Memory(&mut target),
            PixelFormat::Yuv420 { bit_depth: 8 },
            limits
        )
        .is_err()
    );
    assert_eq!(target, original);
    let directory = tempfile::tempdir()?;
    let path = directory.path().join("alpha.png");
    bitmap.save_png(&path)?;
    assert_eq!(
        image::open(&path)
            .map_err(|error| motion_core::Error::Other(Box::new(error)))?
            .to_rgba8()
            .as_raw(),
        bitmap.bytes()
    );
    assert!(bitmap.save_png(&path).is_err());
    assert_eq!(directory.path().read_dir()?.count(), 1);
    Ok(())
}
