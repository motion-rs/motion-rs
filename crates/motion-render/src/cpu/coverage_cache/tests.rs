use super::*;
use crate::cpu::polygon::scratch::MAX_IDLE_SCRATCH_BYTES;
use motion_core::{
    Color, DisplayList, DrawCommand, Node, NodeModifiers, Paint, PixelSize, Shape, Size,
    ValidationError, ValidationLimits,
};

#[test]
fn polygon_area_scratch_reuses_cold_frames_without_changing_pixels_or_admission() {
    let size = PixelSize::new(96, 96);
    let geometry = Arc::new(
        Path2D::parse_svg("M12 12Q18 5 28 14C35 19 29 29 22 31L12 25Z M17 17L17 23L23 22Z")
            .unwrap(),
    );
    let mut cache = CoverageCache::default();
    let mut allocation = std::ptr::null();
    for (index, rule) in [FillRule::NonZero, FillRule::EvenOdd, FillRule::NonZero]
        .into_iter()
        .enumerate()
    {
        cache.shared.clear();
        let source = DisplayList {
            output_size: size,
            background: None,
            commands: vec![DrawCommand::Fill {
                path: geometry.clone(),
                transform: Transform::translate(index as f64 * 0.25, 0.0),
                paint: Color::WHITE.into(),
                rule,
            }],
        };
        let expected = source
            .prepare(ValidationLimits::default(), RasterLimits::default())
            .unwrap();
        let exact = RasterLimits {
            max_bytes: expected.stats().peak_bytes,
            max_work: expected.stats().work,
            ..RasterLimits::default()
        };
        compare(
            &cached(&source, &mut cache, exact, 1.0 / 256.0).unwrap(),
            &expected,
        );
        assert!(cache.polygon_scratch.areas.is_empty());
        assert!(cache.polygon_scratch.areas.capacity() > 0);
        if index == 0 {
            allocation = cache.polygon_scratch.areas.as_ptr();
        } else {
            assert_eq!(cache.polygon_scratch.areas.as_ptr(), allocation);
        }
        for limits in [
            RasterLimits {
                max_bytes: exact.max_bytes - 1,
                ..exact
            },
            RasterLimits {
                max_work: exact.max_work - 1,
                ..exact
            },
        ] {
            cache.shared.clear();
            let actual = cached(&source, &mut cache, limits, 1.0 / 256.0).unwrap_err();
            let fresh = source
                .prepare(ValidationLimits::default(), limits)
                .unwrap_err();
            assert_eq!(
                (actual.kind, actual.field, actual.path),
                (fresh.kind, fresh.field, fresh.path)
            );
            assert!(cache.polygon_scratch.areas.is_empty());
            cache.shared.clear();
            compare(
                &cached(&source, &mut cache, exact, 1.0 / 256.0).unwrap(),
                &expected,
            );
        }
        cache
            .polygon_scratch
            .areas
            .resize(cache.polygon_scratch.areas.capacity(), f64::NAN);
    }
    assert!(CoverageCache::default().polygon_scratch.areas.capacity() == 0);
}

#[test]
fn polygon_area_scratch_discards_oversized_idle_storage_even_after_failure() {
    let source = list(PixelSize::new(16, 16), Color::WHITE);
    let mut cache = CoverageCache::default();
    for limits in [
        RasterLimits::default(),
        RasterLimits {
            max_work: 1,
            ..RasterLimits::default()
        },
    ] {
        cache.polygon_scratch.areas = vec![f64::NAN; MAX_IDLE_SCRATCH_BYTES / size_of::<f64>() + 1];
        let mut frame = source
            .prepare(ValidationLimits::default(), RasterLimits::default())
            .unwrap();
        let mut budget = Budget::new(source.output_size, limits).unwrap();
        let geometry = Path2D::parse_svg("M1 1L14 2L4 14Z").unwrap();
        let result = cache.rasterize_path(
            &mut frame,
            &geometry,
            Transform::IDENTITY,
            FillRule::NonZero,
            1.0 / 256.0,
            &mut RasterScope::new(&mut budget, "test"),
        );
        assert_eq!(result.is_ok(), limits.max_work != 1);
        assert_eq!(cache.polygon_scratch.areas.capacity(), 0);
    }
}

#[cfg(feature = "gpu")]
#[test]
#[ignore = "requires native GPU; checks translated coverage without timing"]
fn translated_curve_bands_match_gpu_pixels_and_exact_replay() -> motion_core::Result {
    let size = PixelSize::new(96, 96);
    let limits = RasterLimits::default();
    let path = Arc::new(Path2D::parse_svg(
        "M12 12Q18 5 28 14C35 19 29 29 22 31L12 25Z M17 17L17 23L23 22Z",
    )?);
    let mut cache = CoverageCache::default();
    let mut gpu = crate::gpu::Gpu::new()?;
    for (dx, dy) in [(0.0, 0.0), (1.0, 1.0), (2.0, 3.0), (7.0, 5.0), (1.0, 1.0)] {
        let source = DisplayList {
            output_size: size,
            background: None,
            commands: vec![DrawCommand::Fill {
                path: path.clone(),
                transform: Transform::translate(dx, dy),
                paint: Color::WHITE.into(),
                rule: FillRule::EvenOdd,
            }],
        };
        let prepared = cached(&source, &mut cache, limits, 1.0 / 256.0)?;
        let cpu = prepared
            .compose_linear(limits)?
            .to_rgba8_frame(false, limits)?;
        let native = gpu
            .render(&prepared, crate::gpu::Plan::new(&prepared, limits)?)?
            .to_rgba8_frame(false, limits)?;
        let replay = gpu
            .render(&prepared, crate::gpu::Plan::new(&prepared, limits)?)?
            .to_rgba8_frame(false, limits)?;
        assert_eq!(native.bytes(), replay.bytes());
        let mut beyond_one = 0_usize;
        for (expected, actual) in cpu.pixels().iter().zip(native.pixels()) {
            let delta = expected
                .iter()
                .zip(actual)
                .map(|(left, right)| left.abs_diff(*right))
                .max()
                .unwrap();
            assert!(delta <= 2);
            beyond_one += usize::from(delta > 1);
        }
        assert!(beyond_one * 1000 < cpu.pixels().len());
        assert_eq!(cache.shared.len(), 1);
    }
    Ok(())
}

#[test]
fn curved_multicontour_paths_reuse_all_integer_phases_and_keep_budget_failures() {
    let size = PixelSize::new(96, 96);
    for geometry in [
        "M12 12Q18 5 28 14C35 19 29 29 22 31L12 25Z M17 17L17 23L23 22Z",
        "M12.25 14.5L29.75 12.25L24.5 31.25Z",
    ] {
        for matrix in [
            Transform::IDENTITY,
            Transform::scale(0.5).then(Transform::translate(12.0, 12.0)),
            Transform::scale_non_uniform(-1.0, 1.0).then(Transform::translate(48.0, 0.0)),
        ] {
            let path = Arc::new(Path2D::parse_svg(geometry).unwrap());
            let mut cache = CoverageCache::default();
            for dy in 0..4 {
                for dx in 0..4 {
                    let transform = matrix.then(Transform::translate(f64::from(dx), f64::from(dy)));
                    let source = DisplayList {
                        output_size: size,
                        background: None,
                        commands: vec![DrawCommand::Fill {
                            path: path.clone(),
                            transform,
                            paint: Color::WHITE.into(),
                            rule: FillRule::EvenOdd,
                        }],
                    };
                    let expected = source
                        .prepare(ValidationLimits::default(), RasterLimits::default())
                        .unwrap();
                    compare(
                        &cached(&source, &mut cache, RasterLimits::default(), 1.0 / 256.0).unwrap(),
                        &expected,
                    );
                    assert_eq!(cache.shared.len(), 1, "{geometry} {transform:?}");
                    for limits in [
                        RasterLimits {
                            max_work: expected.stats.work - 1,
                            ..RasterLimits::default()
                        },
                        RasterLimits {
                            max_bytes: expected.stats.peak_bytes - 1,
                            ..RasterLimits::default()
                        },
                        RasterLimits {
                            max_bytes: 1,
                            ..RasterLimits::default()
                        },
                    ] {
                        let actual = cached(&source, &mut cache, limits, 1.0 / 256.0);
                        let fresh = source.prepare(ValidationLimits::default(), limits);
                        match (actual, fresh) {
                            (Ok(actual), Ok(fresh)) => {
                                compare(&actual, &fresh);
                                let mut reference = expected.clone();
                                reference.stats = actual.stats;
                                compare(&actual, &reference);
                                assert!(actual.stats.peak_bytes <= limits.max_bytes);
                                assert!(actual.stats.work <= limits.max_work);
                                assert!(limits.max_bytes > 1);
                            }
                            (Err(actual), Err(fresh)) => assert_eq!(
                                (actual.kind, actual.field, actual.path),
                                (fresh.kind, fresh.field, fresh.path)
                            ),
                            _ => panic!("cache and fresh preparation disagree on admission"),
                        }
                    }
                }
            }
        }
    }
}

#[test]
fn exact_integer_phase_reuse_matches_independent_clipped_and_fractional_rasters() {
    let size = PixelSize::new(64, 64);
    let mut cache = CoverageCache::default();
    let mut source = DisplayList {
        output_size: size,
        background: None,
        commands: vec![DrawCommand::Fill {
            path: Arc::new(
                Path2D::from_rect(motion_core::Rect::new(8.25, 8.5, 24.75, 28.5)).unwrap(),
            ),
            transform: Transform::IDENTITY,
            paint: Paint::Solid(Color::WHITE),
            rule: FillRule::NonZero,
        }],
    };
    let limits = RasterLimits::default();
    let tolerance = 1.0 / 256.0;
    for (index, (dx, dy)) in [
        (0.0, 0.0),
        (4.0, 8.0),
        (12.0, 4.0),
        (1.0, 0.0),
        (4.25, 8.0),
        (-12.0, -12.0),
        (4.0, 8.0),
    ]
    .into_iter()
    .enumerate()
    {
        if let DrawCommand::Fill { transform, .. } = &mut source.commands[0] {
            *transform = Transform::translate(dx, dy);
        }
        let expected = source.prepare(ValidationLimits::default(), limits).unwrap();
        let actual = cached(&source, &mut cache, limits, tolerance).unwrap();
        compare(&actual, &expected);
        if index < 3 {
            assert_eq!(cache.shared.len(), 1);
        }
    }
}

#[test]
fn indexed_lru_exceeds_old_capacity_and_evicts_the_untouched_oldest_entry() {
    let size = PixelSize::new(1, 1);
    let limits = RasterLimits::default();
    let tolerance = 1.0 / 256.0;
    let geometry = Arc::new(Path2D::new());
    let mut cache = CoverageCache::default();
    let mut source = DisplayList {
        output_size: size,
        background: None,
        commands: vec![DrawCommand::Fill {
            path: geometry.clone(),
            transform: Transform::IDENTITY,
            paint: Paint::Solid(Color::WHITE),
            rule: FillRule::NonZero,
        }],
    };
    for index in 0..2048 {
        if let DrawCommand::Fill { transform, .. } = &mut source.commands[0] {
            *transform = Transform::translate(index as f64 / 4096.0, 0.0);
        }
        cached(&source, &mut cache, limits, tolerance).unwrap();
    }
    assert_eq!(cache.shared.len(), 2048);
    let _first = cache
        .lookup(
            &geometry,
            Transform::IDENTITY,
            size,
            FillRule::NonZero,
            tolerance,
        )
        .unwrap();
    if let DrawCommand::Fill { transform, .. } = &mut source.commands[0] {
        *transform = Transform::translate(0.75, 0.0);
    }
    cached(&source, &mut cache, limits, tolerance).unwrap();
    assert_eq!(cache.shared.len(), 2048);
    assert!(
        cache
            .lookup(
                &geometry,
                Transform::IDENTITY,
                size,
                FillRule::NonZero,
                tolerance
            )
            .is_some()
    );
    assert!(
        cache
            .lookup(
                &geometry,
                Transform::translate(1.0 / 4096.0, 0.0),
                size,
                FillRule::NonZero,
                tolerance
            )
            .is_none()
    );
    assert!(cache.retained_bytes() <= 16 * 1024 * 1024);
}

#[test]
fn clipped_raw_translation_cannot_alias_a_safe_normalized_phase() {
    let size = PixelSize::new(64, 64);
    let mut source = DisplayList {
        output_size: size,
        background: None,
        commands: vec![DrawCommand::Fill {
            path: Arc::new(
                Path2D::from_rect(motion_core::Rect::new(-3.75, -3.5, 12.75, 16.5)).unwrap(),
            ),
            transform: Transform::IDENTITY,
            paint: Paint::Solid(Color::WHITE),
            rule: FillRule::NonZero,
        }],
    };
    let mut cache = CoverageCache::default();
    for offset in [8.0, 12.0, 0.0, 8.0, -4.0, 12.0] {
        if let DrawCommand::Fill { transform, .. } = &mut source.commands[0] {
            *transform = Transform::translate(offset, offset);
        }
        let expected = source
            .prepare(ValidationLimits::default(), RasterLimits::default())
            .unwrap();
        let actual = cached(&source, &mut cache, RasterLimits::default(), 1.0 / 256.0).unwrap();
        compare(&actual, &expected);
    }
}

fn list(size: PixelSize, color: Color) -> DisplayList {
    let geometry = Path2D::parse_svg("M2 2C5 0 14 3 13 12L3 14Z M5 5V9H9V5Z").unwrap();
    Node::from(geometry.fill(color))
        .compile(
            Size::new(f64::from(size.width), f64::from(size.height)),
            size,
            ValidationLimits::default(),
        )
        .unwrap()
        .into_editable()
}

fn cached(
    source: &DisplayList,
    cache: &mut CoverageCache,
    limits: RasterLimits,
    tolerance: f64,
) -> Result<PreparedCpuFrame, ValidationError> {
    let mut preparation = crate::raster::PreparationCache::default();
    std::mem::swap(&mut preparation.coverage, cache);
    let prepared = prepare_with_cache(
        source,
        ValidationLimits::default(),
        limits,
        tolerance,
        Some(&mut preparation),
    );
    std::mem::swap(&mut preparation.coverage, cache);
    prepared
}

fn compare(actual: &PreparedCpuFrame, expected: &PreparedCpuFrame) {
    assert_eq!(actual.strips, expected.strips);
    assert_eq!(actual.spans, expected.spans);
    assert_eq!(actual.alphas, expected.alphas);
    assert_eq!(actual.stats(), expected.stats());
    assert_eq!(
        actual
            .compose_linear(RasterLimits::default())
            .unwrap()
            .pixels(),
        expected
            .compose_linear(RasterLimits::default())
            .unwrap()
            .pixels()
    );
}

#[test]
fn coverage_cache_reuses_content_not_paint_and_preserves_admission() {
    let size = PixelSize::new(16, 16);
    let tolerance = 1.0 / 256.0;
    let source = list(size, Color::BLACK);
    let reference = source
        .prepare(ValidationLimits::default(), RasterLimits::default())
        .unwrap();
    let mut cache = CoverageCache::default();
    compare(
        &cached(&source, &mut cache, RasterLimits::default(), tolerance).unwrap(),
        &reference,
    );
    assert_eq!(cache.shared.len(), 1);
    let changed = list(size, Color::rgb(20, 180, 90));
    let expected = changed
        .prepare(ValidationLimits::default(), RasterLimits::default())
        .unwrap();
    compare(
        &cached(&changed, &mut cache, RasterLimits::default(), tolerance).unwrap(),
        &expected,
    );
    assert_eq!(cache.shared.len(), 1);
    let exact = RasterLimits {
        max_pixels: 256,
        max_bytes: reference.stats().peak_bytes,
        max_work: reference.stats().work,
    };
    compare(
        &cached(&source, &mut cache, exact, tolerance).unwrap(),
        &reference,
    );
    for limits in [
        RasterLimits {
            max_bytes: exact.max_bytes - 1,
            ..exact
        },
        RasterLimits {
            max_work: exact.max_work - 1,
            ..exact
        },
        RasterLimits {
            max_pixels: 255,
            ..exact
        },
    ] {
        let expected = source
            .prepare(ValidationLimits::default(), limits)
            .unwrap_err();
        let actual = cached(&source, &mut cache, limits, tolerance).unwrap_err();
        assert_eq!(
            (actual.kind, actual.field, actual.path),
            (expected.kind, expected.field, expected.path)
        );
    }
    compare(
        &cached(&source, &mut cache, exact, tolerance).unwrap(),
        &reference,
    );
}

#[test]
fn coverage_cache_keys_collisions_validation_and_eviction_are_bounded() {
    let mut cache = CoverageCache::default();
    let size = PixelSize::new(16, 16);
    let tolerance = 1.0 / 256.0;
    let source = list(size, Color::BLACK);
    cached(&source, &mut cache, RasterLimits::default(), tolerance).unwrap();
    for variant in 0..4 {
        let mut changed = list(
            if variant == 2 {
                PixelSize::new(17, 16)
            } else {
                size
            },
            Color::BLACK,
        );
        if let DrawCommand::Fill {
            transform, rule, ..
        } = &mut changed.commands[0]
        {
            if variant == 0 {
                *transform = Transform::translate(0.25, 0.0);
            }
            if variant == 1 {
                *rule = FillRule::EvenOdd;
            }
        }
        let precision = if variant == 3 {
            tolerance * 2.0
        } else {
            tolerance
        };
        let expected = changed
            .prepare_with_tolerance(
                ValidationLimits::default(),
                RasterLimits::default(),
                precision,
            )
            .unwrap();
        compare(
            &cached(&changed, &mut cache, RasterLimits::default(), precision).unwrap(),
            &expected,
        );
    }
    assert_eq!(cache.shared.len(), 5);
    let mut collision = list(size, Color::BLACK);
    if let DrawCommand::Fill { path, .. } = &mut collision.commands[0] {
        *path = std::sync::Arc::new(Path2D::parse_svg("M1 1L12 3L6 13Z").unwrap());
        let original = list(size, Color::BLACK);
        let DrawCommand::Fill {
            path: original_path,
            ..
        } = &original.commands[0]
        else {
            unreachable!()
        };
        let mut entry = cache
            .lookup(
                original_path,
                Transform::IDENTITY,
                size,
                FillRule::NonZero,
                tolerance,
            )
            .unwrap()
            .0
            .as_ref()
            .clone();
        entry.hash = path.fingerprint();
        cache.insert(entry);
    }
    let expected = collision
        .prepare(ValidationLimits::default(), RasterLimits::default())
        .unwrap();
    compare(
        &cached(&collision, &mut cache, RasterLimits::default(), tolerance).unwrap(),
        &expected,
    );
    assert_eq!(cache.shared.len(), 7);
    let before = cache.retained_bytes();
    let mut invalid = list(size, Color::BLACK);
    if let DrawCommand::Fill { transform, .. } = &mut invalid.commands[0] {
        *transform = Transform::from_coefficients([f64::NAN, 0.0, 0.0, 1.0, 0.0, 0.0]);
    }
    assert!(cached(&invalid, &mut cache, RasterLimits::default(), tolerance).is_err());
    assert!(cached(&source, &mut cache, RasterLimits::default(), f64::NAN).is_err());
    assert_eq!(cache.retained_bytes(), before);
    for index in 0..80 {
        let mut changed = list(size, Color::BLACK);
        if let DrawCommand::Fill { transform, .. } = &mut changed.commands[0] {
            *transform = Transform::translate(index as f64 / 1000.0, 0.0);
        }
        cached(&changed, &mut cache, RasterLimits::default(), tolerance).unwrap();
    }
    assert!(cache.shared.len() > 64 && cache.shared.len() <= 2048);
    assert!(cache.retained_bytes() <= 16 * 1024 * 1024);
}

#[test]
fn shape_clip_replay_matches_rasterizing_and_keeps_budget_failures() {
    let size = PixelSize::new(24, 24);
    let clip = Path2D::parse_svg("M3 2C20 0 23 9 18 21L2 17Z").unwrap();
    let source = Node::group([Node::from(
        Path2D::parse_svg("M0 0H24V24H0Z")
            .unwrap()
            .fill(Color::WHITE),
    )
    .clip_with(clip, motion_core::ClipMode::Geometric)])
    .compile(Size::new(24.0, 24.0), size, ValidationLimits::default())
    .unwrap()
    .into_editable();
    let reference = source
        .prepare(ValidationLimits::default(), RasterLimits::default())
        .unwrap();
    assert_eq!(reference.clips.len(), 1);
    let tolerance = 1.0 / 256.0;
    let mut cache = CoverageCache::default();
    for _ in 0..2 {
        compare(
            &cached(&source, &mut cache, RasterLimits::default(), tolerance).unwrap(),
            &reference,
        );
    }
    let exact = RasterLimits {
        max_pixels: 576,
        max_bytes: reference.stats().peak_bytes,
        max_work: reference.stats().work,
    };
    for limits in [
        RasterLimits {
            max_bytes: exact.max_bytes - 1,
            ..exact
        },
        RasterLimits {
            max_work: exact.max_work - 1,
            ..exact
        },
    ] {
        let expected = source
            .prepare(ValidationLimits::default(), limits)
            .unwrap_err();
        let actual = cached(&source, &mut cache, limits, tolerance).unwrap_err();
        assert_eq!(
            (actual.kind, actual.field, actual.path, actual.message),
            (
                expected.kind,
                expected.field,
                expected.path,
                expected.message
            )
        );
    }
    compare(
        &cached(&source, &mut cache, exact, tolerance).unwrap(),
        &reference,
    );
}
