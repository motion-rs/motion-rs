//! Prints the P6 image-quality measurements as JSON; see `docs/quality-report.md`.

#[path = "quality_metrics/metrics.rs"]
mod metrics;

fn main() -> motion::Result {
    let report = serde_json::json!({
        "version": 2,
        "backend": "cpu",
        "translation_step_px": metrics::STEP,
        "fine_lines": metrics::fine_lines()?,
        "text": metrics::text_stability()?,
        "gradient": metrics::gradient_banding()?,
    });
    println!(
        "{}",
        serde_json::to_string_pretty(&report).map_err(std::io::Error::other)?
    );
    Ok(())
}
