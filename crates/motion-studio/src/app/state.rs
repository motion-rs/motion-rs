//! Grouped mutable state of the workbench: playback, inspector, export form and queue,
//! and diagnostics counters that [`super::StudioApp`] owns.

use crate::playback::Transport;
use crate::preview::ExportJob;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Instant;

pub(crate) struct PlaybackState {
    pub(crate) transport: Transport,
    pub(crate) last_interaction: Instant,
    pub(crate) scrub_dragging: bool,
    pub(crate) preview_muted: bool,
    pub(crate) audio_error: Option<String>,
}

pub(crate) struct InspectorState {
    pub(crate) viewer: crate::viewer::Viewer,
    pub(crate) timeline_view: crate::timeline_ui::TimelineView,
    pub(crate) inspector_tab: usize,
    pub(crate) nodes: Arc<[motion_core::authoring::NodeInspection]>,
    pub(crate) output_visible: Option<bool>,
}

pub(crate) struct ExportState {
    pub(crate) export_destination: String,
    pub(crate) export_size: motion_core::PixelSize,
    pub(crate) export_range: (u32, u32),
    pub(crate) export_quality: u8,
    pub(crate) overwrite: bool,
    pub(crate) export_png: bool,
    pub(crate) export_format: crate::export::ExportFormat,
    pub(crate) export_motion_blur: bool,
    pub(crate) export: Option<ExportJob>,
    pub(crate) export_queue: std::collections::VecDeque<crate::export::QueuedExport>,
    pub(crate) completed_exports: std::collections::VecDeque<(PathBuf, bool, String)>,
    pub(crate) export_message: Option<(bool, String)>,
}

pub(crate) struct DiagnosticsState {
    pub(crate) backend_status: String,
    pub(crate) elapsed: std::time::Duration,
    pub(crate) preview_count: u64,
    pub(crate) cache_hits: u64,
    pub(crate) dropped_frames: u64,
    pub(crate) presentation_times: std::collections::VecDeque<Instant>,
    pub(crate) frame_stats: Option<motion_render::FrameStats>,
    pub(crate) timings: Option<motion_render::composition::FrameTimings>,
    pub(crate) diagnostics: std::collections::VecDeque<(u32, String)>,
}
