//! GPU texture ring that recycles preview frame textures between workers and the viewer.

use super::*;
use std::sync::atomic::{AtomicU64, Ordering};

static NEXT_ID: AtomicU64 = AtomicU64::new(1);

pub(crate) struct TextureSlot {
    pub id: u64,
    pub texture: wgpu::Texture,
}

#[derive(Clone)]
pub(crate) struct FrameTexture(Arc<TextureSlot>);

#[derive(Clone, Default)]
pub(super) struct TextureRing(Arc<Mutex<RingState>>);

pub(super) struct TextureReservation {
    ring: TextureRing,
}

/// Upper bound on distinct preview textures; also the number of native registrations the UI keeps.
pub(crate) const MAX_SLOTS: usize = 12;

const MIN_SLOTS: usize = 3;

struct RingState {
    context: Option<crate::GpuContext>,
    size: (u32, u32),
    slots: Vec<Arc<TextureSlot>>,
    reserved: usize,
    capacity: usize,
}

impl Default for RingState {
    fn default() -> Self {
        Self {
            context: None,
            size: (0, 0),
            slots: Vec::new(),
            reserved: 0,
            capacity: MIN_SLOTS,
        }
    }
}

impl FrameTexture {
    pub fn id(&self) -> u64 {
        self.0.id
    }
}

impl std::ops::Deref for FrameTexture {
    type Target = wgpu::Texture;

    fn deref(&self) -> &Self::Target {
        &self.0.texture
    }
}

impl TextureRing {
    /// Sizes the ring to the playback window: every in-flight job, every prefetched frame and the
    /// displayed one hold a slot, and a ring smaller than that evicts frames before they are shown.
    pub fn set_capacity(&self, slots: usize) {
        self.0.lock().expect("preview texture ring").capacity = slots.clamp(MIN_SLOTS, MAX_SLOTS);
    }

    pub fn reserve(&self) -> Option<TextureReservation> {
        let mut ring = self.0.lock().expect("preview texture ring");
        let capacity = ring.capacity.saturating_sub(ring.slots.len())
            + ring.slots.iter().filter(|slot| available(slot)).count();
        if ring.reserved >= capacity {
            return None;
        }
        ring.reserved += 1;
        Some(TextureReservation { ring: self.clone() })
    }

    pub fn lease(
        &self,
        context: &crate::GpuContext,
        width: u32,
        height: u32,
    ) -> Option<FrameTexture> {
        let mut ring = self.0.lock().expect("preview texture ring");
        if ring.size != (width, height)
            || ring
                .context
                .as_ref()
                .is_none_or(|old| !std::ptr::eq(old.device(), context.device()))
        {
            ring.slots.clear();
            ring.size = (width, height);
            ring.context = Some(context.clone());
        }
        if let Some(slot) = ring.slots.iter().find(|slot| available(slot)) {
            return Some(FrameTexture(slot.clone()));
        }
        if ring.slots.len() >= ring.capacity {
            return None;
        }
        let slot = Arc::new(TextureSlot {
            id: NEXT_ID.fetch_add(1, Ordering::Relaxed),
            texture: context.device().create_texture(&wgpu::TextureDescriptor {
                label: Some("Studio reusable preview slot"),
                size: wgpu::Extent3d {
                    width,
                    height,
                    depth_or_array_layers: 1,
                },
                mip_level_count: 1,
                sample_count: 1,
                dimension: wgpu::TextureDimension::D2,
                format: wgpu::TextureFormat::Rgba8Unorm,
                usage: wgpu::TextureUsages::RENDER_ATTACHMENT
                    | wgpu::TextureUsages::TEXTURE_BINDING
                    | wgpu::TextureUsages::COPY_SRC
                    | wgpu::TextureUsages::COPY_DST,
                // The renderer writes encoded bytes through the plain view; egui must sample the
                // same bytes through an sRGB view, or it would encode them a second time.
                view_formats: &[wgpu::TextureFormat::Rgba8UnormSrgb],
            }),
        });
        ring.slots.push(slot.clone());
        Some(FrameTexture(slot))
    }
}

impl Drop for TextureReservation {
    fn drop(&mut self) {
        self.ring.0.lock().expect("preview texture ring").reserved -= 1;
    }
}

fn available<T>(slot: &Arc<T>) -> bool {
    Arc::strong_count(slot) == 1
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::future::Future;

    #[test]
    #[ignore = "requires native GPU; no window, fonts or audio device"]
    fn slots_expose_an_srgb_view_for_egui_over_the_encoded_bytes() -> motion_core::Result {
        let context = crate::GpuContext::new()?;
        let ring = TextureRing::default();
        let slot = ring
            .lease(&context, 8, 4)
            .expect("an empty ring leases a slot");
        assert_eq!(slot.format(), wgpu::TextureFormat::Rgba8Unorm);
        context
            .device()
            .push_error_scope(wgpu::ErrorFilter::Validation);
        let _view = slot.create_view(&wgpu::TextureViewDescriptor {
            format: Some(wgpu::TextureFormat::Rgba8UnormSrgb),
            ..Default::default()
        });
        // Native error scopes resolve without waiting, so one poll suffices.
        let mut scope = std::pin::pin!(context.device().pop_error_scope());
        let mut task = std::task::Context::from_waker(std::task::Waker::noop());
        let std::task::Poll::Ready(error) = scope.as_mut().poll(&mut task) else {
            panic!("the validation scope should already be resolved");
        };
        assert!(error.is_none(), "{error:?}");
        Ok(())
    }

    #[test]
    fn capacity_follows_the_playback_window_within_fixed_bounds() {
        let ring = TextureRing::default();
        let reserve_all = |ring: &TextureRing| {
            std::iter::from_fn(|| ring.reserve())
                .collect::<Vec<_>>()
                .len()
        };
        assert_eq!(reserve_all(&ring), MIN_SLOTS);
        ring.set_capacity(8);
        assert_eq!(reserve_all(&ring), 8);
        ring.set_capacity(MAX_SLOTS + 10);
        assert_eq!(reserve_all(&ring), MAX_SLOTS);
        ring.set_capacity(0);
        assert_eq!(reserve_all(&ring), MIN_SLOTS);
    }

    #[test]
    fn occupied_slots_are_not_reused_until_frame_holders_release_them() {
        let slots = (0..3).map(|_| Arc::new(())).collect::<Vec<_>>();
        let frames = slots.clone();
        assert!(slots.iter().all(|slot| !available(slot)));
        drop(frames);
        assert!(slots.iter().all(available));
    }
}
