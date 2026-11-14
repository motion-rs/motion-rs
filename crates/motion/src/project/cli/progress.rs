//! CLI export progress on stderr, separate from committed destination paths on stdout.

use crate::encode::{ExportPhase, ProgressSink};
use motion_core::sync::MutexExt;
use std::io::{self, IsTerminal, Write};
use std::sync::Mutex;

/// Percentage spacing between progress lines when stderr is redirected.
const LOG_PERCENT_STEP: u64 = 10;

pub(super) struct CliProgress<W: Write + Send> {
    state: Mutex<State<W>>,
}

impl CliProgress<io::Stderr> {
    pub(super) fn stderr() -> Self {
        let writer = io::stderr();
        let terminal = writer.is_terminal();
        Self::new(writer, terminal)
    }
}

impl<W: Write + Send> CliProgress<W> {
    fn new(writer: W, terminal: bool) -> Self {
        Self {
            state: Mutex::new(State {
                writer,
                terminal,
                last_percent: None,
                line_width: 0,
                error: None,
            }),
        }
    }

    pub(super) fn finish(&self) -> io::Result<()> {
        let mut state = self.state.lock_unpoisoned();
        if state.line_width != 0 {
            state.line("".into(), false);
        }
        match state.error.take() {
            Some(error) => Err(error),
            None => Ok(()),
        }
    }
}

impl<W: Write + Send> ProgressSink for CliProgress<W> {
    fn phase(&self, phase: ExportPhase) {
        let mut state = self.state.lock_unpoisoned();
        let message = match phase {
            ExportPhase::Preparing => "Export: preparing...",
            ExportPhase::Rendering => "Export: rendering...",
            ExportPhase::Finalizing => "Export: finalizing...",
            ExportPhase::Completed => "Export: completed",
        };
        state.line(message.into(), false);
        state.last_percent = None;
    }

    fn frame_completed(&self, completed: u64, total: u64) {
        let mut state = self.state.lock_unpoisoned();
        let percent = (u128::from(completed) * 100 / u128::from(total.max(1))).min(100) as u64;
        let step = if state.terminal { 1 } else { LOG_PERCENT_STEP };
        if let Some(previous) = state.last_percent
            && (percent == previous || percent < 100 && percent / step == previous / step)
        {
            return;
        }
        state.last_percent = Some(percent);
        let terminal = state.terminal;
        state.line(
            format!("Export: rendering {completed}/{total} frames ({percent}%)"),
            terminal,
        );
    }

    fn is_cancelled(&self) -> bool {
        self.state.lock_unpoisoned().error.is_some()
    }
}

struct State<W> {
    writer: W,
    terminal: bool,
    last_percent: Option<u64>,
    line_width: usize,
    error: Option<io::Error>,
}

impl<W: Write> State<W> {
    fn line(&mut self, message: String, inline: bool) {
        if self.error.is_some() {
            return;
        }
        let result = (|| {
            if inline {
                let width = self.line_width.max(message.len());
                write!(self.writer, "\r{message:width$}")?;
                self.line_width = width;
            } else {
                if self.line_width != 0 {
                    writeln!(self.writer)?;
                    self.line_width = 0;
                }
                if !message.is_empty() {
                    writeln!(self.writer, "{message}")?;
                }
            }
            self.writer.flush()
        })();
        if let Err(error) = result {
            self.error = Some(error);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn redirected_progress_is_throttled_and_separates_rendering_from_commit() {
        let progress = CliProgress::new(Vec::new(), false);
        progress.phase(ExportPhase::Preparing);
        progress.phase(ExportPhase::Rendering);
        for completed in 1..=100 {
            progress.frame_completed(completed, 100);
        }
        progress.phase(ExportPhase::Finalizing);
        progress.phase(ExportPhase::Completed);
        progress.finish().unwrap();
        let state = progress.state.lock_unpoisoned();
        let text = std::str::from_utf8(&state.writer).unwrap();
        assert!(!text.contains('\r'));
        assert_eq!(text.lines().count(), 15);
        assert!(text.contains("rendering 1/100 frames (1%)\n"));
        assert!(text.contains(
            "rendering 100/100 frames (100%)\nExport: finalizing...\nExport: completed\n"
        ));
        assert!(!text.contains("rendering 2/100"));
    }

    #[test]
    fn terminal_progress_rewrites_lines_and_closes_them_on_failure() {
        let progress = CliProgress::new(Vec::new(), true);
        progress.phase(ExportPhase::Rendering);
        progress.frame_completed(1, 3);
        progress.frame_completed(2, 3);
        progress.finish().unwrap();
        let state = progress.state.lock_unpoisoned();
        let text = std::str::from_utf8(&state.writer).unwrap();
        assert!(text.contains("\rExport: rendering 1/3 frames (33%)"));
        assert!(text.contains("\rExport: rendering 2/3 frames (66%)"));
        assert!(text.ends_with('\n'));
        assert!(!text.contains("completed"));
    }

    #[test]
    fn progress_write_errors_cancel_export_and_are_returned() {
        struct Broken;

        impl Write for Broken {
            fn write(&mut self, _: &[u8]) -> io::Result<usize> {
                Err(io::Error::new(io::ErrorKind::BrokenPipe, "closed stderr"))
            }

            fn flush(&mut self) -> io::Result<()> {
                Ok(())
            }
        }
        let progress = CliProgress::new(Broken, false);
        progress.phase(ExportPhase::Preparing);
        assert!(progress.is_cancelled());
        assert_eq!(
            progress.finish().unwrap_err().kind(),
            io::ErrorKind::BrokenPipe
        );
    }
}
