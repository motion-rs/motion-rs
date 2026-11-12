//! Native GPU layer attachment capability checks used only by tests.

#[cfg(test)]
mod layer_format_tests {
    #[test]
    #[ignore = "requires a native GPU; checks layer attachment capabilities without a window"]
    fn native_layer_attachment_capabilities() -> motion_core::Result {
        pollster::block_on(async {
            let instance = crate::gpu::instance(crate::BackendKind::Gpu)?;
            let adapter = instance
                .request_adapter(&wgpu::RequestAdapterOptions::default())
                .await
                .map_err(|error| motion_core::Error::GpuUnavailable(Box::new(error)))?;
            let info = crate::gpu::adapter_info(&adapter)?;
            for format in [
                wgpu::TextureFormat::Rgba32Float,
                wgpu::TextureFormat::Rgba16Float,
            ] {
                let features = adapter.get_texture_format_features(format);
                eprintln!(
                    "Layer attachment {format:?} on {info:?}: usages={:?}, flags={:?}, adapter_specific={}",
                    features.allowed_usages,
                    features.flags,
                    adapter
                        .features()
                        .contains(wgpu::Features::TEXTURE_ADAPTER_SPECIFIC_FORMAT_FEATURES)
                );
                assert!(
                    features
                        .allowed_usages
                        .contains(wgpu::TextureUsages::RENDER_ATTACHMENT)
                );
            }
            let context = crate::GpuContext::new()?;
            let supported = adapter
                .features()
                .contains(wgpu::Features::TEXTURE_ADAPTER_SPECIFIC_FORMAT_FEATURES)
                && adapter
                    .get_texture_format_features(wgpu::TextureFormat::Rgba32Float)
                    .flags
                    .contains(wgpu::TextureFormatFeatureFlags::BLENDABLE);
            assert_eq!(context.textures.blends.is_some(), supported);
            Ok(())
        })
    }
}
