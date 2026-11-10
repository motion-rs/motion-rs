//! Command-line grammar for [`super::Project::cli`], declared with clap: the subcommands
//! and their flags, plus the value parsers for parameters, sizes and ranges.

use super::MAX_PARAMETERS;
use super::flags::parse_time;
use crate::{RenderOptions, Time};
use clap::{Args, Subcommand, ValueEnum};
use serde_json::Value;
use std::{io::Read, path::PathBuf};

/// A parsed project command, executed by [`super::Project::execute`].
#[derive(Debug, Subcommand)]
pub enum ProjectCommand {
    /// List the registered timelines without building them
    List,
    /// Print the JSON Schema of the declarative document format
    #[cfg(feature = "declarative")]
    Schema,
    /// Render one frame to a PNG
    Still(StillArgs),
    /// Export a video
    Export(ExportArgs),
    /// Open a timeline in Motion Studio
    Studio(StudioArgs),
    /// Check a timeline or document without rendering
    Validate(ValidateArgs),
}

/// Parameter overrides: inline JSON or a JSON file, never both.
#[derive(Args, Debug, Default)]
pub struct ParameterArgs {
    /// Parameter overrides as a JSON object
    #[arg(long, value_name = "JSON", conflicts_with = "params_file", value_parser = parse_parameters)]
    pub params: Option<Value>,
    /// Read parameter overrides from a JSON file
    #[arg(long, alias = "parameters", value_name = "FILE", value_parser = read_parameters)]
    pub params_file: Option<Value>,
}

impl ParameterArgs {
    pub(super) fn into_value(self) -> Option<Value> {
        self.params.or(self.params_file)
    }
}

/// Render configuration shared by every command that builds a film.
#[derive(Args, Debug, Default)]
pub struct RenderArgs {
    /// Output size
    #[arg(long, value_name = "WIDTHxHEIGHT", value_parser = parse_size)]
    pub size: Option<crate::PixelSize>,
    /// Render backend
    #[arg(long, value_enum)]
    pub backend: Option<Backend>,
    /// Allow a software GPU adapter
    #[arg(long)]
    pub allow_software_gpu: bool,
}

impl RenderArgs {
    pub(super) fn apply(&self, mut options: RenderOptions) -> RenderOptions {
        let allow_software = options.backend.allow_software() || self.allow_software_gpu;
        options.backend = self
            .backend
            .map_or(crate::BackendPreference::automatic(), Backend::preference)
            .with_software(allow_software);
        if let Some(size) = self.size {
            options.size = Some(size);
        }
        options
    }
}

/// Render backend selection.
#[derive(Clone, Copy, Debug, ValueEnum)]
pub enum Backend {
    /// Validate a GPU once, falling back to the CPU before the session starts
    Automatic,
    /// Bounded CPU reference
    Cpu,
    /// Explicit GPU without fallback
    Gpu,
}

impl Backend {
    fn preference(self) -> crate::BackendPreference {
        match self {
            Self::Automatic => crate::BackendPreference::automatic(),
            Self::Cpu => crate::BackendPreference::Cpu,
            Self::Gpu => crate::BackendPreference::gpu(),
        }
    }
}

/// Destination controls shared by still and export.
#[derive(Args, Debug)]
pub struct OutputArgs {
    /// Destination path (defaults to the timeline name or document stem)
    #[arg(short, long, value_name = "PATH")]
    pub output: Option<PathBuf>,
    /// Replace an existing destination
    #[arg(long)]
    pub overwrite: bool,
}

/// Arguments of `still`.
#[derive(Args, Debug)]
pub struct StillArgs {
    /// Registered timeline name or declarative `.json` document path
    pub name: String,
    /// Time to render in seconds, e.g. 1.5 or 500ms
    #[arg(long, value_name = "SECONDS", value_parser = parse_time_argument, conflicts_with = "frame")]
    pub time: Option<Time>,
    /// Frame index to render
    #[arg(long, value_name = "INDEX")]
    pub frame: Option<u32>,
    /// Parameter overrides
    #[command(flatten)]
    pub parameters: ParameterArgs,
    /// Render configuration
    #[command(flatten)]
    pub render: RenderArgs,
    /// Destination controls
    #[command(flatten)]
    pub output: OutputArgs,
}

/// Arguments of `export`.
#[derive(Args, Debug)]
pub struct ExportArgs {
    /// Registered timeline name or declarative `.json` document path
    pub name: String,
    /// Time range, e.g. 2s..6s or 2:6
    #[arg(long, value_name = "START..END", value_parser = parse_range)]
    pub range: Option<std::ops::Range<Time>>,
    /// Video codec
    #[arg(long, value_parser = CODECS)]
    pub codec: Option<String>,
    /// Encoder quality; not accepted by prores, png or gif
    #[arg(long)]
    pub quality: Option<u8>,
    /// Preserve transparency (vp9 and png sequences)
    #[arg(long)]
    pub alpha: bool,
    /// Parameter overrides
    #[command(flatten)]
    pub parameters: ParameterArgs,
    /// Render configuration
    #[command(flatten)]
    pub render: RenderArgs,
    /// Destination controls
    #[command(flatten)]
    pub output: OutputArgs,
}

/// Arguments of `studio`.
#[derive(Args, Debug)]
pub struct StudioArgs {
    /// Registered timeline name or declarative `.json` document path
    #[arg(conflicts_with = "timeline")]
    pub name: Option<String>,
    /// Timeline to open, as an alternative to the positional name
    #[arg(long)]
    pub timeline: Option<String>,
    /// Parameter overrides
    #[command(flatten)]
    pub parameters: ParameterArgs,
    /// Render configuration
    #[command(flatten)]
    pub render: RenderArgs,
}

impl StudioArgs {
    /// The selected timeline name, if any.
    pub fn selection(&self) -> Option<&str> {
        self.name.as_deref().or(self.timeline.as_deref())
    }

    /// Selects a timeline when none was chosen.
    pub fn select(&mut self, name: impl Into<String>) {
        self.name = Some(name.into());
    }
}

/// Arguments of `validate`.
#[derive(Args, Debug)]
pub struct ValidateArgs {
    /// Registered timeline name or declarative `.json` document path
    pub name: String,
    /// Parameter overrides
    #[command(flatten)]
    pub parameters: ParameterArgs,
    /// Render configuration
    #[command(flatten)]
    pub render: RenderArgs,
}

const CODECS: [&str; 9] = [
    "h264",
    "h265",
    "hevc",
    "vp9",
    "prores",
    "prores4444",
    "png",
    "png-sequence",
    "gif",
];

fn parse_parameters(value: &str) -> Result<Value, String> {
    if value.len() > MAX_PARAMETERS {
        return Err("parameters exceed 1 MiB".into());
    }
    serde_json::from_str(value).map_err(|error| error.to_string())
}

fn read_parameters(path: &str) -> Result<Value, String> {
    let metadata = std::fs::metadata(path).map_err(|error| error.to_string())?;
    if !metadata.is_file() || metadata.len() > MAX_PARAMETERS as u64 {
        return Err("parameter source must be a regular file <=1 MiB".into());
    }
    let mut bytes = Vec::new();
    std::fs::File::open(path)
        .and_then(|file| file.take(MAX_PARAMETERS as u64 + 1).read_to_end(&mut bytes))
        .map_err(|error| error.to_string())?;
    if bytes.len() > MAX_PARAMETERS {
        return Err("parameter source exceeds 1 MiB".into());
    }
    serde_json::from_slice(&bytes).map_err(|error| error.to_string())
}

fn parse_time_argument(value: &str) -> Result<Time, String> {
    parse_time(value).map_err(|error| error.to_string())
}

fn parse_range(value: &str) -> Result<std::ops::Range<Time>, String> {
    let (start, end) = value
        .split_once("..")
        .or_else(|| value.split_once(':'))
        .ok_or("expected 2s..6s or START:END seconds")?;
    let interval = parse_time_argument(start)?..parse_time_argument(end)?;
    if interval.start >= interval.end {
        return Err("range requires start < end".into());
    }
    Ok(interval)
}

fn parse_size(value: &str) -> Result<crate::PixelSize, String> {
    let (width, height) = value.split_once('x').ok_or("expected WIDTHxHEIGHT")?;
    let dimension = |value: &str| value.parse::<u32>().map_err(|error| error.to_string());
    Ok(crate::PixelSize::new(dimension(width)?, dimension(height)?))
}
