//! Gallery command-line entry point that reports failures through the exit code.

fn main() -> std::process::ExitCode {
    match motion_gallery::run() {
        Ok(()) => std::process::ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("{error}");
            std::process::ExitCode::FAILURE
        }
    }
}
