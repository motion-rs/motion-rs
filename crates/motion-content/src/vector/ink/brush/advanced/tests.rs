use super::super::*;
use super::geometry::{outline, samples};
use super::material::material;
use super::*;
use motion_core::TimeLiteral;
use motion_render::RenderPrepare;

#[test]
fn pressure_follow_path_multi_subpaths_and_validation() {
    let stroke = BrushStroke::new(
        Path2D::parse_svg("M0 0L40 0M80 0L80 40").unwrap(),
        8.0,
        Color::WHITE,
    )
    .pressure_curve(vec![(0.0, 0.1), (0.5, 2.0), (1.0, 0.1)])
    .nib_mode(NibMode::FollowPath(0.0));
    assert!(bounds(&stroke).is_ok());
    let groups = samples(&stroke, 1.0).unwrap();
    assert_eq!(groups.len(), 2);
    let outline = outline(&stroke, &groups).unwrap();
    assert!(!outline.verbs().is_empty());
    assert!(bounds(&stroke.clone().pressure_curve(vec![(0.0, 1.0), (0.0, 2.0)])).is_err());
    assert!(samples(&stroke.clone().pressure_from(|_| f64::NAN), 1.0).is_err());
    let replaced = stroke
        .clone()
        .pressure_from(|_| f64::NAN)
        .pressure(1.0, 1.0, 1.0);
    assert!(samples(&replaced, 1.0).is_ok());
    assert!(
        samples(
            &stroke
                .pressure_from(|_| f64::NAN)
                .pressure_curve(vec![(0.0, 1.0), (1.0, 1.0)]),
            1.0
        )
        .is_ok()
    );
}

#[test]
fn brush_pressure_peak_keeps_full_arc_phase_during_reveal() -> Result<(), ValidationError> {
    let outline_bounds = |stroke: &BrushStroke| -> Result<Rect, ValidationError> {
        let groups = samples(stroke, stroke.progress)?;
        Ok(outline(stroke, &groups)?.bounding_box())
    };
    for length in [32.0, 36.0, 100.0] {
        let path = Path2D::parse_svg(&format!("M8 24L{} 24", 8.0 + length))?;
        let stroke = BrushStroke::new(path, 8.0, Color::BLACK)
            .pressure(0.0, 2.0, 0.0)
            .ink_load(1.0, 0.0);
        let full = outline_bounds(&stroke.clone().write(1.0))?;
        assert!(full.height() > 14.0);
        let bounds = outline_bounds(&stroke.clone().write(0.75))?;
        assert!(
            (bounds.top - full.top).abs() < 1e-9 && (bounds.bottom - full.bottom).abs() < 1e-9,
            "length={length} bounds={bounds:?}"
        );
        let early = outline_bounds(&stroke.write(0.25))?;
        assert!(early.height() < full.height() * 0.75);
    }
    Ok(())
}

#[test]
fn recorded_timing_and_cached_material_identity() {
    let input = [
        InputSample {
            position: Point::new(0.0, 0.0),
            pressure: 1.0,
            tilt: 0.0,
            time: Time::ZERO,
        },
        InputSample {
            position: Point::new(20.0, 0.0),
            pressure: 0.5,
            tilt: 0.3,
            time: 1.seconds(),
        },
        InputSample {
            position: Point::new(40.0, 0.0),
            pressure: 1.0,
            tilt: 0.6,
            time: 3.seconds(),
        },
    ];
    let stroke = BrushStroke::from_samples(&input, 8.0, Color::WHITE)
        .unwrap()
        .nib_mode(NibMode::Tilt);
    let revealed = stroke.clone().write_with(2.seconds()).unwrap();
    assert!((revealed.progress - 0.75).abs() < 1e-9);
    assert!(Arc::ptr_eq(&stroke.full_material, &revealed.full_material));
    assert!(Arc::ptr_eq(
        &stroke.full_material,
        &stroke
            .clone()
            .write_timed(1.seconds(), &BrushTiming::new(3.seconds()))
            .unwrap()
            .full_material
    ));
    assert!(!Arc::ptr_eq(
        &stroke.full_material,
        &stroke.clone().edge_darkening(0.3).full_material
    ));
    assert!(BrushStroke::from_samples(&[input[0], input[0]], 8.0, Color::WHITE).is_err());
}

#[test]
fn curved_recordings_pauses_tilt_and_flat_caps_are_analytic() -> motion_core::Result {
    let input = [
        InputSample {
            position: Point::new(0.0, 0.0),
            pressure: 1.0,
            tilt: 0.0,
            time: Time::ZERO,
        },
        InputSample {
            position: Point::new(20.0, 0.0),
            pressure: 1.0,
            tilt: 0.6,
            time: 1.seconds(),
        },
        InputSample {
            position: Point::new(20.0, 0.0),
            pressure: 1.0,
            tilt: 0.6,
            time: 2.seconds(),
        },
        InputSample {
            position: Point::new(30.0, 30.0),
            pressure: 0.5,
            tilt: 1.2,
            time: 3.seconds(),
        },
    ];
    let stroke = BrushStroke::from_samples(&input, 8.0, Color::WHITE)?.nib_mode(NibMode::Tilt);
    let options = &stroke.advanced;
    assert_eq!(options.input_arcs[1], options.input_arcs[2]);
    assert_eq!(
        stroke.clone().write_with(1.seconds())?.progress,
        stroke.clone().write_with(1500.milliseconds())?.progress
    );
    let halfway = samples(&stroke, 1.0)?;
    assert!(
        halfway[0]
            .iter()
            .any(|sample| sample.angle > 0.0 && sample.angle < 0.6)
    );
    let path = Path2D::parse_svg("M0 0L20 0M40 0L60 0")?;
    let mut timing = BrushTiming::new(2.seconds());
    timing.lift_pause = 1.seconds();
    let stroke = BrushStroke::new(path, 8.0, Color::WHITE);
    assert_eq!(
        stroke
            .clone()
            .write_timed(1500.milliseconds(), &timing)?
            .progress,
        0.5
    );
    assert_eq!(
        stroke
            .clone()
            .write_timed(2500.milliseconds(), &timing)?
            .progress,
        0.75
    );
    let flat = stroke.clone().caps(BrushCap::Flat, BrushCap::Flat);
    let outline = outline(&flat, &samples(&flat, 1.0)?)?;
    assert!(outline.bounding_box().left >= 0.0 && outline.bounding_box().right <= 60.0);
    assert!(
        stroke
            .clone()
            .tip_node(motion_core::Circle::new(Point::new(0.0, 0.0), 1.0).fill(Color::WHITE))
            .is_ok()
    );
    let late = stroke.clone().boil(1.0, 0.1, 17, 100);
    let early = stroke.clone().boil(1.0, 0.1, 17, 3);
    let again = stroke.boil(1.0, 0.1, 17, 100);
    assert_eq!(
        format!("{:?}", samples(&late, 1.0)?),
        format!("{:?}", samples(&again, 1.0)?)
    );
    assert_ne!(
        format!("{:?}", samples(&late, 1.0)?),
        format!("{:?}", samples(&early, 1.0)?)
    );
    Ok(())
}

#[test]
fn material_tiles_parallelism_and_reveal_cache_are_exact() -> motion_core::Result {
    let stroke = BrushStroke::new(
        Path2D::parse_svg("M8 16L56 16")?,
        8.0,
        Color::from_hex(0x4080c0),
    )
    .ink(Ink::new(19).granulation(0.3))
    .tiled_material(32);
    let bounds = bounds(&stroke)?;
    let serial = material(&stroke, bounds)?;
    let parallel = material(&stroke.clone().parallel_material(true), bounds)?;
    assert_eq!(serial.len(), parallel.len());
    for (serial, parallel) in serial.iter().zip(&parallel) {
        assert_eq!(serial.paint, parallel.paint);
    }
    motion_core::Node::from(stroke.clone().write(0.3)).compile(
        motion_core::Size::new(64.0, 32.0),
        motion_core::PixelSize::new(64, 32),
        motion_core::ValidationLimits::default(),
    )?;
    assert!(stroke.full_material.get().unwrap().is_ok());
    let first = stroke.full_material.get().unwrap() as *const _;
    motion_core::Node::from(stroke.clone().write(0.7)).compile(
        motion_core::Size::new(64.0, 32.0),
        motion_core::PixelSize::new(64, 32),
        motion_core::ValidationLimits::default(),
    )?;
    assert_eq!(first, stroke.full_material.get().unwrap() as *const _);
    assert!(material(&stroke.material_work_limit(1), bounds).is_err());
    Ok(())
}

#[test]
fn pigment_intersections_and_height_outputs_are_nonblank() -> motion_core::Result {
    let mixed = PigmentMix::KubelkaMunk
        .mix(Color::from_hex(0x0000ff), Color::from_hex(0xffff00), 0.5)?
        .to_rgba_float();
    assert!(mixed[1] > mixed[0] && mixed[1] > mixed[2]);
    let first = BrushStroke::new(
        Path2D::parse_svg("M8 16L56 16")?,
        8.0,
        Color::from_hex(0x0000ff),
    );
    let second = BrushStroke::new(
        Path2D::parse_svg("M32 4L32 28")?,
        8.0,
        Color::from_hex(0xffff00),
    );
    let limits = motion_core::RasterLimits::default();
    let render = |node: motion_core::Node| -> motion_core::Result<Vec<u8>> {
        Ok(node
            .compile(
                motion_core::Size::new(64.0, 32.0),
                motion_core::PixelSize::new(64, 32),
                motion_core::ValidationLimits::default(),
            )?
            .prepare(motion_core::ValidationLimits::default(), limits)?
            .render_linear(limits)?
            .to_srgb_rgba8(false, limits)?
            .bytes()
            .to_vec())
    };
    let painting = PigmentMix::KubelkaMunk.painting(vec![first.clone(), second]);
    let pixels = render(motion_core::Node::from(painting.clone()))?;
    let center = &pixels[(16 * 64 + 32) * 4..(16 * 64 + 33) * 4];
    assert!(center[1] > center[0] && center[1] > center[2]);
    assert_eq!(pixels, render(motion_core::Node::from(painting))?);
    let height = render(motion_core::Node::from(first.height_map(1.0)))?;
    assert!(
        height
            .as_chunks::<4>()
            .0
            .iter()
            .any(|pixel| pixel[0] > 0 && pixel[0] == pixel[1] && pixel[1] == pixel[2])
    );
    Ok(())
}

#[test]
fn tiled_pixels_match_single_tile_and_default_reveal_is_unchanged() -> motion_core::Result {
    let stroke = BrushStroke::new(
        Path2D::parse_svg("M8 16L56 16")?,
        8.0,
        Color::from_hex(0x4080c0),
    )
    .ink(Ink::new(19).granulation(0.3));
    let render = |stroke: BrushStroke| -> motion_core::Result<Vec<u8>> {
        let limits = motion_core::RasterLimits::default();
        let prepared = motion_core::Node::from(stroke)
            .compile(
                motion_core::Size::new(64.0, 32.0),
                motion_core::PixelSize::new(64, 32),
                motion_core::ValidationLimits::default(),
            )?
            .prepare(motion_core::ValidationLimits::default(), limits)?;
        Ok(prepared
            .render_linear(limits)?
            .to_srgb_rgba8(false, limits)?
            .bytes()
            .to_vec())
    };
    let tiled = render(stroke.clone().tiled_material(16))?;
    let whole = render(stroke.clone().tiled_material(256))?;
    assert_eq!(tiled, whole);
    assert!(tiled.iter().any(|byte| *byte > 0));
    let revealed = stroke.clone().write(0.5);
    let mut uncached = revealed.clone();
    uncached.reset();
    assert_eq!(render(revealed)?, render(uncached)?);
    Ok(())
}

#[test]
#[cfg(feature = "gpu")]
#[ignore = "requires native GPU; no fonts, window or encoder"]
fn extended_brush_material_matches_cpu_gpu() -> motion_core::Result {
    let stroke = BrushStroke::new(
        Path2D::parse_svg("M8 16Q32 0 56 16")?,
        8.0,
        Color::from_hex(0x4080c0),
    )
    .ink(Ink::new(19).granulation(0.3))
    .pressure_curve(vec![(0.0, 0.2), (0.5, 1.0), (1.0, 0.1)])
    .nib_mode(NibMode::FollowPath(0.0))
    .tiled_material(16);
    let limits = motion_core::RasterLimits::default();
    let prepared = motion_core::Node::from(stroke)
        .compile(
            motion_core::Size::new(64.0, 32.0),
            motion_core::PixelSize::new(64, 32),
            motion_core::ValidationLimits::default(),
        )?
        .prepare(motion_core::ValidationLimits::default(), limits)?;
    let expected = prepared
        .render_linear(limits)?
        .to_srgb_rgba8(false, limits)?;
    let mut gpu = motion_render::gpu::Gpu::new()?;
    let actual = gpu
        .render(&prepared, motion_render::gpu::Plan::new(&prepared, limits)?)?
        .to_srgb_rgba8(false, limits)?;
    assert!(
        expected
            .bytes()
            .iter()
            .zip(actual.bytes())
            .all(|(expected, actual)| expected.abs_diff(*actual) <= 2)
    );
    let replay = gpu
        .render(&prepared, motion_render::gpu::Plan::new(&prepared, limits)?)?
        .to_srgb_rgba8(false, limits)?;
    assert_eq!(actual.bytes(), replay.bytes());
    Ok(())
}
