//! Automatic preview backend selection: calibrates CPU and GPU render latency per preview
//! setting and picks the faster backend for playback.

use super::*;
use std::collections::VecDeque;
use std::sync::OnceLock;
use std::time::{Duration, Instant};

#[derive(Clone)]
pub(super) struct Choice {
    pub backend: motion_render::BackendPreference,
    pub reason: String,
}

type Entry = (PreviewSettings, Arc<OnceLock<Choice>>);

#[derive(Clone)]
pub(super) struct Calibration {
    entries: Arc<Mutex<VecDeque<Entry>>>,
    workers: u32,
}

impl Default for Calibration {
    fn default() -> Self {
        Self::new(1)
    }
}

impl Calibration {
    /// `workers` is the number of preview slots that render concurrently during playback.
    pub fn new(workers: usize) -> Self {
        Self {
            entries: Arc::default(),
            workers: u32::try_from(workers).unwrap_or(u32::MAX).max(1),
        }
    }

    pub fn choose(&self, scene: &StudioScene, settings: PreviewSettings, frame: u32) -> Choice {
        let cell = {
            let mut entries = self.entries.lock().expect("preview calibration");
            if let Some(position) = entries.iter().position(|entry| entry.0 == settings) {
                let entry = entries.remove(position).expect("preview calibration entry");
                let cell = entry.1.clone();
                entries.push_back(entry);
                cell
            } else {
                let cell = Arc::new(OnceLock::new());
                if entries.len() == 16 {
                    entries.pop_front();
                }
                entries.push_back((settings, cell.clone()));
                cell
            }
        };
        cell.get_or_init(|| {
            let cpu = measure(
                scene,
                settings,
                frame,
                motion_render::BackendPreference::Cpu,
            );
            #[cfg(feature = "gpu")]
            let gpu = measure(
                scene,
                settings,
                frame,
                motion_render::BackendPreference::gpu(),
            );
            #[cfg(not(feature = "gpu"))]
            let gpu = None;
            decide(cpu, gpu, self.workers)
        })
        .clone()
    }
}

/// Picks the backend with the shorter time per presented frame during playback. CPU slots render
/// in parallel, while GPU slots share one device whose submissions are serialized, so only CPU
/// latency is divided across the workers.
fn decide(cpu: Option<Duration>, gpu: Option<Duration>, workers: u32) -> Choice {
    let throughput = cpu.map(|cpu| cpu / workers);
    let backend = if gpu.is_some_and(|gpu| throughput.is_none_or(|cpu| gpu < cpu)) {
        motion_render::BackendPreference::gpu()
    } else {
        motion_render::BackendPreference::Cpu
    };
    let reason = match (cpu, gpu) {
        (Some(cpu), Some(gpu)) => format!(
            "Auto: {backend:?}; two-frame mean CPU {:.2} ms over {workers} slots, GPU {:.2} ms",
            cpu.as_secs_f64() * 1000.0,
            gpu.as_secs_f64() * 1000.0
        ),
        (_, None) => "Auto: CPU; no healthy GPU calibration".into(),
        (None, Some(_)) => "Auto: GPU; CPU calibration failed".into(),
    };
    Choice { backend, reason }
}

fn measure(
    scene: &StudioScene,
    settings: PreviewSettings,
    frame: u32,
    backend: motion_render::BackendPreference,
) -> Option<Duration> {
    let size = scene.composition.configured_output_size();
    let composition = scene
        .composition
        .clone()
        .output_size(motion_core::PixelSize::new(
            ((u64::from(size.width) * u64::from(settings.scale)) / 100).max(1) as u32,
            ((u64::from(size.height) * u64::from(settings.scale)) / 100).max(1) as u32,
        ))
        .ok()?
        .backend(backend)
        .quality(settings.quality);
    let scaled = StudioScene {
        composition,
        ..scene.clone()
    };
    let initial = if matches!(backend, motion_render::BackendPreference::Gpu { .. }) {
        Renderer::Gpu
    } else {
        Renderer::Cpu
    };
    let mut renderer = PreviewRenderer::new(&scaled, initial).ok()?;
    let count = u32::try_from(scene.composition.configured_frame_count()).ok()?;
    if count == 0 {
        return None;
    }
    // The first frame pays one-time setup (shader compilation, device and cache warm-up) that
    // playback never sees again.
    let warmup = frame.saturating_add(count / 2).min(count - 1);
    let frames = [
        warmup,
        frame.min(count - 1),
        frame.saturating_add(count / 3).min(count - 1),
    ];
    let mut total = Duration::ZERO;
    for (index, frame) in frames.into_iter().enumerate() {
        let started = Instant::now();
        let image = renderer
            .begin_preview(scene, frame, settings)
            .ok()?
            .finish()
            .ok()?;
        if !renderer.diagnostics().is_empty() {
            return None;
        }
        if index > 0 {
            total = total.saturating_add(started.elapsed());
        }
        drop(image);
    }
    Some(total / 2)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn automatic_choice_uses_healthy_samples_and_prefers_cpu_on_ties() {
        let fast = Some(Duration::from_millis(2));
        let slow = Some(Duration::from_millis(8));
        assert_eq!(
            decide(fast, slow, 1).backend,
            motion_render::BackendPreference::Cpu
        );
        assert_eq!(
            decide(slow, fast, 1).backend,
            motion_render::BackendPreference::gpu()
        );
        assert_eq!(
            decide(fast, fast, 1).backend,
            motion_render::BackendPreference::Cpu
        );
        assert_eq!(
            decide(fast, None, 1).backend,
            motion_render::BackendPreference::Cpu
        );
        assert_eq!(
            decide(None, fast, 1).backend,
            motion_render::BackendPreference::gpu()
        );
        assert_eq!(
            decide(None, None, 1).backend,
            motion_render::BackendPreference::Cpu
        );
    }

    #[test]
    fn parallel_cpu_slots_outweigh_a_single_serialized_gpu_queue() {
        let cpu = Some(Duration::from_millis(50));
        let gpu = Some(Duration::from_millis(30));
        assert_eq!(
            decide(cpu, gpu, 1).backend,
            motion_render::BackendPreference::gpu()
        );
        assert_eq!(
            decide(cpu, gpu, 5).backend,
            motion_render::BackendPreference::Cpu
        );
        assert_eq!(
            decide(Some(Duration::from_millis(400)), gpu, 5).backend,
            motion_render::BackendPreference::gpu()
        );
    }
}
