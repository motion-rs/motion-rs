//! Benchmark replay checks using exact CPU and visual GPU acceptance.

#[cfg(feature = "gpu")]
use motion::render::PixelFormat;
use motion::render::{BackendKind, FrameBuffer};

#[cfg(feature = "gpu")]
use super::visual;

pub(super) fn verify(
    first: &FrameBuffer,
    replay: &FrameBuffer,
    backend: BackendKind,
) -> Result<(), String> {
    if first.size() != replay.size() || first.format() != replay.format() {
        return Err("frame size or format changed".into());
    }
    match backend {
        BackendKind::Cpu => {
            if first.packed_rgb() == replay.packed_rgb() {
                Ok(())
            } else {
                Err("CPU output is not byte-identical".into())
            }
        }
        BackendKind::Gpu => {
            #[cfg(feature = "gpu")]
            {
                if first.format()
                    != (PixelFormat::SrgbRgba8 {
                        premultiplied: false,
                    })
                {
                    return Err("GPU replay requires straight RGBA8 output".into());
                }
                let expected = first.bytes().as_chunks::<4>();
                let actual = replay.bytes().as_chunks::<4>();
                if !expected.1.is_empty() || !actual.1.is_empty() {
                    return Err("GPU replay contains incomplete RGBA pixels".into());
                }
                visual::compare_rgba8(
                    actual.0,
                    expected.0,
                    first.size().width as usize,
                    visual::VisualTolerance::REPLAY,
                )
                .map(|_| ())
                .map_err(|error| error.to_string())
            }
            #[cfg(not(feature = "gpu"))]
            {
                Err("GPU replay requires the gpu feature".into())
            }
        }
    }
}
