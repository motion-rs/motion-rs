//! Criterion benchmarks for representative single-frame renders.

use criterion::{Criterion, Throughput, criterion_group, criterion_main};
use motion::particles::{Orbit, ParticleStyle, ParticleTrajectory};
use motion::prelude::*;
use motion::render::{Composition, RasterLimits, RenderContext, ValidationLimits};
use std::{hint::black_box, time::Duration};

fn measure_frame(
    criterion: &mut Criterion,
    name: &str,
    size: (usize, usize),
    resources: &Assets,
    limits: RasterLimits,
    build: impl Fn(&SceneContext<'_>) -> Node,
) {
    let context = RenderContext::new()
        .with_validation_limits(ValidationLimits {
            max_nodes: 100_016,
            ..ValidationLimits::default()
        })
        .expect("bounded benchmark structure");
    let mut session = Composition::new(size.0, size.1, motion::render::FrameClock::new(30, 150))
        .with_options(
            motion::render::RenderOptions::default()
                .context(context)
                .resources(resources.clone())
                .limits(limits),
        )
        .expect("bounded benchmark raster policy")
        .frame_renderer(Size::new(size.0 as f64, size.1 as f64))
        .expect("valid CPU benchmark session");
    let expected = session
        .render_frame(30, &build)
        .expect("benchmark admission");
    let replay = session.render_frame(30, &build).expect("benchmark replay");
    assert_eq!(expected.pixels(), replay.pixels());
    assert!(expected.pixels().iter().any(|pixel| *pixel != 0xffff_ffff));
    let mut group = criterion.benchmark_group("cpu_warm_frame");
    group.throughput(Throughput::Elements(1));
    group.bench_function(name, |bencher| {
        bencher.iter(|| {
            black_box(
                session
                    .render_frame(black_box(30), &build)
                    .expect("benchmark frame"),
            );
        });
    });
    group.finish();
}

fn frame_benches(criterion: &mut Criterion) {
    let mut fonts = FontLibrary::new();
    fonts
        .load_file(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../../tests/fixtures/fonts/NotoSans-Regular.ttf"
        ))
        .expect("bundled Noto Sans font");
    let resources = Assets::new(".")
        .expect("benchmark asset root")
        .with_fonts(fonts);
    let shapes = Node::group((0..10_000).map(|index| {
        let left = f64::from(index % 100) * 6.4;
        let top = f64::from(index / 100) * 3.6;
        Node::from(Rect::new(left, top, left + 4.0, top + 2.0).fill(Color::BLACK))
    }));
    measure_frame(
        criterion,
        "10000_shapes_640x360",
        (640, 360),
        &resources,
        RasterLimits::default(),
        |_| shapes.clone(),
    );

    let particles = [17, 29].map(|seed| {
        ParticleTrajectory::new(Orbit::new(Point::new(320.0, 180.0), 120.0, 50_000).seed(seed))
            .style(ParticleStyle::Dots { radius: 0.75 })
    });
    measure_frame(
        criterion,
        "100000_particles_two_batches_640x360",
        (640, 360),
        &resources,
        RasterLimits {
            max_work: 4_000_000_000,
            ..RasterLimits::default()
        },
        |context| {
            Node::group(
                particles
                    .iter()
                    .map(|particles| particles.try_render(context).expect("particle batch")),
            )
        },
    );

    let text = Node::from(
        Text::new("Motion keeps geometry, time and typography in agreement. ".repeat(32))
            .size(18.0)
            .max_width(600.0)
            .fill(Color::BLACK),
    );
    measure_frame(
        criterion,
        "long_text_640x360",
        (640, 360),
        &resources,
        RasterLimits {
            max_work: 4_000_000_000,
            ..RasterLimits::default()
        },
        |_| text.clone(),
    );

    let gradients = Node::group((0..32).map(|index| {
        let left = f64::from(index % 8) * 80.0;
        let top = f64::from(index / 8) * 90.0;
        Node::from(
            Rect::new(left, top, left + 78.0, top + 88.0).fill(
                LinearGradient::new(Point::new(0.0, 0.0), Point::new(1.0, 1.0))
                    .stop(0.0, Color::rgb(20, 130, 110))
                    .stop(1.0, Color::rgb(240, 120, 98)),
            ),
        )
    }));
    measure_frame(
        criterion,
        "32_gradients_640x360",
        (640, 360),
        &resources,
        RasterLimits::default(),
        |_| gradients.clone(),
    );

    let layers = Node::group((0..200).map(|index| {
        let left = f64::from(index % 20) * 32.0;
        let top = f64::from(index / 20) * 36.0;
        Node::group([
            Node::from(Rect::new(left, top, left + 24.0, top + 24.0).fill(Color::BLACK)),
            Node::from(
                Circle::new(Point::new(left + 20.0, top + 20.0), 8.0)
                    .unwrap()
                    .fill(Color::WHITE),
            ),
        ])
        .opacity(0.6)
    }));
    measure_frame(
        criterion,
        "200_isolated_layers_640x360",
        (640, 360),
        &resources,
        RasterLimits::default(),
        |_| layers.clone(),
    );

    let blurred = Node::group([Node::from(
        Circle::new(Point::new(320.0, 180.0), 100.0)
            .unwrap()
            .fill(Color::BLACK),
    )])
    .filter(Filter::Blur { sigma: 12.0 });
    measure_frame(
        criterion,
        "blur_sigma_12_640x360",
        (640, 360),
        &resources,
        RasterLimits::default(),
        |_| blurred.clone(),
    );

    let tiles = Node::group((0..64).map(|index| {
        let left = f64::from(index % 8) * 480.0;
        let top = f64::from(index / 8) * 270.0;
        Node::from(Rect::new(left, top, left + 480.0, top + 270.0).fill(Color::rgb(20, 130, 110)))
    }));
    measure_frame(
        criterion,
        "4k_tiles_3840x2160",
        (3840, 2160),
        &resources,
        RasterLimits::default(),
        |_| tiles.clone(),
    );
}

criterion_group! {
    name = benches;
    config = Criterion::default().sample_size(10)
        .warm_up_time(Duration::from_secs(1))
        .measurement_time(Duration::from_secs(3));
    targets = frame_benches
}

criterion_main!(benches);
