//! First-party FFmpeg C API boundary using build-time bindgen declarations.
//!
//! Building requires pkg-config, FFmpeg development headers/libraries and libclang.
//! Only functions used by the safe boundary are generated into OUT_DIR. Bindgen
//! and libclang are build-time requirements, not runtime encoding dependencies.

use std::ffi::{CStr, CString};
use std::io;
use std::path::Path;

mod audio;
mod encoder;
/// Bounded, native inspection helpers for export verification; no subprocesses.
pub mod inspect;
mod settings;

pub use audio::{AudioMix, AudioPcm, Mixer, PcmSource};
pub use encoder::Encoder;
pub use settings::{H265Profile, MediaChapter, MediaMetadata, VideoCodec, VideoSettings};

/// Integer timeline clock used for chapters and the MP4 movie timescale. It divides evenly by
/// common frame rates and by 48 kHz audio, avoiding millisecond rounding.
const TIMELINE_TICKS_PER_SECOND: i32 = 705_600_000;

#[allow(
    non_camel_case_types,
    non_snake_case,
    non_upper_case_globals,
    dead_code
)]
#[allow(
    clippy::type_complexity,
    reason = "Bindgen preserves C ABI function-pointer declarations"
)]
mod ffi {
    include!(concat!(env!("OUT_DIR"), "/bindings.rs"));

    pub use self::MOTION_AVERROR_EOF as AVERROR_EOF;
}

fn check(code: i32, operation: &str) -> io::Result<()> {
    if code >= 0 {
        return Ok(());
    }
    let mut buffer = [0 as std::ffi::c_char; 256];
    // SAFETY: `av_strerror` writes at most `buffer.len()` bytes and always NUL-terminates
    // within that length, so `CStr::from_ptr` reads inside the stack buffer.
    let message = unsafe {
        ffi::av_strerror(code, buffer.as_mut_ptr(), buffer.len());
        CStr::from_ptr(buffer.as_ptr()).to_string_lossy()
    };
    Err(io::Error::other(format!("{operation}: {message} ({code})")))
}

fn path_string(path: &Path) -> io::Result<CString> {
    #[cfg(unix)]
    let bytes = {
        use std::os::unix::ffi::OsStrExt;
        path.as_os_str().as_bytes()
    };
    #[cfg(not(unix))]
    let bytes = path
        .to_str()
        .ok_or_else(|| {
            io::Error::new(
                io::ErrorKind::InvalidInput,
                "FFmpeg requires UTF-8 paths on this platform",
            )
        })?
        .as_bytes();
    CString::new(bytes).map_err(|error| io::Error::new(io::ErrorKind::InvalidInput, error))
}

fn again(code: i32) -> bool {
    code < 0 && std::io::Error::from_raw_os_error(-code).kind() == io::ErrorKind::WouldBlock
}

/// Returns the linked libavutil version, without launching a process.
pub fn version() -> u32 {
    // SAFETY: `avutil_version` has no preconditions; it only returns the linked version number.
    unsafe { ffi::avutil_version() }
}
