//! Unit tests for mesh gradient validation, baking and independent analytic patches.

use super::*;
use crate::ValidationErrorKind;

#[test]
fn analytic_patch_inverts_warped_affine_and_nearly_affine_geometry() {
    use crate::{Color, MeshGradient, MeshVertex, Point};
    let colors = [
        Color::from_hex(0xff4020).with_alpha(0.25),
        Color::from_hex(0x20dd30).with_alpha(0.5),
        Color::from_hex(0x2060ee).with_alpha(0.75),
        Color::WHITE,
    ];
    for positions in [
        [
            Point::new(0.0, 0.0),
            Point::new(1.0, 0.0),
            Point::new(0.0, 1.0),
            Point::new(1.0, 1.0),
        ],
        [
            Point::new(0.1, 0.2),
            Point::new(0.9, 0.35),
            Point::new(0.2, 1.1),
            Point::new(1.0, 1.25),
        ],
        [
            Point::new(0.0, 0.0),
            Point::new(1.0, 0.0),
            Point::new(0.0, 1.0),
            Point::new(1.0, 1.0000001),
        ],
        [
            Point::new(0.1, 0.0),
            Point::new(1.0, 0.2),
            Point::new(0.0, 0.9),
            Point::new(0.8, 1.0),
        ],
    ] {
        let mesh = MeshGradient::new(
            2,
            2,
            positions
                .into_iter()
                .zip(colors)
                .map(|(position, color)| MeshVertex { position, color })
                .collect(),
        )
        .unwrap();
        let patch = mesh.patch(0, 0).unwrap();
        for u in [0.0, 0.1, 0.5, 0.9, 1.0] {
            for v in [0.0, 0.1, 0.5, 0.9, 1.0] {
                let mix = |axis: fn(Point) -> f64| {
                    let top = axis(positions[0]) + (axis(positions[1]) - axis(positions[0])) * u;
                    let bottom = axis(positions[2]) + (axis(positions[3]) - axis(positions[2])) * u;
                    top + (bottom - top) * v
                };
                let point = Point::new(mix(|point| point.x), mix(|point| point.y));
                let actual = patch.sample(point).unwrap().unwrap();
                let smooth = |value: f64| value * value * (3.0 - 2.0 * value);
                let expected = colors[0]
                    .try_interpolate(&colors[1], smooth(u))
                    .unwrap()
                    .try_interpolate(
                        &colors[2].try_interpolate(&colors[3], smooth(u)).unwrap(),
                        smooth(v),
                    )
                    .unwrap()
                    .to_linear_premultiplied();
                for (actual, expected) in actual.into_iter().zip(expected) {
                    assert!(
                        (f64::from(actual) - expected).abs() <= 5e-7,
                        "{positions:?} {u} {v}"
                    );
                }
            }
        }
        assert!(patch.sample(Point::new(-2.0, -2.0)).unwrap().is_none());
        assert!(patch.sample(Point::new(f64::NAN, 0.0)).is_err());
    }
}

fn texel(image: &MeshImage, column: usize, row: usize) -> [f32; 4] {
    image.texel(0, column, row)
}

fn corners() -> MeshGradient {
    MeshGradient::grid(
        2,
        2,
        &[
            Color::from_hex(0xff0000),
            Color::from_hex(0x00ff00),
            Color::from_hex(0x0000ff),
            Color::WHITE,
        ],
    )
    .unwrap()
}

#[test]
fn rasterized_corners_match_vertex_colors_and_cover_every_pixel() {
    let image = corners()
        .rasterize(PixelSize::new(32, 32), 4, RasterLimits::default())
        .unwrap();
    assert!(image.pixels.iter().all(|pixel| pixel[3] == 1.0));
    let red = texel(&image, 0, 0);
    assert!(red[0] > 0.9 && red[1] < 0.05 && red[2] < 0.05);
    let white = texel(&image, 31, 31);
    assert!(white.iter().all(|value| *value > 0.9));
    let middle = texel(&image, 16, 16);
    assert!(middle[0] > 0.05 && middle[1] > 0.05 && middle[2] > 0.05);
}

#[test]
fn rasterizing_replays_identically_and_patterns_map_to_bounds() {
    let mesh = corners();
    let limits = RasterLimits::default();
    let first = mesh.rasterize(PixelSize::new(8, 8), 2, limits).unwrap();
    assert_eq!(
        first,
        mesh.rasterize(PixelSize::new(8, 8), 2, limits).unwrap()
    );
    let pattern = mesh
        .pattern(
            Rect::new(10.0, 20.0, 18.0, 36.0),
            PixelSize::new(8, 8),
            2,
            limits,
        )
        .unwrap();
    assert_eq!(
        pattern.transform.apply(Point::ORIGIN),
        Point::new(10.0, 20.0)
    );
    assert_eq!(
        pattern.transform.apply(Point::new(8.0, 8.0)),
        Point::new(18.0, 36.0)
    );
}

#[test]
fn displaced_vertices_warp_coverage() {
    let mut vertices = corners().vertices().to_vec();
    for vertex in &mut vertices {
        vertex.position = Point::new(vertex.position.x * 0.5, vertex.position.y);
    }
    let mesh = MeshGradient::new(2, 2, vertices).unwrap();
    let image = mesh
        .rasterize(PixelSize::new(16, 16), 2, RasterLimits::default())
        .unwrap();
    assert_eq!(texel(&image, 4, 8)[3], 1.0);
    assert_eq!(texel(&image, 12, 8), [0.0; 4]);
}

#[test]
fn invalid_meshes_and_limits_are_rejected() {
    assert!(MeshGradient::grid(1, 2, &[Color::WHITE, Color::BLACK]).is_err());
    assert!(MeshGradient::grid(2, 2, &[Color::WHITE]).is_err());
    let mesh = corners();
    let limits = RasterLimits::default();
    assert!(mesh.rasterize(PixelSize::new(4, 4), 0, limits).is_err());
    assert!(mesh.rasterize(PixelSize::new(0, 4), 1, limits).is_err());
    let small = RasterLimits {
        max_bytes: 64,
        ..limits
    };
    let error = mesh
        .rasterize(PixelSize::new(64, 64), 1, small)
        .unwrap_err();
    assert_eq!(error.kind, ValidationErrorKind::StructuralLimit);
}
