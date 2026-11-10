//! Path, surface, image and glyph emission plus the four analytic trajectory models.

use super::{SOURCE_SECONDS, Specimen, Study, VIEWPORT, base};
use crate::DemoParameters;
use motion::extend::DrawContext;
use motion::particles::{
    Ballistic, Converge, ImageDensity, ImageEmitter, NoiseDrift, Orbit, ParticleSimulation,
    ParticleStyle, ParticleTrajectory, PathEmitter, SimulationLimits, SurfaceEmitter,
};
use motion::prelude::*;
use std::sync::Arc;

/// Fixed source-image side in texels, admitted independently of user density.
const IMAGE_SIDE: u32 = 48;

pub(super) fn path(parameters: DemoParameters) -> motion::Result<Study> {
    let path = Path2D::builder()
        .move_to(Point::new(60.0, 210.0))
        .cubic_to(
            Point::new(150.0, -20.0),
            Point::new(360.0, 324.0),
            Point::new(452.0, 90.0),
        )
        .build()?;
    Ok(Study::new(
        base(parameters).emitter(
            PathEmitter::new(Arc::new(path))
                .along_path(Tween::new(0.0, 1.0, SOURCE_SECONDS.seconds()))
                .rate(f64::from(parameters.count) * 2.0)
                .speed(28.0)
                .lifetime(2.4.seconds())
                .color([0.2, 0.9, 0.65, 1.0]),
        ),
    ))
}

pub(super) fn surface(parameters: DemoParameters) -> motion::Result<Study> {
    let logo = super::super::particle_recipes::logo(VIEWPORT)?;
    Ok(Study::new(
        base(parameters).emitter(
            SurfaceEmitter::new(logo)
                .inherit_color(true)
                .rate(f64::from(parameters.count) * 2.0)
                .speed(16.0)
                .lifetime(3.seconds()),
        ),
    ))
}

pub(super) fn image_alpha(parameters: DemoParameters) -> motion::Result<Study> {
    image_study(parameters, ImageDensity::Alpha)
}

pub(super) fn image_luminance(parameters: DemoParameters) -> motion::Result<Study> {
    image_study(parameters, ImageDensity::Luminance)
}

fn image_study(parameters: DemoParameters, density: ImageDensity) -> motion::Result<Study> {
    let emitter = ImageEmitter::new(
        source_image()?,
        Rect::new(140.0, 36.0, 372.0, 268.0),
        density,
        SimulationLimits::default(),
    )?
    .rate(f64::from(parameters.count) * 3.0)
    .speed(6.0)
    .lifetime(3.seconds());
    Ok(Study::new(base(parameters).emitter(emitter)))
}

fn source_image() -> motion::Result<Arc<ImageAsset>> {
    let mut pixels = Vec::with_capacity((IMAGE_SIDE * IMAGE_SIDE * 4) as usize);
    for row in 0..IMAGE_SIDE {
        for column in 0..IMAGE_SIDE {
            let horizontal = (f64::from(column) + 0.5) / f64::from(IMAGE_SIDE);
            let vertical = (f64::from(row) + 0.5) / f64::from(IMAGE_SIDE);
            let radius = math::hypot(horizontal * 2.0 - 1.0, vertical * 2.0 - 1.0);
            pixels.extend_from_slice(&[
                (horizontal * 255.0) as u8,
                (vertical * 200.0) as u8,
                ((1.0 - horizontal) * 255.0) as u8,
                if radius < 0.92 { 255 } else { 0 },
            ]);
        }
    }
    Ok(Arc::new(ImageAsset::from_rgba8(
        IMAGE_SIDE, IMAGE_SIDE, pixels,
    )?))
}

pub(super) fn text_study(parameters: DemoParameters) -> motion::Result<Study> {
    Ok(Study {
        source: Specimen::Text(parameters),
        guides: Node::empty(),
    })
}

pub(super) fn text(
    context: &SceneContext<'_>,
    parameters: DemoParameters,
) -> Result<ParticleSimulation, ValidationError> {
    let draw = DrawContext::default().with_assets(context.assets());
    let text = Text::new("motion")
        .family("")
        .size(76.0)
        .fill(Color::from_hex(0x54dfb2));
    base(parameters)
        .emitter(
            SurfaceEmitter::from_text(&text, &draw)?
                .rate(f64::from(parameters.count) * 4.0)
                .speed(4.0)
                .lifetime(3.seconds()),
        )
        .build()
}

pub(super) fn ballistic(parameters: DemoParameters) -> motion::Result<Study> {
    Ok(Study::trajectory(
        ParticleTrajectory::new(
            Ballistic::burst(Point::new(256.0, 116.0), u64::from(parameters.count) * 2)
                .seed(parameters.seed)
                .speed(20.0, 52.0)
                .gravity(Vector2::new(0.0, 20.0))
                .drag(0.2)
                .lifetime(SOURCE_SECONDS.seconds()),
        )
        .style(ParticleStyle::GlowDots { radius: 2.2 }),
    ))
}

pub(super) fn converge(parameters: DemoParameters) -> motion::Result<Study> {
    let count = parameters.count as usize * 2;
    let from = (0..count)
        .map(|index| {
            Point::new(
                76.0 + (index % 16) as f64 * 24.0,
                44.0 + (index / 16) as f64 * 14.0,
            )
        })
        .collect::<Vec<_>>();
    let to = (0..count)
        .map(|index| {
            let angle = index as f64 / count as f64 * std::f64::consts::TAU;
            Point::new(
                256.0 + math::cos(angle) * 94.0,
                152.0 + math::sin(angle) * 94.0,
            )
        })
        .collect::<Vec<_>>();
    Ok(Study::trajectory(
        ParticleTrajectory::new(Converge::new(&from, &to, SOURCE_SECONDS.seconds())?)
            .style(ParticleStyle::Dots { radius: 2.2 }),
    ))
}

pub(super) fn orbit(parameters: DemoParameters) -> motion::Result<Study> {
    Ok(Study::trajectory(
        ParticleTrajectory::new(
            Orbit::new(VIEWPORT.center(), 96.0, u64::from(parameters.count))
                .seed(parameters.seed)
                .angular_velocity(0.9)
                .lifetime(SOURCE_SECONDS.seconds()),
        )
        .style(ParticleStyle::Trails {
            length: 0.2.seconds(),
            width: 1.8,
        }),
    ))
}

pub(super) fn noise(parameters: DemoParameters) -> motion::Result<Study> {
    Ok(Study::trajectory(
        ParticleTrajectory::new(
            NoiseDrift::new(Point::new(180.0, 152.0), u64::from(parameters.count))
                .seed(parameters.seed)
                .velocity(Vector2::new(28.0, 0.0))
                .noise(54.0, 0.7)
                .lifetime(SOURCE_SECONDS.seconds()),
        )
        .style(ParticleStyle::Dots { radius: 2.4 }),
    ))
}
