//! Video export benchmark for the performance target, measuring end-to-end encoding of a
//! short animated composition.

use super::artwork::rectangle;
use super::{invalid, support};
use motion::prelude::*;
use motion::render::RasterLimits;

pub(super) fn video_benchmark(
    report: &mut support::Report,
    settings: &support::Settings,
    size: PixelSize,
    frames: u32,
    renderers: &[Renderer],
    limits: RasterLimits,
) -> motion::Result {
    use motion::authoring::Scene as _;
    use serde_json::json;
    let timeline = Timeline::try_new(Size::new(320.0, 180.0), 30)?.try_track(|track| {
        track.try_push(
            (|context: &SceneContext<'_>| {
                rectangle(
                    Rect::new(
                        16.0 + context.seconds() * 240.0,
                        32.0,
                        96.0 + context.seconds() * 240.0,
                        128.0,
                    ),
                    Color::BLACK,
                )
            })
            .with_duration(Time::from_frame(i64::from(frames), FrameRate::from(30))),
        )
    })?;
    for &renderer in renderers {
        let measured = support::measure(
            || {
                let temporary = tempfile::tempdir()?;
                let output = temporary.path().join("sample.mp4");
                let mut options =
                    ExportOptions::new(&output)
                        .size(size)
                        .backend(if renderer == Renderer::Gpu {
                            motion::BackendPreference::gpu()
                        } else {
                            motion::BackendPreference::Cpu
                        });
                options.render.limits = limits;
                let completed = export(&timeline, &options, &NoProgress)?;
                if completed.frames != u64::from(frames) {
                    return Err(invalid("Incomplete video benchmark"));
                }
                Ok(std::fs::metadata(output)?.len())
            },
            settings,
            1,
        )?;
        report.record(
        format!("{renderer:?}/video/export{frames}"),
        measured,
        json!({"includes_render_codec_filesystem":true,"size":[size.width,size.height],"frames":frames,"codec":"h264"}),
    )?;
    }
    Ok(())
}
