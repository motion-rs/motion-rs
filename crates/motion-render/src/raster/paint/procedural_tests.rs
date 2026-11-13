use super::*;
use motion_core::{
    AtmosphereKind, AtmospherePaint, AtmosphereParameters, Color, MeshGradient, MeshPaint,
    MeshVertex, NodeModifiers, PixelSize, Shape, Size, TimeLiteral, ValidationLimits,
};

fn atmosphere(kind: AtmosphereKind, time: f64) -> AtmospherePaint {
    AtmospherePaint::new(
        AtmosphereParameters {
            kind,
            seed: 0xfedc_ba98,
            color: Color::from_rgb_float(1.0, 0.4, 0.15).with_alpha(0.65),
            opacity: 0.7,
            intensity: 3.0,
            scale: 12.0,
            drift: [2.0, -3.0],
            fractal: motion_core::noise::Fractal::default(),
        },
        PixelSize::new(48, 32),
        motion_core::Time::try_from_seconds(time).unwrap(),
    )
    .unwrap()
}

fn paints() -> Vec<Paint> {
    let mesh = MeshGradient::new(
        2,
        2,
        vec![
            MeshVertex {
                position: Point::new(0.0, 0.0),
                color: Color::from_hex(0xff0000).with_alpha(0.2),
            },
            MeshVertex {
                position: Point::new(1.0, 0.1),
                color: Color::from_hex(0x00ff00).with_alpha(0.8),
            },
            MeshVertex {
                position: Point::new(0.15, 1.0),
                color: Color::from_hex(0x0000ff).with_alpha(0.4),
            },
            MeshVertex {
                position: Point::new(0.95, 0.9),
                color: Color::WHITE,
            },
        ],
    )
    .unwrap();
    let mut paints = vec![Paint::Mesh(MeshPaint::new(mesh, PixelSize::new(1, 1), 1))];
    paints.push(Paint::Mesh(MeshPaint::new(
        MeshGradient::grid(
            3,
            3,
            &[
                Color::from_hex(0xe03020),
                Color::from_hex(0x20dd40),
                Color::from_hex(0x2060ee),
                Color::WHITE.with_alpha(0.1),
                Color::from_hex(0xe03020),
                Color::WHITE.with_alpha(0.2),
                Color::from_hex(0x2060ee),
                Color::from_hex(0x20dd40),
                Color::from_hex(0xe03020),
            ],
        )
        .unwrap(),
        PixelSize::new(1, 1),
        1,
    )));
    for kind in [
        AtmosphereKind::Fog {
            gradient: [0.1, 2.0],
        },
        AtmosphereKind::Dust {
            radius: 3.0,
            coverage: 0.65,
        },
        AtmosphereKind::LightLeak {
            center: [0.0, 0.5],
            extent: [0.4, 0.7],
        },
    ] {
        for time in [-0.75, 0.0, 1.25] {
            paints.push(atmosphere(kind, time).into());
        }
    }
    paints
}

#[test]
fn procedural_paints_cpu_samples_rows_and_admission() -> motion_core::Result {
    let limits = RasterLimits::default();
    for paint in paints() {
        let node = motion_core::Node::from(Rect::new(0.0, 0.0, 48.0, 32.0).fill(paint.clone()));
        let validated = node.compile(
            Size::new(48.0, 32.0),
            PixelSize::new(48, 32),
            ValidationLimits::default(),
        )?;
        let prepared = validated.prepare(ValidationLimits::default(), limits)?;
        assert!(!matches!(
            prepared.paint_sources[0].as_ref().unwrap().paint,
            Paint::Pattern(_)
        ));
        let source = prepared.paint_sources[0].as_ref().unwrap();
        match &paint {
            Paint::Mesh(mesh) => {
                assert_eq!(
                    source.mesh_patches.len(),
                    ((mesh.mesh.rows() - 1) * (mesh.mesh.columns() - 1)) as usize
                );
                assert_eq!(
                    source.work_per_pixel(),
                    source.mesh_patches.len() as u64 * 512
                );
            }
            Paint::Atmosphere(atmosphere) => {
                assert_eq!(source.work_per_pixel(), atmosphere.work_per_pixel())
            }
            _ => panic!("Fixture paints are procedural"),
        }
        for row in [0, 13, 31] {
            let mut sampling = prepared.paint_row(0, 0, row);
            for column in 0..48 {
                let point = Point::new(f64::from(column) + 0.5, f64::from(row) + 0.5);
                let value = prepared.paint_at(0, point)?;
                assert_eq!(value, sampling.sample(column)?);
                if let Paint::Atmosphere(atmosphere) = &paint {
                    let expected = atmosphere.sample(Point::new(point.x / 48.0, point.y / 32.0))?;
                    assert_eq!(value, expected);
                }
            }
        }
        let work = prepared.compose_linear(limits)?.pixels().len();
        assert_eq!(work, 48 * 32);
        let tiny = RasterLimits {
            max_work: 1,
            ..limits
        };
        assert!(
            validated
                .prepare(ValidationLimits::default(), tiny)
                .is_err()
        );
        let tiny = RasterLimits {
            max_bytes: 1,
            ..limits
        };
        assert!(
            validated
                .prepare(ValidationLimits::default(), tiny)
                .is_err()
        );
    }
    Ok(())
}

#[test]
fn analytic_paints_do_not_allocate_bake_resolution_or_atmospheric_canvas() -> motion_core::Result {
    let mesh = MeshGradient::grid(2, 2, &[Color::WHITE; 4])?;
    let mesh = Paint::Mesh(MeshPaint::new(mesh, PixelSize::new(u32::MAX, u32::MAX), 32));
    let atmospheric = AtmospherePaint::new(
        *atmosphere(
            AtmosphereKind::Fog {
                gradient: [0.0, 1.0],
            },
            0.0,
        )
        .parameters(),
        PixelSize::new(16384, 16384),
        0.seconds().after_origin(),
    )?;
    for paint in [mesh, atmospheric.into()] {
        let node = motion_core::Node::from(Rect::new(0.0, 0.0, 8.0, 8.0).fill(paint));
        let prepared = node
            .compile(
                Size::new(8.0, 8.0),
                PixelSize::new(8, 8),
                ValidationLimits::default(),
            )?
            .prepare(ValidationLimits::default(), RasterLimits::default())?;
        assert!(prepared.compose_linear(RasterLimits::default()).is_ok());
    }
    Ok(())
}

#[test]
fn procedural_paints_reject_invisible_invalid_recipes_and_working_overflow() {
    let mesh = MeshGradient::new(
        2,
        2,
        vec![
            MeshVertex {
                position: Point::new(1e100, 0.0),
                color: Color::WHITE
            };
            4
        ],
    )
    .unwrap();
    let node = motion_core::Node::from(
        Rect::new(0.0, 0.0, 8.0, 8.0).fill(Paint::Mesh(MeshPaint::new(
            mesh,
            PixelSize::new(1, 1),
            1,
        ))),
    )
    .opacity(0.0);
    assert!(
        node.compile(
            Size::new(8.0, 8.0),
            PixelSize::new(8, 8),
            ValidationLimits::default()
        )
        .is_err()
    );
    let mut recipe = *atmosphere(
        AtmosphereKind::Fog {
            gradient: [0.0, 1.0],
        },
        0.0,
    )
    .parameters();
    recipe.drift = [f64::MAX, 0.0];
    assert!(
        AtmospherePaint::new(recipe, PixelSize::new(8, 8), 2.seconds().after_origin()).is_err()
    );
    recipe.drift = [0.0; 2];
    recipe.scale = f64::NAN;
    assert!(
        AtmospherePaint::new(recipe, PixelSize::new(8, 8), 0.seconds().after_origin()).is_err()
    );
}

#[test]
#[cfg(feature = "gpu")]
#[ignore = "requires native GPU; independent procedural evaluation, no timing"]
fn independent_procedural_paints_cpu_gpu_visual_and_replay() -> motion_core::Result {
    let limits = RasterLimits::default();
    let mut gpu = crate::gpu::Gpu::new()?;
    let mut maximum = 0.0f32;
    for paint in paints() {
        for transform in [
            Transform::IDENTITY,
            Transform::from_coefficients([0.8, 0.125, -0.125, 0.8, 5.0, 1.0]),
        ] {
            for filtered in [false, true] {
                let mut node =
                    motion_core::Node::from(Rect::new(0.0, 0.0, 48.0, 32.0).fill(paint.clone()))
                        .transformed(transform);
                if filtered {
                    node = motion_core::Node::group([
                        motion_core::Node::from(
                            Rect::new(0.0, 0.0, 48.0, 32.0).fill(Color::rgb(15, 35, 55)),
                        ),
                        node.filter(motion_core::Filter::Blur { sigma: 0.6 })
                            .clip_with(
                                Rect::new(3.0, 2.0, 45.0, 30.0),
                                motion_core::ClipMode::Geometric,
                            )
                            .opacity(0.65),
                    ]);
                }
                let validated = node.compile(
                    Size::new(48.0, 32.0),
                    PixelSize::new(48, 32),
                    ValidationLimits::default(),
                )?;
                let prepared = validated.prepare(ValidationLimits::default(), limits)?;
                let device = crate::gpu::raster::prepare::prepare_validated_device(
                    &validated,
                    ValidationLimits::default(),
                    limits,
                    1.0 / 256.0,
                    None,
                )?;
                assert!(
                    device
                        .records
                        .paint_sources
                        .iter()
                        .flatten()
                        .all(|source| !matches!(source.paint, Paint::Pattern(_)))
                );
                let cpu = prepared.compose_linear(limits)?;
                let actual = gpu.render(&device.records, crate::gpu::Plan::new(&device, limits)?)?;
                let replay = gpu.render(&device.records, crate::gpu::Plan::new(&device, limits)?)?;
                for ((cpu, actual), replay) in cpu
                    .pixels()
                    .iter()
                    .zip(actual.pixels())
                    .zip(replay.pixels())
                {
                    assert_eq!(actual, replay);
                    for (first, second) in cpu.iter().zip(actual) {
                        maximum = maximum.max((first - second).abs());
                        assert!(
                            (first - second).abs() <= 2e-4,
                            "{paint:?}: {first} != {second}"
                        );
                    }
                }
                let cpu = cpu.to_rgba8_frame(false, limits)?;
                let actual = actual.to_rgba8_frame(false, limits)?;
                for (cpu, actual) in cpu.pixels().iter().zip(actual.pixels()) {
                    for (first, second) in cpu.iter().zip(actual) {
                        assert!(first.abs_diff(*second) <= 1);
                    }
                }
            }
        }
    }
    eprintln!(
        "independent_procedural_paints max_linear_difference={maximum} rgba_tolerance=1 replay=exact"
    );
    Ok(())
}
