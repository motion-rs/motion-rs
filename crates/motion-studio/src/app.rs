//! Optional egui workbench. Scene rendering and export remain in the headless core.
//!
//! Enable Cargo feature `studio` to create a [`StudioScene`] and call
//! [`run`] from the main thread. A native desktop and compatible GPU are required
//! for the workbench even when composition rendering uses CPU. Feature
//! `gpu` enables scene GPU composition.
//!
//! Scene builders execute on background workers, not the UI thread. They must be
//! deterministic and thread-safe; changing compiled Rust code requires rebuilding.
//! A scene registered with [`StudioScene::reloadable`] is rebuilt in place when its
//! watched files change.

use std::io;
use std::sync::Arc;
#[cfg(test)]
use std::time::Duration;
use std::time::Instant;

use crate::audio::preview::AudioPreview;
use crate::audio::settings::AudioForm;
use crate::playback::{StudioClock, Transport};
use crate::preview;
use crate::preview::PreviewCoordinator;
#[cfg(test)]
use crate::session::{read_session, write_session};
use eframe::egui::{self, RichText};
use motion_core::Result;
use motion_render::Renderer;

mod reload;
mod scene;
#[cfg(test)]
mod snapshot;
mod state;
#[cfg(test)]
mod tests;
mod workers;

pub use scene::StudioScene;
pub(crate) use state::{DiagnosticsState, ExportState, InspectorState, PlaybackState};

pub(crate) struct StudioApp {
    pub(crate) scene: StudioScene,
    pub(crate) clock: StudioClock,

    pub(crate) renderer: Renderer,
    pub(crate) gpu_available: bool,
    pub(crate) preview_settings: preview::PreviewSettings,

    pub(crate) playback: PlaybackState,
    pub(crate) inspection: InspectorState,
    pub(crate) preview_state: preview::PreviewState,

    pub(crate) preview: PreviewCoordinator,
    pub(crate) texture: Option<egui::TextureHandle>,
    pub(crate) preview_pixels: Option<Arc<egui::ColorImage>>,

    #[cfg(feature = "gpu")]
    pub(crate) preview_only: Option<(egui::TextureId, crate::preview::FrameTexture)>,
    #[cfg(feature = "gpu")]
    pub(crate) registered_previews: std::collections::VecDeque<(u64, egui::TextureId)>,
    #[cfg(feature = "gpu")]
    pub(crate) register_texture: Option<Box<TextureRegistrar>>,
    #[cfg(feature = "gpu")]
    pub(crate) free_texture: Option<Box<dyn Fn(egui::TextureId)>>,

    pub(crate) audio: AudioForm,
    pub(crate) audio_preview: AudioPreview,
    pub(crate) exports: ExportState,

    pub(crate) cached_frames: Vec<u32>,
    pub(crate) cached_previews: Vec<(u32, Arc<egui::ColorImage>)>,
    pub(crate) thumbnail: Option<(u32, egui::TextureHandle)>,
    pub(crate) cached_bytes: usize,
    pub(crate) observations: DiagnosticsState,

    pub(crate) reload: Option<reload::ReloadState>,
    pub(crate) reload_status: Option<(bool, String)>,

    pub(crate) close_when_idle: bool,
    persist_session: bool,
}

#[cfg(feature = "gpu")]
type TextureRegistrar = dyn Fn(&wgpu::Texture) -> egui::TextureId;

/// Opens the Studio on the main thread. The selected backend applies to
/// preview and export.
/// The UI itself is drawn by eframe, not by the scene renderer.
///
/// Blocks in the event loop until the window closes. Preview coalesces seek
/// requests and may skip frames; exports render every frame on a separate worker.
/// Closing during an active export requests cancellation and waits for cleanup.
/// Scene panics are reported by worker error states rather than propagated to UI.
///
/// # Errors
/// Returns an error for invalid composition timing/dimensions,
/// worker startup failures, or native window/GPU initialization failures. Errors
/// encountered after launch (fonts, scenes, export) are shown in the workbench.
///
/// # Example
///
/// ```no_run
/// use motion_studio::prelude::*;
/// use motion_core::prelude::*;
/// use motion_core::{
///     DisplayList, RasterLimits, ResourceStore, ValidatedDisplayList, MAX_PATH_VERBS,
///     MAX_STROKE_VERBS, MAX_SVG_PATH_BYTES, sample_offsets,
/// };
/// use motion_render::{Composition, RenderPrepare, RenderOptions, Renderer, render_still};
/// use motion_studio::{self as studio, StudioScene};
/// # fn main() -> motion_core::Result {
/// let timeline = Timeline::try_new(Size::new(640.0, 360.0), fps!(30))?
///     .try_track(|track| {
///         track.try_push(
///             (|context: &SceneContext<'_>| {
///                 Node::from(
///                     Rect::new(20.0, 40.0, 20.0 + 400.0 * context.progress(), 52.0)
///                         .fill(Color::rgb(20, 150, 125)),
///                 )
///             })
///             .with_duration(3.seconds()),
///         )
///     })?
///     .build()?;
/// studio::run(StudioScene::new(
///     "Study",
///     Composition::new(640, 360, motion_render::FrameClock::new(fps!(30), 90)),
///     timeline,
/// )?)
/// # }
/// ```
pub fn run(scene: StudioScene) -> Result {
    let frame = validate_scene(&scene)?;
    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_title(&scene.name)
            .with_inner_size([1240.0, 800.0])
            .with_min_inner_size([900.0, 620.0]),
        renderer: eframe::Renderer::Wgpu,
        ..Default::default()
    };
    eframe::run_native(
        "Motion Studio",
        options,
        Box::new(move |context| Ok(Box::new(StudioApp::new(context, scene, frame)?))),
    )
    .map_err(|error| io::Error::other(error.to_string()))?;
    Ok(())
}

/// Checks the clock and output dimensions that launch and reload both require.
pub(crate) fn validate_scene(scene: &StudioScene) -> Result<StudioClock> {
    let frame = StudioClock::new(&scene.composition)?;
    if frame.width() == 0 || frame.height() == 0 || frame.width() > 16384 || frame.height() > 16384
    {
        return Err(
            io::Error::other("Studio dimensions must be between 1 and 16384 pixels").into(),
        );
    }
    motion_core::PixelSize::new(frame.width() as u32, frame.height() as u32)
        .mapping(scene.timeline.size())?;
    Ok(frame)
}

/// Frame-boundary alignment notes of a scene's timeline.
pub(crate) fn alignment_diagnostics(
    scene: &StudioScene,
    clock: StudioClock,
) -> std::collections::VecDeque<(u32, String)> {
    scene
        .timeline
        .diagnostics()
        .boundary_adjustments()
        .take(128)
        .map(|adjustment| {
            let frame = adjustment
                .frame
                .clamp(0, i64::from(clock.frames.saturating_sub(1))) as u32;
            (
                frame,
                format!(
                    "Frame-boundary alignment: track {:?}, clip {:?}, {:?}: {:?} -> {:?}",
                    adjustment.track,
                    adjustment.clip,
                    adjustment.edge,
                    adjustment.requested,
                    adjustment.aligned
                ),
            )
        })
        .collect()
}

impl StudioApp {
    fn new(
        context: &eframe::CreationContext<'_>,
        scene: StudioScene,
        clock: StudioClock,
    ) -> io::Result<Self> {
        Self::with_persistence(context, scene, clock, true)
    }

    fn with_persistence(
        context: &eframe::CreationContext<'_>,
        scene: StudioScene,
        clock: StudioClock,
        persist_session: bool,
    ) -> io::Result<Self> {
        context.egui_ctx.set_fonts(crate::theme::font_definitions());
        crate::theme::apply(&context.egui_ctx, true);
        #[cfg(feature = "gpu")]
        let mut scene = scene;
        // Rendering on the UI's device is opt-in: while the UI thread waits for the next drawable it
        // holds that device's fence lock, which stalls every submission from the preview workers.
        #[cfg(feature = "gpu")]
        let shared = context.wgpu_render_state.as_ref().and_then(|state| {
            crate::GpuContext::from_existing(
                state.device.clone(),
                state.queue.clone(),
                state.adapter.get_info(),
            )
            .ok()
        });
        #[cfg(feature = "gpu")]
        if let Some(shared) = shared.as_ref().filter(|_| {
            std::env::var_os("MOTION_STUDIO_SHARED_GPU").is_some_and(|value| value == "1")
        }) {
            scene.composition = scene
                .composition
                .clone()
                .configure(|options| options.gpu_context(shared.clone()));
        }
        let preview = PreviewCoordinator::new(scene.clone(), context.egui_ctx.clone())?;
        #[cfg(feature = "gpu")]
        let gpu_available = shared.is_some();
        #[cfg(not(feature = "gpu"))]
        let gpu_available = false;
        let audio = AudioForm::default();
        let timeline_view = crate::timeline_ui::TimelineView::new(&scene, &context.egui_ctx, clock);
        let diagnostics = alignment_diagnostics(&scene, clock);
        let export_size = scene.composition.output_size();
        let export_range = (0, clock.frame_count());
        let last_interaction = Instant::now();
        #[cfg(feature = "gpu")]
        let register_texture = context.wgpu_render_state.as_ref().map(|state| {
            let state = state.clone();
            Box::new(move |texture: &wgpu::Texture| {
                // egui's shader assumes sampled texels are linear and encodes them for display, so
                // the already encoded preview bytes are registered through an sRGB view.
                state.renderer.write().register_native_texture(
                    &state.device,
                    &texture.create_view(&wgpu::TextureViewDescriptor {
                        format: Some(wgpu::TextureFormat::Rgba8UnormSrgb),
                        ..Default::default()
                    }),
                    wgpu::FilterMode::Linear,
                )
            }) as Box<dyn Fn(&wgpu::Texture) -> egui::TextureId>
        });
        #[cfg(feature = "gpu")]
        let free_texture = context.wgpu_render_state.as_ref().map(|state| {
            let state = state.clone();
            Box::new(move |texture| state.renderer.write().free_texture(&texture))
                as Box<dyn Fn(egui::TextureId)>
        });
        let audio_preview = AudioPreview::new(context.egui_ctx.clone())?;
        let mut app = Self {
            scene,
            clock,

            renderer: Renderer::Cpu,
            gpu_available,
            preview_settings: preview::PreviewSettings {
                quality: motion_core::RenderQuality::Final,
                scale: 50,
                transparent: true,
                backend: Some(if gpu_available {
                    motion_render::BackendPreference::automatic()
                } else {
                    motion_render::BackendPreference::Cpu
                }),
                inspect: false,
                pixels: false,
                idle: false,
            },

            playback: PlaybackState {
                transport: Transport::default(),
                last_interaction,
                scrub_dragging: false,
                preview_muted: false,
                audio_error: None,
            },
            inspection: InspectorState {
                viewer: crate::viewer::Viewer::default(),
                timeline_view,
                inspector_tab: 0,
                nodes: Arc::from([]),
                output_visible: None,
            },
            preview_state: Default::default(),

            preview,
            texture: None,
            preview_pixels: None,

            #[cfg(feature = "gpu")]
            preview_only: None,
            #[cfg(feature = "gpu")]
            registered_previews: std::collections::VecDeque::new(),
            #[cfg(feature = "gpu")]
            register_texture,
            #[cfg(feature = "gpu")]
            free_texture,

            audio,
            audio_preview,
            exports: ExportState {
                export_destination: "render.mp4".into(),
                export_size,
                export_range,
                export_quality: 18,
                overwrite: false,
                export_png: false,
                export_format: crate::export::ExportFormat::default(),
                export_motion_blur: false,
                export: None,
                export_queue: std::collections::VecDeque::new(),
                completed_exports: std::collections::VecDeque::new(),
                export_message: None,
            },

            cached_frames: Vec::new(),
            cached_previews: Vec::new(),
            thumbnail: None,
            cached_bytes: 0,
            observations: DiagnosticsState {
                backend_status: String::new(),
                elapsed: std::time::Duration::ZERO,
                preview_count: 0,
                cache_hits: 0,
                dropped_frames: 0,
                presentation_times: std::collections::VecDeque::new(),
                frame_stats: None,
                timings: None,
                diagnostics,
            },

            reload: None,
            reload_status: None,

            close_when_idle: false,
            persist_session,
        };
        app.watch_scene();
        if persist_session {
            app.restore_session();
        }
        Ok(app)
    }
}

impl Drop for StudioApp {
    fn drop(&mut self) {
        self.audio_preview.stop();
        if self.persist_session {
            self.save_session();
        }
        #[cfg(feature = "gpu")]
        if let Some(free) = &self.free_texture {
            for (_, id) in self.registered_previews.drain(..) {
                free(id);
            }
        }
    }
}

pub(crate) fn tool(ui: &mut egui::Ui, icon: &str, label: &str) -> bool {
    ui.add_sized(
        [30.0, 30.0],
        egui::Button::new(RichText::new(icon).size(18.0)),
    )
    .on_hover_text(label)
    .clicked()
}
