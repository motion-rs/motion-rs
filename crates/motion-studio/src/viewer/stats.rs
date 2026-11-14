//! Studio statistics display and plain-text clipboard export from the same rows.

use super::inspector_action;
use crate::app::StudioApp;
use crate::theme;
use eframe::egui::{self, RichText};
use egui_phosphor::regular as icons;

impl StudioApp {
    pub(super) fn stats_panel(&self, ui: &mut egui::Ui) {
        let copy = inspector_action(ui, icons::COPY, "Copy stats to clipboard");
        let mut panel = StatsPanel {
            ui,
            text: copy.then(String::new),
        };
        let rate =
            f64::from(self.clock.rate.numerator()) / f64::from(self.clock.rate.denominator());
        let elapsed = self
            .observations
            .presentation_times
            .front()
            .and_then(|first| {
                self.observations
                    .presentation_times
                    .back()?
                    .checked_duration_since(*first)
            })
            .filter(|elapsed| !elapsed.is_zero());
        panel.section("Playback");
        panel.row(
            "Rate",
            &elapsed.map_or_else(
                || format!("-- / {rate:.2} fps"),
                |elapsed| {
                    format!(
                        "{:.1} / {rate:.2} fps",
                        (self.observations.presentation_times.len() - 1) as f64
                            / elapsed.as_secs_f64()
                    )
                },
            ),
        );
        panel.row(
            "Skipped frames",
            &self.observations.dropped_frames.to_string(),
        );
        panel.section("Preview worker");
        panel.row(
            "Total",
            &format!("{:.2} ms", self.observations.elapsed.as_secs_f64() * 1000.0),
        );
        if let Some(timings) = self.observations.timings {
            for (stage, time) in [
                ("Evaluate", timings.evaluation),
                ("Compile", timings.compile),
                ("Coverage", timings.coverage),
                ("Plan (host)", timings.plan),
                ("Command encode (host)", timings.encode),
                ("CPU composition", timings.composition),
                (
                    if timings.fused_output {
                        "CPU compose + output"
                    } else {
                        "Output conversion"
                    },
                    timings.output,
                ),
                ("Completion/readback (host)", timings.readback),
                ("Upload enqueue", timings.upload),
                ("Native submit (CPU wall)", timings.submission),
            ] {
                panel.row(stage, &format!("{:.2} ms", time.as_secs_f64() * 1000.0));
            }
            #[cfg(feature = "gpu")]
            if let Some(gpu) = timings.gpu {
                panel.section("GPU timestamps");
                for (stage, time) in [
                    ("Composition", gpu.composition),
                    ("Output", gpu.output),
                    ("Readback copy", gpu.readback_copy),
                    ("Flatten", gpu.coverage[0]),
                    ("Count", gpu.coverage[1]),
                    ("Prefix", gpu.coverage[2]),
                    ("Scatter", gpu.coverage[3]),
                    ("Carry", gpu.coverage[4]),
                    ("Coverage", gpu.coverage[5]),
                ] {
                    let value = match time.duration {
                        Some(duration) => {
                            format!("{:.2} ms", duration.as_secs_f64() * 1000.0)
                        }
                        None => format!("{:?}", time.status),
                    };
                    panel.row(stage, &value);
                }
                panel.row(
                    "Query buffers",
                    &format!("{} bytes", gpu.instrumentation_bytes),
                );
            } else {
                panel.row("GPU timestamps", "Unavailable");
            }
        }
        panel.section("Cache");
        panel.row(
            "Hits",
            &format!(
                "{} / {}",
                self.observations.cache_hits, self.observations.preview_count
            ),
        );
        panel.row(
            "Size",
            &format!("{:.1} MiB", self.cached_bytes as f64 / 1048576.0),
        );
        if let Some(stats) = self.observations.frame_stats {
            panel.section("Frame cost (work / peak)");
            for (stage, work, peak) in [
                (
                    "Prepare",
                    stats.preparation.work,
                    stats.preparation.peak_bytes,
                ),
                ("Composite", stats.linear.work, stats.linear.peak_bytes),
                ("Output", stats.output.work, stats.output.peak_bytes),
            ] {
                panel.row(
                    stage,
                    &format!("{work} / {:.1} MiB", peak as f64 / 1048576.0),
                );
            }
        }
        if let Some(text) = panel.text {
            panel.ui.ctx().copy_text(text);
        }
    }
}

struct StatsPanel<'a> {
    ui: &'a mut egui::Ui,
    text: Option<String>,
}

impl StatsPanel<'_> {
    fn section(&mut self, title: &str) {
        theme::section(self.ui, title);
        if let Some(text) = &mut self.text {
            if !text.is_empty() {
                text.push('\n');
            }
            text.push_str(title);
            text.push('\n');
        }
    }

    fn row(&mut self, name: &str, value: &str) {
        self.ui.horizontal(|ui| {
            ui.add(egui::Label::new(RichText::new(name).weak()).truncate());
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                ui.add(egui::Label::new(RichText::new(value).monospace()).truncate());
            });
        });
        if let Some(text) = &mut self.text {
            text.push_str(name);
            text.push_str(": ");
            text.push_str(value);
            text.push('\n');
        }
    }
}
