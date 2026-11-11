//! Unified engine failures with retained causes and frame diagnostics.

use std::{error::Error as StdError, fmt, io};

use crate::ValidationError;

/// Owned source error that can cross rendering-worker boundaries.
pub type BoxedError = Box<dyn StdError + Send + Sync + 'static>;

/// Stable categories for programmatic handling across engine boundaries.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub enum ErrorKind {
    /// Invalid scene properties or exhausted structural/raster allowances.
    Validation,
    /// Filesystem or other operating-system I/O failure.
    Io,
    /// Loading or decoding a named resource failed.
    Asset,
    /// SVG parsing failed.
    Svg,
    /// Formula compilation failed.
    Formula,
    /// Project parameters could not be decoded.
    Parameters,
    /// Timeline construction or timing failed.
    Timeline,
    /// A requested GPU could not be initialized.
    GpuUnavailable,
    /// GPU submission, readback or allocation failed without confirmed device loss.
    Gpu,
    /// An initialized GPU device was lost.
    DeviceLost,
    /// A scene callback panicked at a worker boundary.
    ScenePanic,
    /// Video or audio encoding failed.
    Encoder,
    /// Cancellation was observed before output commit.
    Cancelled,
    /// An extension or other subsystem failed.
    Other,
}

/// Engine failure retaining its original source and optional frame context.
///
/// Lower-level geometry APIs still return ValidationError directly. Converting
/// them into this type retains all property/path information. Synchronous scene
/// callbacks still unwind; workers turn caught payloads into ScenePanic.
///
/// ```
/// use motion_core::{Error, ErrorKind};
/// let error = Error::from(std::io::Error::other("disk failure")).at_frame(12);
/// assert_eq!(error.kind(), ErrorKind::Io);
/// assert_eq!(error.frame(), Some(12));
/// assert!(error.downcast_ref::<std::io::Error>().is_some());
/// ```
#[derive(Debug)]
#[non_exhaustive]
pub enum Error {
    /// Ordered diagnostics retained while authoring an invalid value.
    BuildErrors(Vec<std::sync::Arc<Error>>),
    /// Detailed scene/property diagnostic.
    Validation(ValidationError),
    /// Original filesystem or operating-system error.
    Io(io::Error),
    /// Named resource failure with the original decoder/load error.
    Asset {
        /// Resource category, such as image, font or model.
        kind: &'static str,
        /// User-facing resource identifier or source path.
        source_name: String,
        /// Original failure.
        cause: BoxedError,
    },
    /// SVG parser failure.
    Svg(BoxedError),
    /// Formula compilation failure, optionally retaining structured diagnostics.
    Formula {
        /// User-facing explanation.
        message: String,
        /// Original structured diagnostics with source ranges, when available.
        source: Option<BoxedError>,
    },
    /// Invalid serialized project parameters.
    Parameters(BoxedError),
    /// Timeline construction failure.
    Timeline {
        /// Timeline identifier.
        timeline: String,
        /// Violated timing contract.
        message: String,
    },
    /// GPU setup failure; explicit GPU selection must not silently fall back.
    GpuUnavailable(BoxedError),
    /// GPU submission/readback failure without a confirmed device-loss callback.
    Gpu(BoxedError),
    /// Runtime GPU loss with its original driver error.
    DeviceLost(BoxedError),
    /// Caught scene panic. The message is retained for string payloads.
    ScenePanic {
        /// Registered scene or timeline name.
        timeline: String,
        /// Requested frame index.
        frame: u64,
        /// Panic payload text, or a non-string-payload marker.
        message: String,
    },
    /// Native encoder failure with its original source.
    Encoder(BoxedError),
    /// Export was cancelled before destination commit.
    Cancelled,
    /// An otherwise classified error annotated with a frame index.
    AtFrame {
        /// Requested frame index.
        frame: u64,
        /// Original classified error.
        cause: Box<Error>,
    },
    /// Extension error not covered by a more specific category.
    Other(BoxedError),
}

impl Error {
    /// All deferred authoring diagnostics, preserving their original categories.
    pub fn build_errors(&self) -> &[std::sync::Arc<Error>] {
        match self {
            Self::BuildErrors(errors) => errors,
            Self::AtFrame { cause, .. } => cause.build_errors(),
            _ => &[],
        }
    }

    /// Associates an original failure with a named resource load.
    pub fn asset(
        kind: &'static str,
        source_name: impl Into<String>,
        cause: impl StdError + Send + Sync + 'static,
    ) -> Self {
        Self::Asset {
            kind,
            source_name: source_name.into(),
            cause: Box::new(cause),
        }
    }

    /// Formula failure without structured diagnostics.
    pub fn formula(message: impl Into<String>) -> Self {
        Self::Formula {
            message: message.into(),
            source: None,
        }
    }

    /// Formula failure retaining its structured diagnostics as the source.
    pub fn formula_with_source(
        message: impl Into<String>,
        source: impl StdError + Send + Sync + 'static,
    ) -> Self {
        Self::Formula {
            message: message.into(),
            source: Some(Box::new(source)),
        }
    }

    /// Classifies an error independently of any frame annotation.
    pub fn kind(&self) -> ErrorKind {
        match self {
            Self::BuildErrors(errors) => errors
                .first()
                .map_or(ErrorKind::Validation, |error| error.kind()),
            Self::Validation(_) => ErrorKind::Validation,
            Self::Io(_) => ErrorKind::Io,
            Self::Asset { .. } => ErrorKind::Asset,
            Self::Svg(_) => ErrorKind::Svg,
            Self::Formula { .. } => ErrorKind::Formula,
            Self::Parameters(_) => ErrorKind::Parameters,
            Self::Timeline { .. } => ErrorKind::Timeline,
            Self::GpuUnavailable(_) => ErrorKind::GpuUnavailable,
            Self::Gpu(_) => ErrorKind::Gpu,
            Self::DeviceLost(_) => ErrorKind::DeviceLost,
            Self::ScenePanic { .. } => ErrorKind::ScenePanic,
            Self::Encoder(_) => ErrorKind::Encoder,
            Self::Cancelled => ErrorKind::Cancelled,
            Self::AtFrame { cause, .. } => cause.kind(),
            Self::Other(_) => ErrorKind::Other,
        }
    }

    /// Frame of failure, if supplied by rendering or a worker boundary.
    pub fn frame(&self) -> Option<u64> {
        match self {
            Self::Validation(error) => error.frame.map(u64::from),
            Self::ScenePanic { frame, .. } | Self::AtFrame { frame, .. } => Some(*frame),
            _ => None,
        }
    }

    /// Attaches a frame without replacing a more specific existing frame.
    /// Validation diagnostics keep their original concrete type and fields.
    pub fn at_frame(mut self, frame: u64) -> Self {
        if self.frame().is_some() {
            return self;
        }
        if let Self::Validation(error) = &mut self
            && let Ok(frame) = u32::try_from(frame)
        {
            error.frame = Some(frame);
            return self;
        }
        Self::AtFrame {
            frame,
            cause: Box::new(self),
        }
    }

    /// Finds an original concrete error through the complete source chain.
    /// This preserves property-diagnostic queries without parsing display strings.
    pub fn downcast_ref<T: StdError + 'static>(&self) -> Option<&T> {
        let mut current: Option<&(dyn StdError + 'static)> = Some(self);
        while let Some(error) = current {
            if let Some(error) = error.downcast_ref::<T>() {
                return Some(error);
            }
            current = error.source();
        }
        None
    }

    /// Classifies a caught scene panic while preserving its scene and frame.
    pub fn scene_panic(
        timeline: impl Into<String>,
        frame: u64,
        payload: Box<dyn std::any::Any + Send>,
    ) -> Self {
        let message = payload
            .downcast_ref::<String>()
            .cloned()
            .or_else(|| {
                payload
                    .downcast_ref::<&str>()
                    .map(|message| (*message).to_owned())
            })
            .unwrap_or_else(|| "non-string panic payload".to_owned());
        Self::ScenePanic {
            timeline: timeline.into(),
            frame,
            message,
        }
    }
}

impl fmt::Display for Error {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::BuildErrors(errors) => {
                write!(formatter, "{} authoring error(s)", errors.len())?;
                for error in errors {
                    write!(formatter, "\n  {error}")?;
                }
                Ok(())
            }
            Self::Validation(error) => error.fmt(formatter),
            Self::Io(error) => error.fmt(formatter),
            Self::Asset {
                kind,
                source_name,
                cause,
            } => write!(
                formatter,
                "{kind} asset `{source_name}` failed to load: {cause}"
            ),
            Self::Svg(cause) => write!(formatter, "SVG parse error: {cause}"),
            Self::Formula { message, .. } => write!(formatter, "formula compile error: {message}"),
            Self::Parameters(cause) => write!(formatter, "invalid parameters: {cause}"),
            Self::Timeline { timeline, message } => {
                write!(formatter, "timeline `{timeline}`: {message}")
            }
            Self::GpuUnavailable(cause) => write!(formatter, "GPU unavailable: {cause}"),
            Self::Gpu(cause) => write!(formatter, "GPU error: {cause}"),
            Self::DeviceLost(cause) => write!(formatter, "GPU device lost: {cause}"),
            Self::ScenePanic {
                timeline,
                frame,
                message,
            } => write!(
                formatter,
                "scene panicked at frame {frame} of `{timeline}`: {message}"
            ),
            Self::Encoder(cause) => write!(formatter, "encoder error: {cause}"),
            Self::Cancelled => formatter.write_str("export cancelled"),
            Self::AtFrame { frame, cause } => write!(formatter, "frame {frame}: {cause}"),
            Self::Other(cause) => cause.fmt(formatter),
        }
    }
}

impl StdError for Error {
    fn source(&self) -> Option<&(dyn StdError + 'static)> {
        match self {
            Self::Validation(error) => Some(error),
            Self::Io(error) => Some(error),
            Self::Asset { cause, .. }
            | Self::Svg(cause)
            | Self::Parameters(cause)
            | Self::GpuUnavailable(cause)
            | Self::Gpu(cause)
            | Self::DeviceLost(cause)
            | Self::Encoder(cause)
            | Self::Other(cause) => Some(cause.as_ref()),
            Self::Formula { source, .. } => source
                .as_deref()
                .map(|cause| cause as &(dyn StdError + 'static)),
            Self::AtFrame { cause, .. } => Some(cause.as_ref()),
            _ => None,
        }
    }
}

impl From<ValidationError> for Error {
    fn from(error: ValidationError) -> Self {
        if error.field == "formula" {
            Self::Formula {
                message: error.to_string(),
                source: Some(Box::new(error)),
            }
        } else {
            Self::Validation(error)
        }
    }
}

impl From<io::Error> for Error {
    fn from(error: io::Error) -> Self {
        Self::Io(error)
    }
}

impl From<BoxedError> for Error {
    fn from(error: BoxedError) -> Self {
        Self::Other(error)
    }
}

impl From<std::collections::TryReserveError> for Error {
    fn from(error: std::collections::TryReserveError) -> Self {
        Self::Io(io::Error::new(io::ErrorKind::OutOfMemory, error))
    }
}

impl From<std::env::VarError> for Error {
    fn from(error: std::env::VarError) -> Self {
        Self::Parameters(Box::new(error))
    }
}

impl From<std::str::Utf8Error> for Error {
    fn from(error: std::str::Utf8Error) -> Self {
        Self::Other(Box::new(error))
    }
}

impl From<std::sync::mpsc::RecvError> for Error {
    fn from(error: std::sync::mpsc::RecvError) -> Self {
        Self::Gpu(Box::new(error))
    }
}

impl From<std::array::TryFromSliceError> for Error {
    fn from(error: std::array::TryFromSliceError) -> Self {
        Self::Gpu(Box::new(error))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn classification_frames_and_sources_survive_annotation() {
        fn send_sync<T: Send + Sync>() {}
        send_sync::<Error>();
        let original = io::Error::new(io::ErrorKind::PermissionDenied, "destination denied");
        let error = Error::Encoder(Box::new(original)).at_frame(7).at_frame(9);
        assert_eq!(error.kind(), ErrorKind::Encoder);
        assert_eq!(error.frame(), Some(7));
        assert_eq!(
            error.downcast_ref::<io::Error>().unwrap().kind(),
            io::ErrorKind::PermissionDenied
        );
        assert!(
            error
                .to_string()
                .contains("frame 7: encoder error: destination denied")
        );
        let original = ValidationError::invalid("nodes[2]", "opacity", "invalid opacity");
        let error = Error::from(original).at_frame(11);
        let diagnostic = error.downcast_ref::<ValidationError>().unwrap();
        assert_eq!(diagnostic.frame, Some(11));
        assert_eq!(diagnostic.path, "nodes[2]");
        assert_eq!(error.frame(), Some(11));
        assert_eq!(error.kind(), ErrorKind::Validation);
    }

    #[test]
    fn panic_payloads_keep_scene_and_frame() {
        for payload in [
            Box::new("bad scene") as Box<dyn std::any::Any + Send>,
            Box::new("bad scene".to_owned()),
        ] {
            let error = Error::scene_panic("intro", 23, payload);
            assert_eq!(error.kind(), ErrorKind::ScenePanic);
            assert_eq!(error.frame(), Some(23));
            assert!(error.to_string().contains("intro"));
            assert!(error.to_string().contains("bad scene"));
        }
        assert!(
            Error::scene_panic("intro", 1, Box::new(42_u32))
                .to_string()
                .contains("non-string")
        );
        assert_eq!(Error::Cancelled.kind(), ErrorKind::Cancelled);
    }
}
