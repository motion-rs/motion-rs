//! `motion`: loads declarative JSON documents and validates, renders, exports or previews them.
//!
//! The clap grammar lives in `motion::Project`; this program registers no timelines, so every
//! NAME is a path to a `.json` document.

fn main() -> std::process::ExitCode {
    motion::Project::new("motion").run()
}
