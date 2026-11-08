//! Texture interfaces reuse the existing composition arithmetic.

use super::*;

pub(in crate::gpu) fn shader_source() -> String {
    let (_, arithmetic) = include_str!("../shaders/compose.wgsl")
        .split_once("fn checked(pixel: vec4f)")
        .expect("shared shader arithmetic boundary");
    let (arithmetic, _) = arithmetic
        .split_once("@vertex\nfn strip_vertex")
        .expect("shared arithmetic end");
    let constants = shader_constants();
    [
        constants.as_str(),
        include_str!("../shaders/texture_header.wgsl"),
        include_str!("../shaders/noise.wgsl"),
        "\nfn checked(pixel: vec4f)",
        arithmetic,
        include_str!("../shaders/lut.wgsl"),
        include_str!("../shaders/texture_entries.wgsl"),
    ]
    .concat()
}

impl TexturePipelines {
    pub(super) fn new(device: &wgpu::Device, shader: &wgpu::ShaderModule, hardware: bool) -> Self {
        let compute = |entry, tiled| {
            device.create_compute_pipeline(&wgpu::ComputePipelineDescriptor {
                label: Some(entry),
                layout: None,
                module: shader,
                entry_point: Some(entry),
                compilation_options: wgpu::PipelineCompilationOptions {
                    constants: &std::collections::HashMap::from([(
                        "PARTICLE_TILES".to_owned(),
                        tiled,
                    )]),
                    ..Default::default()
                },
                cache: None,
            })
        };
        let render = |entry, blend| {
            device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
                label: Some(entry),
                layout: None,
                vertex: wgpu::VertexState {
                    module: shader,
                    entry_point: Some("texture_vertex"),
                    compilation_options: Default::default(),
                    buffers: &[wgpu::VertexBufferLayout {
                        array_stride: 16,
                        step_mode: wgpu::VertexStepMode::Instance,
                        attributes: &wgpu::vertex_attr_array![0 => Float32x4],
                    }],
                },
                fragment: Some(wgpu::FragmentState {
                    module: shader,
                    entry_point: Some(entry),
                    compilation_options: Default::default(),
                    targets: &[Some(wgpu::ColorTargetState {
                        format: wgpu::TextureFormat::Rgba32Float,
                        blend,
                        write_mask: wgpu::ColorWrites::ALL,
                    })],
                }),
                primitive: Default::default(),
                depth_stencil: None,
                multisample: Default::default(),
                multiview: None,
                cache: None,
            })
        };
        Self {
            compute: compute("texture_main", 0.0),
            particles: compute("texture_main", 1.0),
            check: compute("texture_check", 0.0),
            backdrop: render("texture_backdrop", None),
            blends: hardware.then(|| {
                [
                    render(
                        "texture_source",
                        Some(wgpu::BlendState::PREMULTIPLIED_ALPHA_BLENDING),
                    ),
                    render(
                        "texture_source",
                        Some(wgpu::BlendState {
                            color: wgpu::BlendComponent {
                                src_factor: wgpu::BlendFactor::One,
                                dst_factor: wgpu::BlendFactor::One,
                                operation: wgpu::BlendOperation::Add,
                            },
                            alpha: wgpu::BlendComponent {
                                src_factor: wgpu::BlendFactor::One,
                                dst_factor: wgpu::BlendFactor::One,
                                operation: wgpu::BlendOperation::Add,
                            },
                        }),
                    ),
                ]
            }),
            resolve: compute("texture_resolve", 0.0),
        }
    }
}
