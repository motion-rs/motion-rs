//! Command execution for the shared project CLI: parses process arguments with clap,
//! executes list, still, export, studio and validate commands, renders help and reports
//! the completed [`ProjectOutcome`].

use super::args::{ExportArgs, ProjectCommand, StillArgs, StudioArgs, ValidateArgs};
use super::flags::parse_codec;
use super::{Project, ProjectTimelineInfo, ValidationReport, invalid, json_error};
use crate::encode::ExportReport;
use crate::{ExportOptions, Result, Time};
use clap::{CommandFactory, FromArgMatches, Parser, error::ErrorKind};
use std::path::PathBuf;

mod progress;

/// A CLI command's completed result, never a partial file notification.
#[derive(Debug)]
pub enum ProjectOutcome {
    /// Usage text produced without loading project content.
    Help(String),
    /// Registry discovery without timeline construction.
    Listed(Vec<ProjectTimelineInfo>),
    /// JSON Schema of the declarative document format.
    #[cfg(feature = "declarative")]
    Schema(serde_json::Value),
    /// Committed still PNG.
    Still(PathBuf),
    /// Successfully committed video.
    Exported(ExportReport),
    /// A film passed `validate`.
    Validated(ValidationReport),
    /// Explicit Studio returned.
    StudioClosed,
}

#[derive(Parser)]
struct ProjectArguments {
    #[command(subcommand)]
    command: ProjectCommand,
}

impl Project {
    /// Parses list/still/export/studio/validate, plus `schema` with feature `declarative`,
    /// where NAME may also be a `.json` document path. Use `--params JSON` or `--params-file PATH`,
    /// `--size WxH`, `--backend cpu|gpu`, `--time SECONDS`, `--range START:END`,
    /// `--output PATH` and `--overwrite`. Studio opens only on an explicit command.
    ///
    /// `arguments` excludes the program name. Help requests, including no arguments at all,
    /// return [`ProjectOutcome::Help`]; malformed arguments are errors.
    pub fn cli(&self, arguments: impl IntoIterator<Item = String>) -> Result<ProjectOutcome> {
        let program = self.program_name();
        let arguments = std::iter::once(program).chain(arguments);
        let mut matches = match self.command().try_get_matches_from(arguments) {
            Ok(matches) => matches,
            Err(error)
                if matches!(
                    error.kind(),
                    ErrorKind::DisplayHelp | ErrorKind::DisplayHelpOnMissingArgumentOrSubcommand
                ) =>
            {
                return Ok(ProjectOutcome::Help(error.to_string()));
            }
            Err(error) => return Err(invalid(error.to_string())),
        };
        let arguments = ProjectArguments::from_arg_matches_mut(&mut matches)
            .map_err(|error| invalid(error.to_string()))?;
        self.execute(arguments.command)
    }

    /// Executes a command parsed by clap, for programs that embed [`ProjectCommand`] in their
    /// own command line.
    pub fn execute(&self, command: ProjectCommand) -> Result<ProjectOutcome> {
        match command {
            ProjectCommand::List => Ok(ProjectOutcome::Listed(self.list())),
            #[cfg(feature = "declarative")]
            ProjectCommand::Schema => Ok(ProjectOutcome::Schema(motion_scene::schema())),
            ProjectCommand::Still(arguments) => self.execute_still(arguments),
            ProjectCommand::Export(arguments) => self.execute_export(arguments),
            ProjectCommand::Studio(arguments) => self.execute_studio(arguments),
            ProjectCommand::Validate(arguments) => self.execute_validate(arguments),
        }
    }

    fn command(&self) -> clap::Command {
        ProjectArguments::command()
            .name(self.program_name())
            .about(self.name.clone())
            .arg_required_else_help(true)
    }

    fn program_name(&self) -> String {
        std::env::args()
            .next()
            .and_then(|program| {
                PathBuf::from(program)
                    .file_name()
                    .map(|name| name.to_string_lossy().into_owned())
            })
            .unwrap_or_else(|| self.name.clone())
    }

    fn execute_studio(&self, arguments: StudioArgs) -> Result<ProjectOutcome> {
        let name = arguments
            .selection()
            .ok_or_else(|| invalid("Studio requires NAME or --timeline NAME"))?
            .to_owned();
        #[cfg(feature = "studio")]
        {
            let options = arguments.render.apply(self.options.clone());
            let parameters = arguments.parameters.into_value();
            let scene = self.studio_scene_with(&name, parameters, options)?;
            crate::studio::run(scene)?;
            Ok(ProjectOutcome::StudioClosed)
        }
        #[cfg(not(feature = "studio"))]
        Err(invalid(format!(
            "Studio command requires feature studio (requested {name})"
        )))
    }

    fn execute_validate(&self, arguments: ValidateArgs) -> Result<ProjectOutcome> {
        Ok(ProjectOutcome::Validated(self.check(
            &arguments.name,
            arguments.parameters.into_value(),
        )?))
    }

    fn execute_still(&self, arguments: StillArgs) -> Result<ProjectOutcome> {
        let StillArgs {
            name,
            time,
            frame,
            parameters,
            render,
            output,
        } = arguments;
        let options = render.apply(self.options.clone());
        let (timeline, options) = self.film_options(&name, parameters.into_value(), options)?;
        let time = match frame {
            Some(frame) => Time::checked_from_frame(i64::from(frame), timeline.frame_rate())
                .ok_or_else(|| invalid("Frame timestamp overflow"))?,
            None => time.unwrap_or(Time::ZERO),
        };
        let path = output
            .output
            .unwrap_or_else(|| PathBuf::from(format!("{}.png", stem(&name))));
        if !output.overwrite && path.try_exists()? {
            return Err(std::io::Error::new(
                std::io::ErrorKind::AlreadyExists,
                "Still destination exists",
            )
            .into());
        }
        crate::render_still(&timeline, time, &options)?
            .save_png_with_overwrite(&path, output.overwrite)?;
        Ok(ProjectOutcome::Still(path))
    }

    fn execute_export(&self, arguments: ExportArgs) -> Result<ProjectOutcome> {
        let ExportArgs {
            name,
            range,
            codec,
            quality,
            alpha,
            parameters,
            render,
            output,
        } = arguments;
        let mut options = render.apply(self.options.clone());
        options.background = options.background.or(Some(crate::Color::BLACK));
        let (timeline, options) = self.film_options(&name, parameters.into_value(), options)?;
        let codec = parse_codec(
            codec.as_deref().unwrap_or("h264"),
            quality,
            alpha,
            timeline.frame_rate(),
        )?;
        let extension = match codec {
            crate::Codec::Vp9 { .. } => "webm",
            crate::Codec::ProRes4444 => "mov",
            crate::Codec::PngSequence => "frames",
            crate::Codec::Gif { .. } => "gif",
            _ => "mp4",
        };
        let mut request = ExportOptions::new(
            output
                .output
                .unwrap_or_else(|| PathBuf::from(format!("{}.{extension}", stem(&name)))),
        );
        request.render = options;
        request.range = range;
        request.overwrite = output.overwrite;
        request = request.codec(codec);
        let progress = progress::CliProgress::stderr();
        let result = crate::export(&timeline, &request, &progress);
        progress.finish()?;
        Ok(ProjectOutcome::Exported(result?))
    }

    /// CLI usage without constructing timelines, loading fonts or probing devices.
    pub fn help(&self) -> String {
        self.command().render_help().to_string()
    }

    /// Executes process arguments while leaving result/error handling to the caller.
    pub fn try_run(&self) -> Result<ProjectOutcome> {
        self.cli(std::env::args().skip(1))
    }

    /// Runs the CLI, printing errors with Display and returning a process exit code.
    pub fn run(&self) -> std::process::ExitCode {
        match self
            .try_run()
            .and_then(|outcome| self.print_outcome(outcome))
        {
            Ok(()) => std::process::ExitCode::SUCCESS,
            Err(error) => {
                eprintln!("{error}");
                std::process::ExitCode::FAILURE
            }
        }
    }

    /// Prints a completed CLI result without invoking a builder or opening Studio.
    pub fn print_outcome(&self, outcome: ProjectOutcome) -> Result<()> {
        match outcome {
            ProjectOutcome::Help(help) => println!("{help}"),
            ProjectOutcome::Listed(entries) => {
                println!(
                    "{}",
                    serde_json::to_string_pretty(&entries).map_err(json_error)?
                )
            }
            #[cfg(feature = "declarative")]
            ProjectOutcome::Schema(schema) => {
                println!(
                    "{}",
                    serde_json::to_string_pretty(&schema).map_err(json_error)?
                )
            }
            ProjectOutcome::Validated(report) => println!("{report}"),
            ProjectOutcome::Still(path) => println!("{}", path.display()),
            ProjectOutcome::Exported(report) => println!("{}", report.path.display()),
            ProjectOutcome::StudioClosed => {}
        }
        Ok(())
    }
}

/// Default output stem: a document path's file stem, or the timeline name.
fn stem(name: &str) -> String {
    if name.ends_with(".json") {
        let path = std::path::Path::new(name);
        if let Some(stem) = path.file_stem() {
            return stem.to_string_lossy().into_owned();
        }
    }
    name.to_owned()
}
