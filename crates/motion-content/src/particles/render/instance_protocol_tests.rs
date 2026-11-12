use super::*;
use crate::asset::AssetsExt;
use motion_core::{
    Assets, BlendSpace, Canvas, Color, DrawContext, Drawable, Filter, Pivot, PixelSize, Point,
    Size, Transform, ValidationLimits, Vector2,
};
use motion_core::{NodeModifiers, TimeLiteral};
use motion_render::{Composition, Renderer};

#[derive(Clone, Debug)]
struct ScalarBatch(ParticleBatch);

impl motion_core::instances::InstanceBatch for ScalarBatch {
    fn len(&self) -> usize {
        self.0.len()
    }

    fn bounds(&self) -> Rect {
        self.0.bounds()
    }

    fn bytes(&self) -> u64 {
        self.0.bytes()
    }

    fn work(&self) -> u64 {
        self.0.work()
    }

    fn validate(&self) -> Result<(), ValidationError> {
        self.0.validate()
    }

    fn blend(&self) -> motion_core::instances::InstanceBlend {
        self.0.blend()
    }

    fn sample(
        &self,
        point: Point,
        antialias: f64,
        destination: [f32; 4],
        space: BlendSpace,
        composite: motion_core::instances::InstanceComposite,
    ) -> Result<[f32; 4], ValidationError> {
        self.0
            .sample_with_zero_policy::<true>(point, antialias, destination, space, composite)
    }

    fn as_any(&self) -> &dyn std::any::Any {
        self
    }

    fn content_eq(&self, other: &dyn motion_core::instances::InstanceBatch) -> bool {
        other
            .as_any()
            .downcast_ref::<Self>()
            .is_some_and(|other| self.0 == other.0)
    }
}

impl Drawable for ScalarBatch {
    fn bounds(&self, context: &DrawContext) -> Result<Rect, ValidationError> {
        Drawable::bounds(&self.0, context)
    }

    fn draw(&self, _: &DrawContext, canvas: &mut Canvas<'_>) -> Result<(), ValidationError> {
        canvas.instances(Arc::new(self.clone()), Transform::IDENTITY)
    }
}

fn compare_batches(style: ParticleStyle, renderer: Renderer) -> motion_core::Result {
    let count = if matches!(
        &style,
        ParticleStyle::Trails { .. } | ParticleStyle::Plexus { .. }
    ) {
        6
    } else {
        24
    };
    let source = ParticleTrajectory::new(
        Ballistic::burst(Point::new(96.0, 72.0), count)
            .seed(42)
            .speed(20.0, 40.0)
            .gravity(Vector2::new(0.0, 20.0)),
    )
    .style(style);
    let mut session = Composition::new(
        192,
        144,
        motion_render::FrameClock::new(motion_core::fps!(30), 120),
    )
    .with_options(
        motion_render::RenderOptions::default()
            .backend(renderer.into())
            .background(Color::BLACK),
    )?
    .frame_renderer(Size::new(192.0, 144.0))?;
    for mode in 0..3 {
        let mut reference = None;
        let mut moving = None;
        for index in [30, 0, 45, 75, 30] {
            let make = |context: &SceneContext<'_>, expanded: bool| {
                let node = source.try_render(context).unwrap();
                let list = node
                    .compile(
                        context.size(),
                        PixelSize::new(192, 144),
                        ValidationLimits::default(),
                    )
                    .unwrap();
                let mut nodes = Vec::new();
                for command in &list.commands {
                    if let motion_core::DrawCommand::ParticleInstances { batch, .. } = command {
                        let mut batch = batch
                            .as_any()
                            .downcast_ref::<ParticleBatch>()
                            .unwrap()
                            .clone();
                        // This fixture isolates generic primitive expansion, not trajectory evaluation.
                        batch.analytic = None;
                        if expanded {
                            for instance in batch.instances.slice().iter() {
                                let mut single = batch.clone();
                                single.instances = vec![instance.clone()].into();
                                nodes.push(Node::from(ScalarBatch(single)));
                            }
                        } else {
                            nodes.push(Node::from(batch.clone()));
                        }
                    }
                }
                let node = Node::group(nodes);
                match mode {
                    1 => node
                        .pivot(Pivot::Point(Point::new(96.0, 72.0)))
                        .scale_xy(-0.85, 1.1)
                        .rotate(0.13)
                        .opacity(0.6),
                    2 => node
                        .clip_with(
                            Rect::new(20.0, 20.0, 160.0, 130.0),
                            motion_core::ClipMode::Geometric,
                        )
                        .filter(Filter::Blur { sigma: 1.0 }),
                    _ => node,
                }
            };
            let batch = session.render_frame(index, |context| make(context, false))?;
            let expanded = session.render_frame(index, |context| make(context, true))?;
            assert_eq!(
                batch.packed_rgb(),
                expanded.packed_rgb(),
                "batch/expanded {renderer:?}/{mode}/{index}"
            );
            if index == 30 {
                assert!(batch.packed_rgb().iter().any(|pixel| *pixel != 0));
                if let Some(first) = &reference {
                    assert_eq!(batch.packed_rgb(), *first);
                }
                reference = Some(batch.packed_rgb());
            }
            if index == 45 {
                moving = Some(batch.packed_rgb());
            }
        }
        assert_ne!(reference, moving);
    }
    Ok(())
}

#[test]
fn particle_batches_match_expanded_instances_with_effects_and_seek() -> motion_core::Result {
    compare_batches(ParticleStyle::Dots { radius: 3.0 }, Renderer::Cpu)
}

#[test]
fn tiled_glow_lines_and_plexus_match_the_scalar_oracle() -> motion_core::Result {
    for style in [
        ParticleStyle::GlowDots { radius: 3.0 },
        ParticleStyle::Trails {
            width: 2.0,
            length: 100.milliseconds(),
        },
        ParticleStyle::Plexus {
            radius: 2.0,
            max_distance: 12.0,
        },
    ] {
        compare_batches(style, Renderer::Cpu)?;
    }
    Ok(())
}

#[test]
fn transparent_sprite_batches_match_expanded_instances_and_replay() -> motion_core::Result {
    let directory = tempfile::tempdir()?;
    let pixels = [
        255, 64, 16, 128, 10, 200, 40, 255, 255, 0, 255, 0, 40, 80, 220, 64, 255, 255, 255, 192, 0,
        0, 0, 0,
    ];
    image::save_buffer(
        directory.path().join("sprite.png"),
        &pixels,
        3,
        2,
        image::ColorType::Rgba8,
    )
    .map_err(crate::image_error)?;
    let assets = Assets::new(directory.path())?;
    compare_batches(
        ParticleStyle::Sprites {
            image: assets.image("sprite.png"),
            size: 20.0,
            align_to_velocity: false,
        },
        Renderer::Cpu,
    )
}

fn blend_source(style: ParticleStyle) -> ParticleSimulation {
    ParticleSimulation::new(3)
        .duration(1.seconds())
        .emitter(
            BurstEmitter::new(Time::ZERO, 24, Point::new(48.0, 36.0))
                .speed(10.0)
                .color([0.5, 0.1, 0.05, 0.6]),
        )
        .style(style)
        .build()
        .unwrap()
}

#[test]
fn per_style_blend_reaches_instance_upload_and_cpu_pixels() -> motion_core::Result {
    const BLENDS: [ParticleBlend; 4] = [
        ParticleBlend::SourceOver,
        ParticleBlend::Plus,
        ParticleBlend::Screen,
        ParticleBlend::Multiply,
    ];
    let cache = motion_core::SceneCache::default();
    let context = SceneContext::new(
        300.milliseconds().after_origin(),
        motion_core::fps!(30),
        Size::new(96.0, 72.0),
        &cache,
    )
    .with_clip_duration(1.seconds());
    let styles = [
        ParticleStyle::Dots { radius: 4.0 },
        ParticleStyle::Shape {
            radius: 4.0,
            shape: ParticleShape::Ring { thickness: 0.5 },
        },
        ParticleStyle::Trails {
            length: 100.milliseconds(),
            width: 3.0,
        },
    ];
    for style in styles {
        for blend in BLENDS {
            let node = blend_source(style.clone().blend(blend)).try_render(&context)?;
            let list = node.compile(
                context.size(),
                PixelSize::new(96, 72),
                ValidationLimits::default(),
            )?;
            let mut seen = 0;
            for command in &list.commands {
                if let motion_core::DrawCommand::ParticleInstances { batch, .. } = command {
                    assert_eq!(batch.blend(), blend);
                    let words = batch.upload()?.expect("device payload").into_words();
                    assert_eq!(words[3] as u32 >> 5, blend as u32);
                    seen += 1;
                }
            }
            assert_eq!(seen, 1);
        }
    }
    let mut session = Composition::new(
        96,
        72,
        motion_render::FrameClock::new(motion_core::fps!(30), 30),
    )
    .with_options(
        motion_render::RenderOptions::default()
            .backend(motion_render::BackendPreference::Cpu)
            .background(Color::rgb(96, 96, 96)),
    )?
    .frame_renderer(Size::new(96.0, 72.0))?;
    let frames = BLENDS
        .into_iter()
        .map(|blend| {
            let source = blend_source(ParticleStyle::Dots { radius: 4.0 }.blend(blend));
            session
                .render_frame(9, |context| source.try_render(context).unwrap())
                .map(|frame| frame.packed_rgb())
        })
        .collect::<Result<Vec<_>, _>>()?;
    for first in 0..frames.len() {
        for second in first + 1..frames.len() {
            assert_ne!(frames[first], frames[second], "{first} vs {second}");
        }
    }
    Ok(())
}
