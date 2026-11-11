//! `DepthOfField::capture` must reach the renderer as per-plane bokeh blur.

use motion_core::Shape;
use motion_core::{
    Color, DepthOfField, DepthPlane, Node, PixelSize, RasterLimits, Rect, SceneCache, SceneContext,
    Size, Time, ValidationLimits,
};
use motion_render::{FrameBuffer, RenderPrepare};

fn lens() -> DepthOfField {
    DepthOfField {
        focus_distance: 1000.0,
        f_stop: 2.0,
        focal_length: 50.0,
        max_radius: 6.0,
        perspective: false,
    }
}

// Plane coordinates are camera-centred: the default camera maps (0, 0) to the canvas centre.
fn planes() -> Vec<DepthPlane> {
    let bright = |rect: Rect| Node::from(rect.fill(Color::WHITE));
    vec![
        DepthPlane {
            node: bright(Rect::new(-12.0, -12.0, -8.0, -8.0)),
            depth: 1000.0,
        },
        DepthPlane {
            node: bright(Rect::new(4.0, 4.0, 8.0, 8.0)),
            depth: 250.0,
        },
    ]
}

fn render(node: Node) -> FrameBuffer {
    node.compile(
        Size::new(32.0, 32.0),
        PixelSize::new(32, 32),
        ValidationLimits::default(),
    )
    .unwrap()
    .prepare(ValidationLimits::default(), RasterLimits::default())
    .unwrap()
    .render_linear(RasterLimits::default())
    .unwrap()
}

fn captured(lens: DepthOfField, planes: Vec<DepthPlane>) -> Node {
    let cache = SceneCache::new(0);
    let context = SceneContext::new(
        Time::ZERO,
        motion_core::fps!(30),
        Size::new(32.0, 32.0),
        &cache,
    );
    lens.capture(&context, Default::default(), planes).unwrap()
}

#[test]
fn focused_plane_is_sharp_and_defocused_plane_spreads_energy() {
    let lens = lens();
    assert_eq!(lens.radius(1000.0).unwrap(), 0.0);
    assert!(lens.radius(250.0).unwrap() > 1.0);
    let blurred = render(captured(lens, planes()));
    let sharp = render(captured(
        DepthOfField {
            max_radius: 0.0,
            ..lens
        },
        planes(),
    ));
    let at = |frame: &FrameBuffer, x: u32, y: u32| frame.linear_pixel(x, y).unwrap();
    // The focus plane is byte-identical with and without the lens.
    for y in 3..10 {
        for x in 3..10 {
            assert_eq!(at(&blurred, x, y), at(&sharp, x, y), "({x}, {y})");
        }
    }
    // The near plane loses peak coverage and leaks outside its rectangle.
    assert_eq!(at(&sharp, 25, 22)[3], 0.0);
    assert!(at(&blurred, 25, 22)[3] > 0.01);
    assert!(at(&blurred, 21, 21)[3] < at(&sharp, 21, 21)[3]);
    let total = |frame: &FrameBuffer| frame.linear_pixels().map(|pixel| pixel[3]).sum::<f32>();
    assert!((total(&blurred) - total(&sharp)).abs() < 0.02 * total(&sharp));
}

#[test]
fn radius_grows_with_distance_from_focus_and_respects_cap() {
    let lens = lens();
    let near = lens.radius(500.0).unwrap();
    let nearer = lens.radius(250.0).unwrap();
    assert!(nearer > near && near > 0.0);
    assert_eq!(
        DepthOfField {
            max_radius: 0.5,
            ..lens
        }
        .radius(250.0)
        .unwrap(),
        0.5
    );
}
