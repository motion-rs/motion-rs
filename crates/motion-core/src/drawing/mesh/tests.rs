//! Unit tests for mesh gradient validation and rasterization.

use super::*;
use crate::ValidationErrorKind;

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
