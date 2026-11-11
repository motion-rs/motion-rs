use super::*;
use crate::{FrameBuffer, PixelFormat, WorkStats};
use motion_core::ValidationErrorKind;

#[test]
fn yuv_single_pass_matches_scalar_planes() {
    let size = PixelSize::new(18, 10);
    let mut state = 0x1234_5678_u32;
    let pixels = (0..size.width * size.height)
        .map(|index| {
            if index % 7 == 0 {
                return [0.25, 0.5, 0.75, 1.0];
            }
            let mut pixel = [0.0; 4];
            for channel in &mut pixel[..3] {
                state = state.wrapping_mul(1664525).wrapping_add(1013904223);
                *channel = (state >> 8) as f32 / 16777215.0 * 3.0 - 1.0;
            }
            pixel[3] = 1.0;
            pixel
        })
        .collect();
    let frames = [
        LinearFrame {
            size,
            pixels,
            stats: WorkStats::default(),
        },
        LinearFrame {
            size,
            pixels: vec![[0.25, 0.5, 0.75, 1.0]; 180],
            stats: WorkStats::default(),
        },
        LinearFrame {
            size,
            pixels: (0..180)
                .map(|index| [if index % 3 == 0 { -0.0 } else { 0.0 }, 1.0, 4.0, 1.0])
                .collect(),
            stats: WorkStats::default(),
        },
    ];
    let encode = |pixel: [f32; 4]| {
        let rgb = [pixel[0], pixel[1], pixel[2]]
            .map(|value| f64::from(motion_core::linear_to_srgb(value.clamp(0.0, 1.0))));
        let luma = 0.2126 * rgb[0] + 0.7152 * rgb[1] + 0.0722 * rgb[2];
        [luma, (rgb[2] - luma) / 1.8556, (rgb[0] - luma) / 1.5748]
    };
    for frame in frames {
        for bit_depth in [8, 10] {
            let scale = if bit_depth == 8 { 1.0 } else { 4.0 };
            let mut expected = [Vec::new(), Vec::new(), Vec::new()];
            let push = |data: &mut Vec<u8>, value: f64| {
                let sample = value.round() as u16;
                if bit_depth == 8 {
                    data.push(sample as u8);
                } else {
                    data.extend(sample.to_le_bytes());
                }
            };
            for pixel in frame.pixels() {
                push(&mut expected[0], (16.0 + 219.0 * encode(*pixel)[0]) * scale);
            }
            for row in (0..size.height).step_by(2) {
                for column in (0..size.width).step_by(2) {
                    let mut chroma = [0.0; 2];
                    for vertical in 0..2 {
                        for horizontal in 0..2 {
                            let color =
                                encode(frame.pixel(column + horizontal, row + vertical).unwrap());
                            chroma[0] += color[1] * 0.25;
                            chroma[1] += color[2] * 0.25;
                        }
                    }
                    push(&mut expected[1], (128.0 + 224.0 * chroma[0]) * scale);
                    push(&mut expected[2], (128.0 + 224.0 * chroma[1]) * scale);
                }
            }
            let actual = FrameBuffer::from_linear(
                &frame,
                PixelFormat::Yuv420 { bit_depth },
                RasterLimits::default(),
            )
            .unwrap();
            for (plane, expected) in actual.planes().iter().zip(expected) {
                assert_eq!(plane.bytes(), expected, "bit depth {bit_depth}");
            }
        }
    }
}

#[test]
fn byte_lookup_matches_the_threshold_search_for_every_region_of_the_range() {
    let levels = srgb_levels();
    let top = 1.0_f32.to_bits();
    for rank in 0..16 {
        let thresholds = &srgb_thresholds()[rank];
        // Both edges of every bucket, which is where an index can go wrong.
        let buckets = levels.buckets[rank].len() as u32;
        for bucket in 0..buckets {
            let start = levels.floor + (bucket << BUCKET_SHIFT);
            for bits in [
                start.wrapping_sub(1),
                start,
                start + 1,
                start + (1 << BUCKET_SHIFT) - 1,
            ] {
                let value = f32::from_bits(bits);
                assert_eq!(
                    levels.byte(rank, value),
                    threshold_byte(value, thresholds),
                    "rank {rank} bucket {bucket} bits {bits:#x}"
                );
            }
        }
        // Pseudo-random positions across all exponents, plus the special values.
        let mut state = 0x9e37_79b9_u32 ^ rank as u32;
        for _ in 0..200_000 {
            state = state.wrapping_mul(1664525).wrapping_add(1013904223);
            let value = f32::from_bits(1 + (state >> 1) % (top - 1));
            assert_eq!(
                levels.byte(rank, value),
                threshold_byte(value, thresholds),
                "rank {rank} bits {:#x}",
                value.to_bits()
            );
        }
        for value in [
            0.0,
            -0.0,
            f32::from_bits(1),
            f32::MIN_POSITIVE,
            0.999_999_94,
            1.0,
            1.5,
            f32::INFINITY,
            f32::NEG_INFINITY,
            f32::NAN,
            -1.0,
        ] {
            assert_eq!(
                levels.byte(rank, value),
                threshold_byte(value, thresholds),
                "rank {rank} value {value}"
            );
        }
    }
}

#[test]
fn encoding_into_bytes_matches_encoding_pixel_by_pixel() {
    let mut state = 0x1357_9bdf_u32;
    let mut next = move || {
        state = state.wrapping_mul(1664525).wrapping_add(1013904223);
        (state >> 8) as f32 / 16777215.0
    };
    for width in [1, 3, 7, 16, 33] {
        let mut source = Vec::new();
        for index in 0..width * 9 {
            let alpha = [1.0, 1.0, 0.0, 0.5, next(), 1.0][index % 6];
            source.push([next() * 1.2 - 0.1, next(), next() * 3.0, alpha]);
        }
        for premultiplied in [false, true] {
            let mut expected = Vec::new();
            RgbaEncoder::new(premultiplied).encode(&source, width, |pixel| {
                expected.extend_from_slice(&pixel);
            });
            let mut actual = vec![9];
            RgbaEncoder::new(premultiplied).encode_bytes(&source, width, &mut actual);
            assert_eq!(actual[0], 9, "existing bytes stay in place");
            assert_eq!(&actual[1..], &expected[..], "{width} {premultiplied}");
        }
    }
}

#[test]
fn rgba8_thresholds_match_scalar_boundaries_and_alpha_modes() {
    let levels = srgb_levels();
    for (rank, thresholds) in srgb_thresholds().iter().enumerate() {
        for threshold in thresholds {
            for bits in [threshold - 1, *threshold, threshold + 1] {
                let value = f32::from_bits(bits);
                let expected = (motion_core::linear_to_srgb(value) * 255.0
                    + dither_offset(rank as u8))
                .round() as u8;
                assert_eq!(
                    threshold_byte(value, thresholds),
                    expected,
                    "{rank}: {bits}"
                );
                assert_eq!(levels.byte(rank, value), expected, "lookup {rank}: {bits}");
            }
        }
    }
    let mut state = 0x1234_5678_u32;
    let mut pixels = Vec::new();
    for index in 0..65536 {
        let alpha = [0.0, f32::from_bits(1), 0.001, 0.5, 0.999, 1.0][index % 6];
        let mut pixel = [0.0; 4];
        for channel in &mut pixel[..3] {
            state = state.wrapping_mul(1664525).wrapping_add(1013904223);
            *channel = (state >> 8) as f32 / 16777215.0 * 4.0 - 1.0;
        }
        pixel[3] = alpha;
        pixels.push(pixel);
    }
    pixels.extend(vec![[0.25, 0.5, 0.75, 1.0]; 256]);
    pixels.extend(vec![[0.125, 0.25, 0.375, 0.5]; 256]);
    pixels.extend(
        [
            [0.25, 0.5, 0.75, 1.0],
            [0.0; 4],
            [0.25, 0.5, 0.75, 1.0],
            [0.125, 0.25, 0.375, 0.5],
        ]
        .repeat(64),
    );
    for source in [
        [0.0; 4],
        [-0.0, 0.0, -0.0, 1.0],
        [0.0, -0.0, 0.0, 1.0],
        [0.25, 0.5, 0.75, 1.0],
        [0.125, 0.25, 0.375, 0.5],
        [-1.0, 2.0, 4.0, 1.0],
    ] {
        pixels.extend([source; 257]);
    }
    for width in [1, 3, 7, 17, 32] {
        let count = pixels.len() / width * width;
        let frame = LinearFrame {
            size: PixelSize::new(width as u32, (count / width) as u32),
            pixels: pixels[..count].to_vec(),
            stats: WorkStats::default(),
        };
        for premultiplied in [false, true] {
            let actual = frame
                .to_rgba8_frame(premultiplied, RasterLimits::default())
                .unwrap();
            let buffer = crate::FrameBuffer::from_linear(
                &frame,
                crate::PixelFormat::SrgbRgba8 { premultiplied },
                RasterLimits::default(),
            )
            .unwrap();
            assert_eq!(buffer.planes()[0].bytes(), actual.bytes());
            for (index, source) in frame.pixels().iter().enumerate() {
                let alpha = alpha_to_byte(source[3]);
                let mut expected = [0; 4];
                if alpha != 0 {
                    let offset = dither_offset(BAYER_4X4[(index / width) % 4][(index % width) % 4]);
                    for channel in 0..3 {
                        let straight =
                            (f64::from(source[channel]) / f64::from(source[3])).clamp(0.0, 1.0);
                        let encoded = motion_core::linear_to_srgb(straight as f32);
                        let encoded = if premultiplied {
                            encoded * source[3]
                        } else {
                            encoded
                        };
                        let byte = (encoded * 255.0 + offset).round() as u8;
                        expected[channel] = if premultiplied { byte.min(alpha) } else { byte };
                    }
                    expected[3] = alpha;
                }
                assert_eq!(
                    actual.pixels()[index],
                    expected,
                    "{width}: {index}: {premultiplied}"
                );
            }
        }
    }
}

#[test]
#[ignore = "Exhausts all nonnegative f32 inputs in [0, 1]; run with --release"]
fn rgba8_threshold_domain_is_exhaustively_byte_monotone() {
    let mut previous = 0.0;
    let mut descents = 0_u64;
    for bits in 1..=1.0_f32.to_bits() {
        let encoded = motion_core::linear_to_srgb(f32::from_bits(bits));
        if encoded < previous {
            descents += 1;
            for rank in 0..16 {
                let offset = dither_offset(rank);
                assert!(
                    (encoded * 255.0 + offset).round() as u8
                        >= (previous * 255.0 + offset).round() as u8,
                    "Quantized descent at {bits}, rank {rank}"
                );
            }
        }
        previous = encoded;
    }
    eprintln!("Verified 1065353217 inputs and 16 byte mappings; transfer descents: {descents}");
}

#[test]
#[ignore = "Exhausts all positive f32 inputs below 1 for 16 ranks; run with --release"]
fn byte_lookup_is_exhaustively_equal_to_counting_thresholds() {
    let levels = srgb_levels();
    for rank in 0..16 {
        let thresholds = &srgb_thresholds()[rank];
        // Ascending inputs let the reference count advance instead of being searched.
        let mut count = 0;
        for bits in 1..1.0_f32.to_bits() {
            while count < thresholds.len() && thresholds[count] <= bits {
                count += 1;
            }
            assert_eq!(
                levels.byte(rank, f32::from_bits(bits)),
                count as u8,
                "rank {rank} bits {bits:#x}"
            );
        }
    }
}

#[test]
fn gamma_and_alpha_modes_use_straight_linear_channels_first() {
    let source = LinearFrame {
        size: PixelSize {
            width: 4,
            height: 1,
        },
        pixels: vec![
            [0.5, 0.5, 0.5, 1.0],
            [0.25, 0.0, 0.0, 0.5],
            [0.0; 4],
            [1.0; 4],
        ],
        stats: WorkStats::default(),
    };
    let straight = source
        .to_rgba8_frame(false, RasterLimits::default())
        .unwrap();
    let premultiplied = source
        .to_rgba8_frame(true, RasterLimits::default())
        .unwrap();
    assert_eq!(
        straight.pixels(),
        &[[187, 187, 187, 255], [188, 0, 0, 128], [0; 4], [255; 4]]
    );
    assert_eq!(premultiplied.pixel(1, 0), Some([94, 0, 0, 128]));
    assert!(!straight.premultiplied());
    assert!(premultiplied.premultiplied());
    assert_eq!(straight.bytes().len(), 16);
    assert_eq!(straight.pixel(4, 0), None);
    assert_eq!(straight.pixel(0, 1), None);
}

#[test]
fn ordered_dither_has_fixed_spatial_phase_and_no_channel_or_history_noise() {
    for (width, height) in [(1, 5), (3, 7), (8, 8), (17, 9)] {
        for step in 0..16 {
            let encoded = (64.0 + (step as f32 + 0.25) / 16.0) / 255.0;
            let linear = motion_core::srgb_to_linear(encoded);
            let source = LinearFrame {
                size: PixelSize::new(width, height),
                pixels: vec![[linear, linear, linear, 1.0]; (width * height) as usize],
                stats: WorkStats::default(),
            };
            let first = source
                .to_rgba8_frame(false, RasterLimits::default())
                .unwrap();
            let _other_mode = source
                .to_rgba8_frame(true, RasterLimits::default())
                .unwrap();
            assert_eq!(
                first,
                source
                    .to_rgba8_frame(false, RasterLimits::default())
                    .unwrap()
            );
            for row in 0..height {
                for column in 0..width {
                    let low_bits = ((column ^ row) & 1) * 2 + (row & 1);
                    let high_bits = (((column ^ row) >> 1) & 1) * 2 + ((row >> 1) & 1);
                    let rank = low_bits * 4 + high_bits;
                    let expected = 64 + u8::from(step + rank >= 16);
                    assert_eq!(
                        first.pixel(column, row),
                        Some([expected, expected, expected, 255])
                    );
                }
            }
        }
    }
}

#[test]
fn hdr_clips_only_at_output_and_zero_quantized_alpha_clears_hidden_rgb() {
    let source = LinearFrame {
        size: PixelSize::new(6, 1),
        pixels: vec![
            [f32::MAX, -f32::MAX, 0.0, 1.0],
            [0.5, 5.0, -0.5, 0.25],
            [f32::MAX, -f32::MAX, 0.0, 1.0 / 255.0],
            [f32::MAX, 1.0, -1.0, f32::from_bits(1)],
            [1.0, -1.0, 0.5, 0.001],
            [1.0, -1.0, 0.5, 0.0],
        ],
        stats: WorkStats::default(),
    };
    let original = source.clone();
    let straight = source
        .to_rgba8_frame(false, RasterLimits::default())
        .unwrap();
    assert_eq!(
        straight.pixels(),
        &[
            [255, 0, 0, 255],
            [255, 255, 0, 64],
            [255, 0, 0, 1],
            [0; 4],
            [0; 4],
            [0; 4],
        ]
    );
    let premultiplied = source
        .to_rgba8_frame(true, RasterLimits::default())
        .unwrap();
    assert_eq!(premultiplied.pixel(2, 0), Some([1, 0, 0, 1]));
    for pixel in premultiplied.pixels() {
        assert!(pixel[..3].iter().all(|channel| *channel <= pixel[3]));
    }
    assert_eq!(source, original);
}

#[test]
fn alpha_rounding_and_premultiplied_bounds_hold_at_all_byte_midpoints() {
    let mut pixels = Vec::new();
    for byte in 0..255 {
        let midpoint = ((f64::from(byte) + 0.5) / 255.0) as f32;
        for alpha in [midpoint.next_down(), midpoint, midpoint.next_up()] {
            pixels.push([alpha; 4]);
        }
    }
    let source = LinearFrame {
        size: PixelSize::new(pixels.len() as u32, 1),
        pixels,
        stats: WorkStats::default(),
    };
    for mode in [false, true] {
        let output = source
            .to_rgba8_frame(mode, RasterLimits::default())
            .unwrap();
        for (input, pixel) in source.pixels().iter().zip(output.pixels()) {
            let expected_alpha = alpha_to_byte(input[3]);
            assert_eq!(pixel[3], expected_alpha);
            if mode {
                assert!(pixel[0] <= expected_alpha);
                assert!(expected_alpha - pixel[0] <= 1);
                assert_eq!(pixel[0], pixel[1]);
                assert_eq!(pixel[0], pixel[2]);
            } else {
                let expected_rgb = if expected_alpha == 0 { 0 } else { 255 };
                assert_eq!(
                    *pixel,
                    [expected_rgb, expected_rgb, expected_rgb, expected_alpha]
                );
            }
        }
    }
}

#[test]
fn exact_conversion_limits_succeed_and_short_limits_preserve_the_source() {
    let source = LinearFrame {
        size: PixelSize::new(4, 1),
        pixels: vec![[0.25, 0.0, 0.0, 0.5]; 4],
        stats: WorkStats::default(),
    };
    let original = source.clone();
    let output = source
        .to_rgba8_frame(false, RasterLimits::default())
        .unwrap();
    assert_eq!(output.size(), source.size());
    assert_eq!(output.stats().work, 4);
    assert_eq!(
        output.stats().peak_bytes,
        (size_of::<LinearFrame>() + size_of::<Rgba8Frame>() + 4 * (16 + 4)) as u64
    );
    let exact = RasterLimits {
        max_pixels: 4,
        max_bytes: output.stats().peak_bytes,
        max_work: 4,
    };
    assert_eq!(source.to_rgba8_frame(false, exact).unwrap(), output);
    for (limits, field) in [
        (
            RasterLimits {
                max_pixels: 3,
                ..exact
            },
            "max_pixels",
        ),
        (
            RasterLimits {
                max_bytes: exact.max_bytes - 1,
                ..exact
            },
            "max_bytes",
        ),
        (
            RasterLimits {
                max_work: 3,
                ..exact
            },
            "max_work",
        ),
    ] {
        let error = source.to_rgba8_frame(false, limits).unwrap_err();
        assert_eq!(error.kind, ValidationErrorKind::StructuralLimit);
        assert_eq!(error.field, field);
        assert_eq!(source, original);
        assert_eq!(source.to_rgba8_frame(false, exact).unwrap(), output);
    }
    for limits in [
        RasterLimits {
            max_pixels: 0,
            ..exact
        },
        RasterLimits {
            max_bytes: 0,
            ..exact
        },
        RasterLimits {
            max_work: 0,
            ..exact
        },
    ] {
        assert_eq!(
            source.to_rgba8_frame(false, limits).unwrap_err().kind,
            ValidationErrorKind::InvalidValue
        );
    }
}
