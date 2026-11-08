//! Non-ignored guard for the P6 measurement suite: loose thresholds that only fail on
//! a real regression in coverage accuracy, sub-pixel stability or float gradients.
#![cfg(feature = "text")]

#[path = "../../../examples/quality_metrics/metrics.rs"]
mod metrics;

#[test]
fn fine_line_coverage_is_accurate_and_flicker_free() -> motion::Result {
    let lines = metrics::fine_lines()?;
    assert_eq!(lines.len(), 3);
    for line in lines {
        let width = line["width_px"].as_f64().unwrap();
        // Analytic coverage is stored at 8 bits, so errors beyond ~1.5 codes are bugs.
        let max_error = line["max_coverage_error"].as_f64().unwrap();
        assert!(
            max_error <= 1.5 / 255.0,
            "{width}px max coverage error {max_error}"
        );
        assert!(line["mean_coverage_error"].as_f64().unwrap() <= 0.75 / 255.0);
        // Integrated energy neither drifts from the analytic area nor flickers.
        assert!(
            line["energy_relative_error"].as_f64().unwrap() <= 0.01,
            "{width}px"
        );
        assert!(
            line["energy_relative_range"].as_f64().unwrap() <= 0.01,
            "{width}px"
        );
        // One 1/16px step cannot change the average pixel by more than the step's area.
        assert!(line["mean_adjacent_alpha_difference"].as_f64().unwrap() <= 0.002);
    }
    Ok(())
}

#[test]
fn text_is_stable_under_subpixel_shifts() -> motion::Result {
    let text = metrics::text_stability()?;
    assert!(text["energy_relative_range"].as_f64().unwrap() <= 0.005);
    assert!(
        text["reversal_fraction_of_changing_pixels"]
            .as_f64()
            .unwrap()
            <= 0.12
    );
    // A 1/16px shift moves any edge pixel by at most a small coverage fraction.
    assert!(text["max_adjacent_frame_alpha_step"].as_f64().unwrap() <= 0.1);
    Ok(())
}

#[test]
fn float_gradient_does_not_band_before_output_quantization() -> motion::Result {
    let gradient = metrics::gradient_banding()?;
    assert!(gradient["monotonic"].as_bool().unwrap());
    assert_eq!(gradient["unique_linear_values"].as_u64(), Some(1024));
    assert_eq!(gradient["longest_equal_f32_run"].as_u64(), Some(1));
    assert!(gradient["unique_srgb8_codes"].as_u64().unwrap() >= 250);
    assert!(gradient["max_srgb8_code_step"].as_u64().unwrap() <= 4);
    Ok(())
}
