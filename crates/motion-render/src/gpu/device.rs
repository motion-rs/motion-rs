//! Shared native GPU creation, adapter identity and device-loss monitoring.

use crate::{BackendKind, GpuBackend, GpuInfo};
use motion_core::sync::MutexExt;

use motion_core::Result;
use std::io;
use std::sync::Arc;

const GPU_WAIT_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(60);

/// Creates a Metal/Vulkan/DX12 instance for an explicit GPU request.
/// Rejects CPU renderer requests and unsupported browser targets before requesting an adapter.
pub fn instance(renderer: BackendKind) -> Result<wgpu::Instance> {
    let backends = match renderer {
        BackendKind::Gpu if !cfg!(target_arch = "wasm32") => {
            wgpu::Backends::METAL | wgpu::Backends::VULKAN | wgpu::Backends::DX12
        }
        _ => {
            return Err(motion_core::Error::GpuUnavailable(Box::new(
                io::Error::new(
                    io::ErrorKind::Unsupported,
                    "Requested GPU backend is unavailable on this platform",
                ),
            )));
        }
    };
    Ok(wgpu::Instance::new(wgpu::InstanceDescriptor {
        backends,
        ..wgpu::InstanceDescriptor::new_without_display_handle()
    }))
}

/// Requests an adapter compatible with the optional surface, without creating a device.
pub async fn adapter(
    instance: &wgpu::Instance,
    surface: Option<&wgpu::Surface<'_>>,
) -> Result<wgpu::Adapter> {
    instance
        .request_adapter(&wgpu::RequestAdapterOptions {
            compatible_surface: surface,
            ..Default::default()
        })
        .await
        .map_err(|error| motion_core::Error::GpuUnavailable(Box::new(error)))
}

/// Reads adapter identity and rejects unsupported backend families without initializing a device.
pub fn adapter_info(adapter: &wgpu::Adapter) -> Result<GpuInfo> {
    info_from_adapter(adapter.get_info())
}

/// Converts wgpu identity into engine metadata for Metal, Vulkan or DX12.
pub fn info_from_adapter(info: wgpu::AdapterInfo) -> Result<GpuInfo> {
    let backend = match info.backend {
        wgpu::Backend::Metal => GpuBackend::Metal,
        wgpu::Backend::Vulkan => GpuBackend::Vulkan,
        wgpu::Backend::Dx12 => GpuBackend::Dx12,
        _ => {
            return Err(motion_core::Error::GpuUnavailable(Box::new(
                io::Error::new(io::ErrorKind::Unsupported, "Unsupported native GPU backend"),
            )));
        }
    };
    Ok(GpuInfo {
        backend,
        name: info.name,
        vendor: info.vendor,
        device: info.device,
        device_type: format!("{:?}", info.device_type),
        driver: info.driver,
        driver_info: info.driver_info,
    })
}

pub(super) fn wait_for_gpu(
    device: &wgpu::Device,
    submission: Option<wgpu::SubmissionIndex>,
) -> Result {
    device
        .poll(wgpu::PollType::Wait {
            submission_index: submission,
            timeout: Some(GPU_WAIT_TIMEOUT),
        })
        .map_err(|error| motion_core::Error::Gpu(Box::new(error)))?;
    Ok(())
}

#[derive(Clone, Debug)]
struct DeviceLoss {
    reason: wgpu::DeviceLostReason,
    message: String,
}

impl std::fmt::Display for DeviceLoss {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(formatter, "{:?}: {}", self.reason, self.message)
    }
}

impl std::error::Error for DeviceLoss {}

#[derive(Default)]
/// Shared sticky device-loss state installed on one imported or newly created GPU device.
pub struct DeviceMonitor(Arc<std::sync::Mutex<Option<DeviceLoss>>>);

impl DeviceMonitor {
    /// Installs the device-loss callback; clones observe the same recorded failure.
    /// Register this once per device because wgpu stores only one loss callback.
    pub fn new(device: &wgpu::Device) -> Self {
        let monitor = Self::default();
        let state = monitor.0.clone();
        device.set_device_lost_callback(move |reason, message| {
            *state.lock_unpoisoned() = Some(DeviceLoss { reason, message });
        });
        monitor
    }

    /// Returns the recorded typed device-loss error, without attempting recovery or fallback.
    pub fn check(&self) -> Result {
        match self.0.lock_unpoisoned().clone() {
            Some(cause) => Err(motion_core::Error::DeviceLost(Box::new(cause))),
            None => Ok(()),
        }
    }
}
