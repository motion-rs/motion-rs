//! Shared P6 image-quality measurements for `examples/quality_metrics.rs` and the
//! `quality_metrics` integration test. CPU linear render path only; no timing.

use motion::extend::RenderPrepare;
use motion::prelude::*;
use motion::render::{FrameBuffer, RasterLimits, ValidationLimits};
use serde_json::{Value, json};

/// Translation step used by every sub-pixel sweep (1/16 px).
pub const STEP: f64 = 1.0 / 16.0;

const STEPS: u32 = 16;

fn render(
    node: Node,
    width: u32,
    height: u32,
    fonts: Option<FontLibrary>,
) -> motion::Result<FrameBuffer> {
    let limits = RasterLimits::default();
    let size = Size::new(f64::from(width), f64::from(height));
    let compiled = match fonts {
        Some(fonts) => node.compile_with_fonts(
            size,
            PixelSize::new(width, height),
            ValidationLimits::default(),
            fonts,
        )?,
        None => node.compile(
            size,
            PixelSize::new(width, height),
            ValidationLimits::default(),
        )?,
    };
    Ok(compiled
        .prepare(ValidationLimits::default(), limits)?
        .render_linear(limits)?)
}

fn alpha_sum(frame: &FrameBuffer) -> f64 {
    frame.linear_pixels().map(|pixel| f64::from(pixel[3])).sum()
}

fn mean_adjacent_difference(first: &FrameBuffer, second: &FrameBuffer) -> f64 {
    first
        .linear_pixels()
        .zip(second.linear_pixels())
        .map(|(first, second)| f64::from((first[3] - second[3]).abs()))
        .sum::<f64>()
        / first.linear_pixels().len() as f64
}

/// Exact overlap of `[start, end]` with the unit pixel `[column, column + 1]`.
fn overlap(start: f64, end: f64, column: u32) -> f64 {
    (end.min(f64::from(column) + 1.0) - start.max(f64::from(column))).max(0.0)
}

/// 0.25/0.5/1px lines swept in 1/16px steps in both orientations, compared against
/// analytic pixel coverage and for integrated-energy flicker.
pub fn fine_lines() -> motion::Result<Vec<Value>> {
    const LENGTH: f64 = 48.0;
    let mut report = Vec::new();
    for width in [0.25, 0.5, 1.0] {
        let mut differences = Vec::new();
        let mut energies = Vec::new();
        let mut errors = Vec::new();
        for horizontal in [false, true] {
            let mut previous: Option<FrameBuffer> = None;
            let mut max_error = 0.0_f64;
            let mut sum_error = 0.0;
            let mut samples = 0.0;
            for step in 0..=STEPS {
                let start = 20.0 + f64::from(step) * STEP;
                let rect = if horizontal {
                    Rect::new(8.0, start, 8.0 + LENGTH, start + width)
                } else {
                    Rect::new(start, 8.0, start + width, 8.0 + LENGTH)
                };
                let current = render(rect.fill(Color::WHITE).into(), 128, 64, None)?;
                // Sample a row/column through the middle of the line, away from its ends.
                for coordinate in 16..28_u32 {
                    let (x, y) = if horizontal {
                        (32, coordinate)
                    } else {
                        (coordinate, 32)
                    };
                    let actual = f64::from(current.linear_pixel(x, y).unwrap()[3]);
                    let error = (actual - overlap(start, start + width, coordinate)).abs();
                    max_error = max_error.max(error);
                    sum_error += error;
                    samples += 1.0;
                }
                if !horizontal {
                    energies.push(alpha_sum(&current));
                    if let Some(previous) = &previous {
                        differences.push(mean_adjacent_difference(previous, &current));
                    }
                }
                previous = Some(current);
            }
            errors.push((max_error, sum_error / samples));
        }
        let lowest = energies.iter().copied().fold(f64::INFINITY, f64::min);
        let highest = energies.iter().copied().fold(0.0, f64::max);
        let area = width * LENGTH;
        report.push(json!({
            "width_px": width,
            "max_coverage_error": errors.iter().map(|error| error.0).fold(0.0, f64::max),
            "mean_coverage_error": errors.iter().map(|error| error.1).sum::<f64>() / errors.len() as f64,
            "max_coverage_error_vertical_horizontal": [errors[0].0, errors[1].0],
            "mean_adjacent_alpha_difference": differences.iter().sum::<f64>() / differences.len() as f64,
            "alpha_energy_range": [lowest, highest],
            "energy_relative_error": (lowest - area).abs().max((highest - area).abs()) / area,
            "energy_relative_range": (highest - lowest) / highest,
        }));
    }
    Ok(report)
}

/// Noto Sans text translated in 1/16px steps: integrated energy stability, per-frame
/// edge motion and per-pixel direction reversals.
pub fn text_stability() -> motion::Result<Value> {
    let font_path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../tests/fixtures/fonts/NotoSans-Regular.ttf");
    let mut fonts = FontLibrary::new();
    fonts.load_file(font_path)?;
    let mut energies = Vec::new();
    let mut frames = Vec::new();
    for step in 0..=STEPS {
        let node: Node = Text::new("Motion")
            .size(22.0)
            .fill(Color::WHITE)
            .translate(12.0 + f64::from(step) * STEP, 14.0);
        let current = render(node, 128, 64, Some(fonts.clone()))?;
        energies.push(alpha_sum(&current));
        frames.push(
            current
                .linear_pixels()
                .map(|pixel| pixel[3])
                .collect::<Vec<_>>(),
        );
    }
    let mut reversals = 0;
    let mut touched = 0;
    let mut max_step = 0.0_f32;
    for index in 0..frames[0].len() {
        let mut direction = 0;
        let mut reversed = false;
        let mut changed = false;
        for pair in frames.windows(2) {
            let change = pair[1][index] - pair[0][index];
            max_step = max_step.max(change.abs());
            if change.abs() <= 1.0 / 255.0 {
                continue;
            }
            changed = true;
            let next = if change > 0.0 { 1 } else { -1 };
            reversed |= direction != 0 && direction != next;
            direction = next;
        }
        touched += usize::from(changed);
        reversals += usize::from(reversed);
    }
    let lowest = energies.iter().copied().fold(f64::INFINITY, f64::min);
    let highest = energies.iter().copied().fold(0.0, f64::max);
    Ok(json!({
        "alpha_energy": energies,
        "energy_relative_range": (highest - lowest) / highest,
        "pixels_with_direction_reversal": reversals,
        "pixels_changing_over_sweep": touched,
        "reversal_fraction_of_changing_pixels": reversals as f64 / touched.max(1) as f64,
        "max_adjacent_frame_alpha_step": max_step,
    }))
}

fn srgb_code(linear: f32) -> u8 {
    let linear = f64::from(linear).clamp(0.0, 1.0);
    let encoded = if linear <= 0.003_130_8 {
        linear * 12.92
    } else {
        1.055 * linear.powf(1.0 / 2.4) - 0.055
    };
    (encoded * 255.0).round() as u8
}

/// A 1024px black-to-white float gradient: f32-stage collapse, monotonicity and what
/// a plain 8-bit sRGB quantization would produce (the output stage adds Bayer dither).
pub fn gradient_banding() -> motion::Result<Value> {
    let gradient: Node = Rect::new(0.0, 0.0, 1024.0, 2.0)
        .fill(
            LinearGradient::new(Point::ORIGIN, Point::new(1.0, 0.0))
                .stop(0.0, Color::BLACK)
                .stop(1.0, Color::WHITE),
        )
        .into();
    let frame = render(gradient, 1024, 2, None)?;
    let values = frame
        .linear_pixels()
        .take(1024)
        .map(|pixel| pixel[0])
        .collect::<Vec<_>>();
    let unique = values
        .iter()
        .map(|value| value.to_bits())
        .collect::<std::collections::BTreeSet<_>>()
        .len();
    let monotonic = values.windows(2).all(|pair| pair[1] >= pair[0]);
    let mut longest_equal_run = 1;
    let mut run = 1;
    for pair in values.windows(2) {
        run = if pair[0] == pair[1] { run + 1 } else { 1 };
        longest_equal_run = longest_equal_run.max(run);
    }
    let max_linear_step = values
        .windows(2)
        .map(|pair| pair[1] - pair[0])
        .fold(0.0_f32, f32::max);
    let codes = values.iter().copied().map(srgb_code).collect::<Vec<_>>();
    let unique_codes = codes
        .iter()
        .collect::<std::collections::BTreeSet<_>>()
        .len();
    let max_code_step = codes
        .windows(2)
        .map(|pair| pair[1].abs_diff(pair[0]))
        .max()
        .unwrap_or(0);
    Ok(json!({
        "pixels": values.len(),
        "unique_linear_values": unique,
        "monotonic": monotonic,
        "longest_equal_f32_run": longest_equal_run,
        "max_linear_step": max_linear_step,
        "unique_srgb8_codes": unique_codes,
        "max_srgb8_code_step": max_code_step,
    }))
}
