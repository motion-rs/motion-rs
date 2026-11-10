//! Film playback benchmarks with deterministic checksums and optional frame dumps.

use motion::prelude::*;
use motion::render::Snapshot;
use serde_json::json;
use std::hint::black_box;

#[expect(
    dead_code,
    reason = "this benchmark uses only part of the shared support module"
)]
#[path = "support.rs"]
mod support;

fn checksum(snapshot: &Snapshot) -> u64 {
    snapshot.pixels().iter().fold(0, |state, pixel| {
        support::fold_checksum(state, u64::from(*pixel))
    })
}

fn ppm(snapshot: &Snapshot) -> Vec<u8> {
    let mut bytes = format!("P6\n{} {}\n255\n", snapshot.width(), snapshot.height()).into_bytes();
    for pixel in snapshot.pixels() {
        bytes.extend_from_slice(&[(pixel >> 16) as u8, (pixel >> 8) as u8, *pixel as u8]);
    }
    bytes
}

#[cfg(feature = "gpu")]
fn texture_playback(
    timeline: &Timeline,
    options: &RenderOptions,
    frames: &[u32],
    settings: &support::Settings,
    report: &mut support::Report,
    name: &str,
) -> motion::Result {
    let composition = motion::export::composition(timeline, options)?;
    let mut renderer = composition.frame_renderer(timeline.size())?;
    let context = renderer
        .selected_gpu_context()?
        .expect("explicit GPU backend");
    let size = composition.configured_output_size();
    let texture = context.device().create_texture(&wgpu::TextureDescriptor {
        label: Some("Film native preview benchmark"),
        size: wgpu::Extent3d {
            width: size.width,
            height: size.height,
            depth_or_array_layers: 1,
        },
        mip_level_count: 1,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format: wgpu::TextureFormat::Rgba8Unorm,
        usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::COPY_SRC,
        view_formats: &[],
    });
    let mut submission = Vec::new();
    let mut completion = Vec::new();
    let mut consumed = 0;
    let measured = support::measure_frames(
        frames,
        |frame| {
            let started = std::time::Instant::now();
            let pending = renderer.submit_to_texture(
                frame,
                None,
                &texture,
                motion::render::PixelFormat::SrgbRgba8 {
                    premultiplied: true,
                },
                |context| timeline.render(context),
            )?;
            let submitted = started.elapsed();
            let waiting = std::time::Instant::now();
            pending.finish()?;
            Ok((frame, submitted, waiting.elapsed()))
        },
        |(frame, submitted, waited)| {
            if consumed >= settings.warmups {
                for (samples, duration) in [(&mut submission, submitted), (&mut completion, waited)]
                {
                    samples.push(support::Sample {
                        elapsed_ns: duration
                            .as_nanos()
                            .try_into()
                            .expect("bounded frame duration"),
                        iterations: 1,
                        checksum: format!("{frame:016x}"),
                        frame: Some(frame),
                    });
                }
            }
            consumed += 1;
            u64::from(frame)
        },
        settings,
    )?;
    let metrics = json!({
        "adapter":format!("{:?}", context.info()),
        "target":"premultiplied-rgba8-texture", "background":"transparent",
        "no_window_or_display_timing":true, "checksum":"frame-order-only; see raw texture references",
    });
    report.record(
        format!("Gpu/{name}/native-texture-playback"),
        measured,
        metrics.clone(),
    )?;
    report.record(
        format!("Gpu/{name}/native-prepare-and-submit"),
        support::summarize(submission, "frame-sequence")?,
        metrics.clone(),
    )?;
    report.record(
        format!("Gpu/{name}/native-completion-wait"),
        support::summarize(completion, "frame-sequence")?,
        metrics,
    )?;
    for frame in [24, 120, 216] {
        renderer
            .submit_to_texture(
                frame,
                None,
                &texture,
                motion::render::PixelFormat::SrgbRgba8 {
                    premultiplied: true,
                },
                |context| timeline.render(context),
            )?
            .finish()?;
        let pitch = (size.width * 4).div_ceil(256) * 256;
        let readback = context.device().create_buffer(&wgpu::BufferDescriptor {
            label: Some("Film native reference only"),
            size: u64::from(pitch) * u64::from(size.height),
            usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
            mapped_at_creation: false,
        });
        let mut encoder = context.device().create_command_encoder(&Default::default());
        encoder.copy_texture_to_buffer(
            wgpu::TexelCopyTextureInfo {
                texture: &texture,
                mip_level: 0,
                origin: wgpu::Origin3d::ZERO,
                aspect: wgpu::TextureAspect::All,
            },
            wgpu::TexelCopyBufferInfo {
                buffer: &readback,
                layout: wgpu::TexelCopyBufferLayout {
                    offset: 0,
                    bytes_per_row: Some(pitch),
                    rows_per_image: None,
                },
            },
            texture.size(),
        );
        context.queue().submit(Some(encoder.finish()));
        let (sender, receiver) = std::sync::mpsc::channel();
        readback
            .slice(..)
            .map_async(wgpu::MapMode::Read, move |result| {
                let _ = sender.send(result);
            });
        context
            .device()
            .poll(wgpu::PollType::Wait {
                submission_index: None,
                timeout: Some(std::time::Duration::from_secs(60)),
            })
            .map_err(|error| support::invalid(error.to_string()))?;
        receiver
            .recv()
            .map_err(|error| support::invalid(error.to_string()))?
            .map_err(|error| support::invalid(error.to_string()))?;
        let mapped = readback
            .slice(..)
            .get_mapped_range()
            .map_err(|error| support::invalid(error.to_string()))?;
        let pixels = mapped
            .chunks_exact(pitch as usize)
            .flat_map(|row| row[..size.width as usize * 4].iter().copied())
            .collect::<Vec<_>>();
        report.reference(&format!("Gpu-{name}-native-{frame}.rgba"), &pixels)?;
        drop(mapped);
        readback.unmap();
    }
    Ok(())
}

fn main() -> motion::Result {
    let mut settings = support::Settings::from_env()?;
    if std::env::var_os("MOTION_BENCH_ITERATIONS").is_none() {
        settings.iterations = 1;
    }
    let size = support::size((960, 540))?;
    let entries = motion_gallery::catalog();
    // Showcase films are deliberately excluded from benchmarks.
    let names = entries
        .iter()
        .map(|entry| entry.id)
        .filter(|id| !matches!(*id, "quiet-hours" | "chidori"))
        .collect::<Vec<_>>();
    let films = support::selected("MOTION_BENCH_CASES", &names)?;
    let backends = support::renderers(cfg!(feature = "gpu"))?;
    let sequences = std::env::var("MOTION_BENCH_SEQUENCES").unwrap_or_else(|_| "1".into()) == "1";
    let video = std::env::var("MOTION_BENCH_VIDEO").unwrap_or_default() == "1";
    let textures = std::env::var("MOTION_BENCH_TEXTURES").unwrap_or_default() == "1";
    if textures && (!cfg!(feature = "gpu") || !sequences || video || backends != [Renderer::Gpu]) {
        return Err(support::invalid(
            "Native texture mode requires GPU-only sequences",
        ));
    }
    if video && !cfg!(feature = "ffmpeg") {
        return Err(support::invalid("Film export benchmarks require ffmpeg"));
    }
    let sequence_frames = if sequences {
        support::sequence_length(240)?
    } else {
        0
    };
    let font = std::env::var_os("MOTION_BENCH_FONT")
        .ok_or_else(|| support::invalid("Film benchmarks require explicit MOTION_BENCH_FONT"))?;
    let font = std::path::PathBuf::from(font);
    let mut options = motion_gallery::render_options(Some(&font))?;
    options.size = Some(size);
    if textures {
        options.quality = match std::env::var("MOTION_BENCH_QUALITY")
            .as_deref()
            .unwrap_or("draft")
        {
            "draft" => motion::RenderQuality::Draft,
            "final" => motion::RenderQuality::Final,
            _ => {
                return Err(support::invalid(
                    "Native texture quality must be draft or final",
                ));
            }
        };
    }
    let project = motion_gallery::project(options.clone())?;
    let parameters = json!({"seconds":8,"seed":20261027_u64,"count":72});
    let mut artwork = std::collections::BTreeMap::new();
    for (name, bytes) in [
        (
            "data_story",
            include_bytes!("../../../gallery/src/demos/data_story.rs").as_slice(),
        ),
        (
            "type_in_motion",
            include_bytes!("../../../gallery/src/demos/type_in_motion.rs").as_slice(),
        ),
        (
            "ink_journey",
            include_bytes!("../../../gallery/src/demos/ink_journey.rs").as_slice(),
        ),
        (
            "signal_garden",
            include_bytes!("../../../gallery/src/demos/signal_garden.rs").as_slice(),
        ),
    ] {
        artwork.insert(name, support::fingerprint(bytes));
    }
    #[cfg(feature = "formula")]
    artwork.insert(
        "harmonics",
        support::fingerprint(include_bytes!("../../../gallery/src/demos/harmonics.rs")),
    );
    let mut configuration = json!({
        "size":[size.width,size.height],"design":[960,540],"cases":films,"parameters":parameters,
        "frames":[24,120,216],"frame_rate":[30,1],
        "sequence_frames":sequence_frames,"sequence_sessions":"warm-process-fresh-session","video":video,
        "backends":backends.iter().map(|value|format!("{value:?}")).collect::<Vec<_>>(),
        "limits":{"bytes":options.limits.max_bytes,"work":options.limits.max_work,"ink_material_work":100_000_000},
        "font":support::fingerprint(&std::fs::read(&font)?),"artwork":artwork,
        "features":{"gpu":cfg!(feature="gpu"),"formula":cfg!(feature="formula")},
    });
    if textures {
        configuration["native_textures"] = json!(true);
        configuration["quality"] = json!(format!("{:?}", options.quality));
    }
    let mut report = support::Report::new("films", settings.clone(), configuration)?;
    for film in &films {
        let construction = support::measure_output(
            || project.build(film, Some(parameters.clone())),
            |timeline| black_box(timeline.frame_count()),
            &settings,
            1,
        )?;
        report.record(format!("setup/{film}/timeline"),construction,json!({"includes_audio_analysis":film=="signal-garden","global_caches_may_be_warm":true}))?;
        let timeline = project.build(film, Some(parameters.clone()))?;
        for backend in &backends {
            let mut selected = options.clone();
            selected.backend = if *backend == Renderer::Gpu {
                BackendPreference::Gpu
            } else {
                BackendPreference::Cpu
            };
            #[cfg(feature = "gpu")]
            if textures {
                texture_playback(
                    &timeline,
                    &selected,
                    &support::frame_order(sequence_frames, false),
                    &settings,
                    &mut report,
                    film,
                )?;
                continue;
            }
            let composition = motion::export::composition(&timeline, &selected)?;
            let mut session = composition.frame_renderer(timeline.size())?;
            for frame in [24, 120, 216] {
                let expected = session.render_frame(frame, |context| timeline.render(context))?;
                session.render_frame(0, |context| timeline.render(context))?;
                let replay = session.render_frame(frame, |context| timeline.render(context))?;
                if expected.pixels() != replay.pixels() {
                    return Err(support::invalid(format!(
                        "Film replay differs: {film}/{frame}"
                    )));
                }
                report.reference(&format!("{backend:?}-{film}-{frame}.ppm"), &ppm(&expected))?;
                let measured = support::measure_output(
                    || session.render_frame(frame, |context| timeline.render(context)),
                    |snapshot| checksum(&snapshot),
                    &settings,
                    settings.iterations,
                )?;
                let stats = *session.last_stats().expect("successful frame stats");
                report.record(format!("{backend:?}/{film}/frame-{frame}"),measured,json!({
                    "adapter":format!("{:?}",session.gpu_info()),"peak_bytes":stats.peak_bytes(),"work":stats.work(),
                    "prepare_peak_bytes":stats.preparation.peak_bytes,"prepare_retained_bytes":stats.preparation.retained_bytes,
                    "prepare_work":stats.preparation.work,"linear_work":stats.linear.work,
                    "linear_peak_bytes":stats.linear.peak_bytes,"output_work":stats.output.work,"pack_work":stats.snapshot.work,
                }))?;
            }
            if sequences {
                for shuffled in [false, true] {
                    let frames = support::frame_order(sequence_frames, shuffled);
                    let mut playback = composition.frame_renderer(timeline.size())?;
                    let measured = support::measure_frames(
                        &frames,
                        |frame| playback.render_frame(frame, |context| timeline.render(context)),
                        |snapshot| checksum(&snapshot),
                        &settings,
                    )?;
                    report.record(format!("{backend:?}/{film}/{}",if shuffled {"random-seek"}else{"sequential-playback"}),measured,json!({"adapter":format!("{:?}",playback.gpu_info()),"complete_timeline":sequence_frames==240}))?;
                }
            }
            #[cfg(feature = "ffmpeg")]
            if video {
                let measured = support::measure(
                    || {
                        let temporary = tempfile::tempdir()?;
                        let output = temporary.path().join("film.mp4");
                        let mut export_options = ExportOptions::new(&output);
                        export_options.render = selected.clone();
                        let completed = export(&timeline, &export_options, &NoProgress)?;
                        if completed.frames != timeline.frame_count() {
                            return Err(support::invalid("Incomplete film export benchmark"));
                        }
                        Ok(std::fs::metadata(output)?.len())
                    },
                    &settings,
                    1,
                )?;
                report.record(
                    format!("{backend:?}/{film}/export"),
                    measured,
                    json!({"frames":timeline.frame_count(),"includes_render_codec_filesystem":true,
                        "codec":"h264","fps_basis":"complete_frames / elapsed_seconds"}),
                )?;
            }
        }
    }
    report.finish()?;
    Ok(())
}
