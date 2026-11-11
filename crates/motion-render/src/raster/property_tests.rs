//! Property tests comparing prepared coverage with CPU raster output.

use crate::{PreparedFrame, RenderPrepare};
use motion_core::{
    Color, Node, Path2D, PixelSize, Point, RasterLimits, Rect, Shape, Size, ValidationLimits,
};
use proptest::prelude::*;

fn prepare(node: Node) -> PreparedFrame {
    node.compile(
        Size::new(32.0, 32.0),
        PixelSize::new(32, 32),
        ValidationLimits::default(),
    )
    .unwrap()
    .prepare(ValidationLimits::default(), RasterLimits::default())
    .unwrap()
}

fn coverage(frame: &PreparedFrame) -> Vec<u8> {
    let mut pixels = vec![0; 32 * 32];
    for span in &frame.spans {
        for row in span.y..span.y + span.height {
            for column in span.start_x..span.end_x {
                pixels[(row * 32 + column) as usize] = 255;
            }
        }
    }
    for strip in &frame.strips {
        for row in 0..strip.height {
            for column in 0..strip.width {
                let offset = strip.alpha_offset + (row * strip.width + column) as usize;
                pixels[((strip.y + row) * 32 + strip.x + column) as usize] = frame.alphas[offset];
            }
        }
    }
    pixels
}

proptest! {
    #![proptest_config({
        let mut config = ProptestConfig::default();
        if std::env::var_os("PROPTEST_CASES").is_none() {
            config.cases = 128;
        }
        config
    })]

    #[test]
    fn compact_rectangle_coverage_matches_independent_pixel_areas(
        left in -64_i16..=512,
        top in -64_i16..=512,
        width in 1_u16..=256,
        height in 1_u16..=256,
    ) {
        let left = f64::from(left) / 16.0;
        let top = f64::from(top) / 16.0;
        let right = left + f64::from(width) / 16.0;
        let bottom = top + f64::from(height) / 16.0;
        let actual = coverage(&prepare(Rect::new(left, top, right, bottom).fill(Color::WHITE).into()));
        for row in 0..32 {
            for column in 0..32 {
                let horizontal = (right.min(f64::from(column + 1)) - left.max(f64::from(column))).max(0.0);
                let vertical = (bottom.min(f64::from(row + 1)) - top.max(f64::from(row))).max(0.0);
                let expected = (horizontal * vertical * 255.0).round() as u8;
                prop_assert_eq!(actual[(row * 32 + column) as usize], expected);
            }
        }
    }

    #[test]
    fn polygon_coverage_is_finite_bounded_and_consistent_with_records(
        points in prop::array::uniform6(-64_i16..=576),
    ) {
        let mut path = Path2D::new();
        path.try_move_to(Point::new(f64::from(points[0]) / 16.0, f64::from(points[1]) / 16.0)).unwrap();
        for pair in points[2..].as_chunks::<2>().0 {
            path.try_line_to(Point::new(f64::from(pair[0]) / 16.0, f64::from(pair[1]) / 16.0)).unwrap();
        }
        path.try_close().unwrap();
        let frame = prepare(path.fill(Color::WHITE).into());
        let expected = coverage(&frame);
        let linear = frame.render_linear(RasterLimits::default()).unwrap();
        for row in 0..32 {
            for column in 0..32 {
                let pixel = linear.linear_pixel(column, row).unwrap();
                prop_assert!(pixel.iter().all(|value| value.is_finite() && (0.0..=1.0).contains(value)));
                let alpha = f32::from(expected[(row * 32 + column) as usize]) / 255.0;
                prop_assert!((pixel[3] - alpha).abs() <= f32::EPSILON);
                prop_assert!(pixel[..3].iter().all(|value| (*value - alpha).abs() <= f32::EPSILON));
            }
        }
    }
}
