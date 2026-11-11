//! Integration tests for path morphing rendered through the facade.

use motion::prelude::*;
use motion::render::*;

#[test]
#[cfg(feature = "gpu")]
#[ignore = "requires native GPU; no fonts, encoder or window"]
fn morph_paths_match_gpu_and_replay_across_topology_changes() -> motion::Result {
    use motion::render::{RasterLimits, ValidationLimits};
    use motion::{Circle, Color, Node, PixelSize, Size, Star};
    let limits = RasterLimits::default();
    let mut gpu = motion::render::gpu::Gpu::new()?;
    let info = gpu.info().clone();
    let cases = [
        (
            Circle::new(Point::new(8.0, 8.0), 6.0)?.path()?,
            Star::new(Point::new(8.0, 8.0), 5, 6.0, 2.5)?.path()?,
        ),
        (
            Path2D::parse_svg("M1 1H15V15H1Z M5 5V11H11V5Z")?,
            Path2D::parse_svg("M2 2V14H14V2Z M6 6H10V10H6Z")?,
        ),
        (
            Path2D::new(),
            Path2D::parse_svg("M1 1H15V15H1Z M3 3V6H6V3Z")?,
        ),
        (
            Path2D::parse_svg("M1 12Q8 1 15 12")?,
            Path2D::parse_svg("M2 2H14L8 14Z")?,
        ),
    ];
    let mut maximum = 0.0_f32;
    for (source, target) in cases {
        let (first, second) = MorphPath::pair(source.clone(), target.clone())?;
        let mut saved = None;
        for progress in [0.0, 0.2, 0.5, 0.8, 1.0, 0.2] {
            let morph = first.try_interpolate(&second, progress)?;
            if progress == 0.0 {
                assert_eq!(morph.path()?, source);
            }
            if progress == 1.0 {
                assert_eq!(morph.path()?, target);
            }
            let prepared = Node::from(
                morph
                    .fill(Color::from_rgb_float(0.2, 0.7, 1.1))
                    .stroke(0.35, Color::WHITE),
            )
            .compile(
                Size::new(16.0, 16.0),
                PixelSize::new(16, 16),
                ValidationLimits::default(),
            )?
            .prepare(ValidationLimits::default(), limits)?;
            let cpu = prepared.render_linear(limits)?;
            let rendered = gpu.render_linear(
                &prepared,
                motion::render::gpu::Plan::new(&prepared, limits)?,
            )?;
            for row in 0..16 {
                for column in 0..16 {
                    for (first, second) in cpu
                        .linear_pixel(column, row)
                        .unwrap()
                        .into_iter()
                        .zip(rendered.linear_pixel(column, row).unwrap())
                    {
                        maximum = maximum.max((first - second).abs());
                    }
                }
            }
            let cpu = cpu.to_format(
                motion::render::PixelFormat::SrgbRgba8 {
                    premultiplied: false,
                },
                limits,
            )?;
            let rendered = rendered.to_format(
                motion::render::PixelFormat::SrgbRgba8 {
                    premultiplied: false,
                },
                limits,
            )?;
            for (first, second) in cpu.bytes().iter().zip(rendered.bytes()) {
                assert!(first.abs_diff(*second) <= 1);
            }
            if progress == 0.2 {
                if let Some((cpu_saved, gpu_saved)) = &saved {
                    assert_eq!(cpu.bytes(), cpu_saved);
                    assert_eq!(rendered.bytes(), gpu_saved);
                } else {
                    saved = Some((cpu.bytes().to_vec(), rendered.bytes().to_vec()));
                }
            }
        }
    }
    assert!(maximum <= 2e-6, "maximum linear difference {maximum}");
    eprintln!(
        "_morph_gpu adapter={} backend={:?} pairs=4 requests=24 max_linear_difference={maximum} rgba_tolerance=1 cpu_gpu_replay=exact",
        info.name, info.backend
    );
    Ok(())
}

fn render(path: &MorphPath) -> motion::render::FrameBuffer {
    use motion::render::{RasterLimits, ValidationLimits};
    use motion::{Color, Node, PixelSize, Size};
    Node::from(path.clone().fill(Color::WHITE))
        .compile(
            Size::new(16.0, 16.0),
            PixelSize::new(16, 16),
            ValidationLimits::default(),
        )
        .unwrap()
        .prepare(ValidationLimits::default(), RasterLimits::default())
        .unwrap()
        .render_linear(RasterLimits::default())
        .unwrap()
}

#[test]
fn morph_phase_alignment_and_holes_keep_rendered_coverage() -> motion::Result {
    let source = Path2D::parse_svg("M2 2H10V10H2Z")?;
    let target = Path2D::parse_svg("M10 10H2V2H10Z")?;
    let (first, second) = MorphPath::pair(source, target)?;
    assert_eq!(
        render(&first.interpolate(&second, 0.5)).linear_pixel(5, 5),
        Some([1.0; 4])
    );
    let source = Path2D::from_rect(Rect::new(0.0, 0.0, 16.0, 16.0))?;
    let target = Path2D::parse_svg("M0 0H16V16H0Z M6 6V10H10V6Z")?;
    let (first, second) = MorphPath::pair(source, target)?;
    assert_eq!(render(&first).linear_pixel(8, 8), Some([1.0; 4]));
    assert_eq!(
        render(&first.interpolate(&second, 0.5)).linear_pixel(8, 8),
        Some([0.0; 4])
    );
    let reversed = Path2D::parse_svg("M0 0V16H16V0Z M6 6H10V10H6Z")?;
    let (first, second) = MorphPath::pair(second.path()?, reversed)?;
    assert_eq!(
        render(&first.interpolate(&second, 0.5)).linear_pixel(8, 8),
        Some([0.0; 4])
    );
    Ok(())
}
