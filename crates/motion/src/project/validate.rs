//! Checking a film without rendering it: timeline validation, resource presence and
//! scene preflight of the first, middle and last frames.

use super::{Project, invalid};
use crate::{FrameRate, Result, Size};
use serde_json::Value;
use std::path::PathBuf;

/// What a successful [`Project::check`] verified.
#[derive(Clone, Debug, PartialEq)]
pub struct ValidationReport {
    /// Timeline name or document path that was checked.
    pub name: String,
    /// Logical design size.
    pub size: Size,
    /// Rational frame rate.
    pub frame_rate: FrameRate,
    /// Number of output frames.
    pub frames: u64,
    /// Film duration.
    pub duration: motion_core::Duration,
    /// Frame indices whose scenes were built and preflighted.
    pub checked_frames: Vec<u32>,
    /// Files the film refers to, all present; empty for code timelines.
    pub resources: Vec<PathBuf>,
}

impl std::fmt::Display for ValidationReport {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let rate = if self.frame_rate.denominator() == 1 {
            self.frame_rate.numerator().to_string()
        } else {
            format!(
                "{}/{}",
                self.frame_rate.numerator(),
                self.frame_rate.denominator()
            )
        };
        write!(
            formatter,
            "{}: valid ({}x{}, {rate} fps, {} frames, {} s, {} frames preflighted",
            self.name,
            self.size.width,
            self.size.height,
            self.frames,
            self.duration.as_seconds(),
            self.checked_frames.len()
        )?;
        match self.resources.len() {
            0 => write!(formatter, ")"),
            count => write!(formatter, ", {count} files)"),
        }
    }
}

impl Project {
    /// Checks a timeline or `.json` document without rendering or opening devices.
    ///
    /// The film is built and its timeline validated. A document's files must exist.
    /// The first, middle and last frames are then built and preflighted against the
    /// configured resources and limits, which reports invalid properties, missing
    /// fonts and over-budget content with their frame index. Rendering, encoding and
    /// device selection never happen.
    pub fn check(&self, name: &str, parameters: Option<Value>) -> Result<ValidationReport> {
        let film = self.film(name, parameters)?;
        for file in &film.watch {
            if !file.try_exists()? {
                return Err(invalid(format!("Missing file: {}", file.display())));
            }
        }
        let options = film.options(self.options.clone());
        let timeline = &film.timeline;
        let composition = crate::export::composition(timeline, &options)?;
        let frames = timeline.frame_count();
        let mut checked: Vec<u32> = [0, frames / 2, frames.saturating_sub(1)]
            .into_iter()
            .filter_map(|index| u32::try_from(index).ok())
            .collect();
        checked.sort_unstable();
        checked.dedup();
        if frames > 0 {
            for index in &checked {
                composition
                    .preflight_frame(*index, timeline.size(), |context| timeline.render(context))?;
            }
        } else {
            checked.clear();
        }
        Ok(ValidationReport {
            name: name.to_owned(),
            size: timeline.size(),
            frame_rate: timeline.frame_rate(),
            frames,
            duration: timeline.duration(),
            checked_frames: checked,
            resources: film.watch,
        })
    }
}
