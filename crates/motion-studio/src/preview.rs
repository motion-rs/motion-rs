//! Preview/export worker ownership and bounded cross-thread messaging.
//!
//! GPU sessions stay on the rendering worker. replies carry same-device
//! textures or transparent CPU images; export shares progress and cancellation.

use std::io;
use std::panic::{AssertUnwindSafe, catch_unwind};
#[cfg(test)]
use std::path::PathBuf;
use std::sync::{Arc, Condvar, Mutex};
use std::thread;

#[cfg(any(test, feature = "gpu"))]
use eframe::egui::Color32;
use eframe::egui::{ColorImage, Context};

use super::StudioScene;
use motion_render::{BackendKind, FrameRenderer};

mod pool;
#[cfg(feature = "gpu")]
mod textures;
mod worker;

pub(crate) use super::export::{ExportForm, ExportJob, JobState};
pub(crate) use pool::PreviewCoordinator;

#[cfg(feature = "gpu")]
pub(crate) use textures::MAX_SLOTS as MAX_TEXTURE_SLOTS;

#[cfg(feature = "gpu")]
pub(crate) use textures::FrameTexture;

#[cfg(test)]
use crate::AudioOptions;
#[cfg(test)]
use motion_render::FrameBuffer;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct PreviewKey {
    pub frame: u32,
    pub renderer: BackendKind,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct PreviewRequest {
    generation: u64,
    key: PreviewKey,
    settings: PreviewSettings,
    ahead: u8,
}

/// What the UI asked the preview workers for and what it currently shows. The displayed frame can
/// lag behind the request while a newer one is pending.
#[derive(Default)]
pub(crate) struct PreviewState {
    request: Option<(PreviewKey, PreviewSettings)>,
    pub(crate) pending: bool,
    pub(crate) displayed: Option<PreviewKey>,
    pub(crate) error: Option<String>,
}

impl PreviewState {
    pub(crate) fn is_requested(&self, key: PreviewKey, settings: PreviewSettings) -> bool {
        self.request == Some((key, settings))
    }

    pub(crate) fn requested_key(&self) -> Option<PreviewKey> {
        self.request.map(|(key, _)| key)
    }

    pub(crate) fn request(&mut self, key: PreviewKey, settings: PreviewSettings) {
        self.request = Some((key, settings));
        self.pending = true;
        self.error = None;
    }

    pub(crate) fn cancel_request(&mut self) {
        self.request = None;
        self.pending = false;
    }

    /// The displayed frame is the requested one, so `error` describes what is on screen.
    pub(crate) fn displays_request(&self) -> bool {
        self.displayed == self.requested_key()
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct PreviewSettings {
    pub quality: motion_core::RenderQuality,
    pub scale: u32,
    pub transparent: bool,
    pub backend: Option<motion_render::BackendPreference>,
    pub inspect: bool,
    pub pixels: bool,
    pub idle: bool,
}

impl Default for PreviewSettings {
    fn default() -> Self {
        Self {
            quality: motion_core::RenderQuality::Final,
            scale: 100,
            transparent: false,
            backend: None,
            inspect: false,
            pixels: false,
            idle: false,
        }
    }
}

impl PreviewSettings {
    pub(crate) fn for_interaction(
        mut self,
        playing: bool,
        dragging: bool,
        age: std::time::Duration,
    ) -> Self {
        self.quality = if playing || dragging || age < std::time::Duration::from_millis(150) {
            motion_core::RenderQuality::Draft
        } else {
            motion_core::RenderQuality::Final
        };
        if dragging {
            self.scale = 25;
        }
        self.transparent = true;
        self.idle = false;
        self
    }
}

pub(crate) struct PreviewReply {
    pub key: PreviewKey,
    pub image: motion_core::Result<Arc<ColorImage>>,
    pub nodes: Arc<[motion_core::authoring::NodeInspection]>,
    #[cfg(feature = "gpu")]
    pub texture: Option<FrameTexture>,
    pub backend: String,

    pub cached_frames: Vec<u32>,
    pub cached_bytes: usize,

    pub elapsed: std::time::Duration,
    pub cache_hit: bool,
    pub stats: Option<motion_render::FrameStats>,
    pub timings: Option<motion_render::composition::FrameTimings>,
    pub diagnostics: Vec<Arc<motion_core::Error>>,
}

#[derive(Default)]
struct PreviewRequestQueue {
    generation: u64,
    pending: Option<PreviewRequest>,
    #[cfg(feature = "gpu")]
    texture_reservation: Option<textures::TextureReservation>,
    reply: Option<PreviewReply>,
    stopping: bool,
}

impl PreviewRequestQueue {
    #[cfg(test)]
    fn request(&mut self, key: PreviewKey) {
        self.request_with_settings(key, PreviewSettings::default());
    }

    fn request_with_settings(&mut self, key: PreviewKey, settings: PreviewSettings) {
        // Replace pending work, not an unbounded FIFO; seeking should display the latest request.
        self.generation = self.generation.wrapping_add(1);
        self.pending = Some(PreviewRequest {
            generation: self.generation,
            key,
            settings,
            ahead: 0,
        });
        #[cfg(feature = "gpu")]
        {
            self.texture_reservation = None;
        }
        self.reply = None;
    }

    #[cfg(test)]
    fn publish(
        &mut self,
        request: PreviewRequest,
        image: motion_core::Result<Arc<ColorImage>>,
    ) -> bool {
        self.publish_reply(request, image).is_some()
    }

    fn publish_reply(
        &mut self,
        request: PreviewRequest,
        image: motion_core::Result<Arc<ColorImage>>,
    ) -> Option<&mut PreviewReply> {
        // A frame may finish after a new seek; never let that obsolete result replace the preview.
        if self.stopping || request.generation != self.generation {
            return None;
        }
        self.reply = Some(PreviewReply {
            key: request.key,
            image,
            nodes: Arc::from([]),
            #[cfg(feature = "gpu")]
            texture: None,
            backend: String::new(),
            cached_frames: Vec::new(),
            cached_bytes: 0,
            elapsed: std::time::Duration::ZERO,
            cache_hit: false,
            stats: None,
            timings: None,
            diagnostics: Vec::new(),
        });
        self.reply.as_mut()
    }
}

struct PreviewRenderer {
    /// Frames may stay on the device and reach the UI as textures only when the scene was handed
    /// the UI's own device. A private device cannot share textures with it.
    #[cfg(feature = "gpu")]
    native_textures: bool,
    #[cfg(feature = "gpu")]
    textures: textures::TextureRing,
    renderer: FrameRenderer,
    inspection_error: Option<Arc<motion_core::Error>>,
}

impl PreviewRenderer {
    fn new(scene: &StudioScene, renderer: BackendKind) -> motion_core::Result<Self> {
        let mut frame_renderer = scene
            .composition
            .clone()
            .configure(|options| {
                options
                    .backend(motion_render::BackendPreference::from_kind(renderer))
                    .allow_missing_assets(true)
                    .mark_invalid_geometry(true)
            })
            .frame_renderer(scene.timeline.size())?;
        frame_renderer.enable_timings(true);
        Ok(Self {
            #[cfg(feature = "gpu")]
            native_textures: scene
                .composition
                .studio_render_options(motion_render::BackendPreference::default())
                .gpu_context
                .is_some(),
            #[cfg(feature = "gpu")]
            textures: textures::TextureRing::default(),
            renderer: frame_renderer,
            inspection_error: None,
        })
    }

    #[cfg(test)]
    fn render_frame(
        &mut self,
        scene: &StudioScene,
        index: u32,
    ) -> motion_core::Result<FrameBuffer> {
        self.renderer
            .render_frame_sampled(index, |context| scene.timeline.render(context))
    }

    fn diagnostics(&self) -> Vec<Arc<motion_core::Error>> {
        let mut diagnostics = self.renderer.asset_diagnostics().to_vec();
        diagnostics.extend(self.inspection_error.iter().cloned());
        diagnostics
    }

    #[cfg(all(test, feature = "gpu"))]
    fn preview(
        &mut self,
        scene: &StudioScene,
        index: u32,
        settings: PreviewSettings,
    ) -> motion_core::Result<PreviewImage> {
        self.begin_preview(scene, index, settings)?.finish()
    }

    fn begin_preview(
        &mut self,
        scene: &StudioScene,
        index: u32,
        settings: PreviewSettings,
    ) -> motion_core::Result<PreviewSubmission> {
        self.inspection_error = None;
        let timeline = &scene.timeline;
        let original = scene.composition.output_size();
        let width = ((u64::from(original.width) * u64::from(settings.scale)) / 100).max(1) as u32;
        let height = ((u64::from(original.height) * u64::from(settings.scale)) / 100).max(1) as u32;
        let background = if settings.transparent {
            None
        } else {
            let color = scene.composition.snapshot_background();
            Some(color.with_alpha(1.0))
        };
        let nodes = std::cell::RefCell::new(Vec::new());
        let inspection_error = std::cell::RefCell::new(None);
        let inspect_build = |context: &motion_core::SceneContext<'_>| {
            let node = timeline.render(context);
            if settings.inspect {
                let drawing = motion_core::DrawContext::default()
                    .with_quality(settings.quality)
                    .with_render_cache(
                        scene
                            .composition
                            .studio_render_options(motion_render::BackendPreference::default())
                            .cache,
                    )
                    .with_assets(context.assets())
                    .with_time(context.time());
                match node.inspect_with_context(
                    scene.timeline.size(),
                    motion_core::PixelSize::new(width, height),
                    motion_core::ValidationLimits::default(),
                    &drawing,
                ) {
                    Ok(snapshot) => *nodes.borrow_mut() = snapshot,
                    Err(error) => {
                        *inspection_error.borrow_mut() = Some(Arc::new(
                            motion_core::Error::from(error).at_frame(u64::from(index)),
                        ))
                    }
                }
            }
            node
        };
        #[cfg(feature = "gpu")]
        if !settings.pixels
            && self.native_textures
            && let Some(context) = self.renderer.selected_gpu_context()?
            && let Some(texture) = self.textures.lease(&context, width, height)
        {
            let submitted = std::time::Instant::now();
            let result = self.renderer.submit_to_texture(
                index,
                background,
                &texture,
                motion_render::PixelFormat::SrgbRgba8 {
                    premultiplied: true,
                },
                inspect_build,
            );
            let pending = result?;
            self.inspection_error = inspection_error.into_inner();
            return Ok(PreviewSubmission::Gpu(Box::new(PendingPreview {
                backend: format!("GPU ({})", context.info().name),
                context,
                texture,
                pending,
                nodes: nodes.into_inner().into(),
                stats: self.renderer.last_stats().copied(),
                timings: self.renderer.timings().map(|mut timings| {
                    timings.submission = submitted.elapsed();
                    timings
                }),
            })));
        }
        let frame = self.renderer.render_output_at(
            motion_core::Time::from_frame(i64::from(index), scene.composition.frame_rate()),
            background,
            motion_render::PixelFormat::SrgbRgba8 {
                premultiplied: !settings.pixels,
            },
            inspect_build,
        )?;
        self.inspection_error = inspection_error.into_inner();
        #[cfg(feature = "gpu")]
        let mut upload = std::time::Duration::ZERO;
        #[cfg(not(feature = "gpu"))]
        let upload = std::time::Duration::ZERO;
        #[cfg(feature = "gpu")]
        let uploaded = if !settings.pixels && self.native_textures {
            self.renderer.shared_gpu_context().and_then(|context| {
                let texture = self.textures.lease(&context, width, height)?;
                let started = std::time::Instant::now();
                context.queue().write_texture(
                    wgpu::TexelCopyTextureInfo {
                        texture: &texture,
                        mip_level: 0,
                        origin: wgpu::Origin3d::ZERO,
                        aspect: wgpu::TextureAspect::All,
                    },
                    frame.planes()[0].bytes(),
                    wgpu::TexelCopyBufferLayout {
                        offset: 0,
                        bytes_per_row: Some(width * 4),
                        rows_per_image: Some(height),
                    },
                    wgpu::Extent3d {
                        width,
                        height,
                        depth_or_array_layers: 1,
                    },
                );
                upload = started.elapsed();
                Some(texture)
            })
        } else {
            None
        };
        Ok(PreviewSubmission::Ready(Box::new(PreviewImage {
            backend: format!("{:?}", self.renderer.backend_kind()),
            image: Arc::new(if settings.pixels {
                ColorImage::from_rgba_unmultiplied(
                    [width as usize, height as usize],
                    frame.planes()[0].bytes(),
                )
            } else {
                ColorImage::from_rgba_premultiplied(
                    [width as usize, height as usize],
                    frame.planes()[0].bytes(),
                )
            }),
            nodes: nodes.into_inner().into(),
            stats: self.renderer.last_stats().copied(),
            timings: self.renderer.timings().map(|mut timings| {
                timings.upload = upload;
                timings
            }),
            #[cfg(feature = "gpu")]
            texture: uploaded,
        })))
    }
}

enum PreviewSubmission {
    Ready(Box<PreviewImage>),
    #[cfg(feature = "gpu")]
    Gpu(Box<PendingPreview>),
}

#[cfg(feature = "gpu")]
struct PendingPreview {
    backend: String,
    context: crate::GpuContext,
    texture: FrameTexture,
    pending: motion_render::gpu::PendingTextureFrame,
    nodes: Arc<[motion_core::authoring::NodeInspection]>,
    stats: Option<motion_render::FrameStats>,
    timings: Option<motion_render::composition::FrameTimings>,
}

impl PreviewSubmission {
    fn is_ready(&mut self) -> motion_core::Result<bool> {
        match self {
            Self::Ready(_) => Ok(true),
            #[cfg(feature = "gpu")]
            Self::Gpu(preview) => preview.pending.is_ready(),
        }
    }

    fn finish(self) -> motion_core::Result<PreviewImage> {
        match self {
            Self::Ready(image) => Ok(*image),
            #[cfg(feature = "gpu")]
            Self::Gpu(preview) => {
                let PendingPreview {
                    backend,
                    context,
                    texture,
                    pending,
                    nodes,
                    stats,
                    mut timings,
                } = *preview;
                let gpu_timings = pending.timing_handle();
                let completed = std::time::Instant::now();
                pending.finish()?;
                if let Some(timings) = &mut timings {
                    timings.readback += completed.elapsed();
                    timings.gpu = gpu_timings.and_then(|handle| handle.get());
                }
                #[cfg(test)]
                let image = texture_image_for_test(&context, &texture)?;
                #[cfg(not(test))]
                let image = {
                    let _ = context;
                    ColorImage::filled([1, 1], Color32::TRANSPARENT)
                };
                Ok(PreviewImage {
                    backend,
                    image: Arc::new(image),
                    nodes,
                    stats,
                    timings,
                    texture: Some(texture),
                })
            }
        }
    }
}

#[derive(Clone)]
struct PreviewImage {
    backend: String,
    image: Arc<ColorImage>,
    nodes: Arc<[motion_core::authoring::NodeInspection]>,
    stats: Option<motion_render::FrameStats>,
    timings: Option<motion_render::composition::FrameTimings>,
    #[cfg(feature = "gpu")]
    texture: Option<FrameTexture>,
}

impl PreviewImage {
    fn metadata_bytes() -> usize {
        std::mem::size_of::<(PreviewKey, PreviewSettings, Self, usize)>()
            + std::mem::size_of::<ColorImage>()
            + 2 * std::mem::size_of::<usize>()
    }

    fn retained_bytes(&self) -> usize {
        let metadata = Self::metadata_bytes();
        let bytes = self
            .image
            .pixels
            .len()
            .saturating_mul(4)
            .saturating_add(metadata)
            .saturating_add(self.backend.len());
        let bytes = bytes.saturating_add(
            self.nodes
                .iter()
                .map(|node| {
                    std::mem::size_of_val(node)
                        + node.path.len()
                        + node.filters.len() * std::mem::size_of::<motion_core::Filter>()
                })
                .sum::<usize>(),
        );
        #[cfg(feature = "gpu")]
        let bytes = bytes.saturating_add(self.texture.as_ref().map_or(0, |texture| {
            (u64::from(texture.width()) * u64::from(texture.height()) * 4).min(usize::MAX as u64)
                as usize
        }));
        bytes
    }
}

fn preview_cache_bytes(value: Option<&str>) -> io::Result<usize> {
    let mib = value
        .map_or(Ok(256usize), str::parse::<usize>)
        .map_err(|_| {
            io::Error::new(
                io::ErrorKind::InvalidInput,
                "MOTION_STUDIO_CACHE_MIB must be 0..=1024",
            )
        })?;
    if mib > 1024 {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "MOTION_STUDIO_CACHE_MIB must be 0..=1024",
        ));
    }
    Ok(mib * 1024 * 1024)
}

#[cfg(all(test, feature = "gpu"))]
fn texture_image_for_test(
    context: &crate::GpuContext,
    texture: &wgpu::Texture,
) -> motion_core::Result<ColorImage> {
    let pitch = (texture.width() * 4).div_ceil(256) * 256;
    let buffer = context.device().create_buffer(&wgpu::BufferDescriptor {
        label: Some("Studio acceptance-only texture readback"),
        size: u64::from(pitch) * u64::from(texture.height()),
        usage: wgpu::BufferUsages::MAP_READ | wgpu::BufferUsages::COPY_DST,
        mapped_at_creation: false,
    });
    let mut encoder = context.device().create_command_encoder(&Default::default());
    encoder.copy_texture_to_buffer(
        wgpu::TexelCopyTextureInfo {
            texture,
            mip_level: 0,
            origin: wgpu::Origin3d::ZERO,
            aspect: wgpu::TextureAspect::All,
        },
        wgpu::TexelCopyBufferInfo {
            buffer: &buffer,
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
    buffer
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
        .map_err(|error| io::Error::other(error.to_string()))?;
    receiver
        .recv()
        .map_err(|error| io::Error::other(error.to_string()))?
        .map_err(|error| io::Error::other(error.to_string()))?;
    let mapped = buffer
        .slice(..)
        .get_mapped_range()
        .map_err(|error| io::Error::other(error.to_string()))?;
    let bytes = mapped
        .chunks_exact(pitch as usize)
        .flat_map(|row| row[..texture.width() as usize * 4].iter().copied())
        .collect::<Vec<_>>();
    let image = ColorImage::from_rgba_premultiplied(
        [texture.width() as usize, texture.height() as usize],
        &bytes,
    );
    drop(mapped);
    buffer.unmap();
    Ok(image)
}

#[cfg(test)]
mod tests;
