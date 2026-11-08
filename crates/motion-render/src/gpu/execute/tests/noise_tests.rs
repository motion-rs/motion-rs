//! Direct native readback of shared integer hashes and continuous noise samples.

use super::*;
use motion_core::noise;
use wgpu::util::DeviceExt;

const KERNEL: &str = r#"
struct Input { cells: vec4u, point: vec4f }
struct Output { hashes: vec4u, value: vec4f, perlin: vec4f, fractal: vec4f, advanced: vec4f, warp: vec4f, extended: vec4f }
@group(0) @binding(0) var<storage, read> inputs: array<Input>;
@group(0) @binding(1) var<storage, read_write> outputs: array<Output>;
@compute @workgroup_size(64)
fn sample_noise(@builtin(global_invocation_id) id: vec3u) {
    if id.x >= arrayLength(&inputs) { return; }
    let input = inputs[id.x];
    let cell = bitcast<vec3i>(input.cells.xyz);
    let seed = input.cells.w;
    let point = input.point.xy;
    outputs[id.x].hashes = vec4u(noise_hash32(seed), noise_hash2(cell.xy, seed), noise_hash3(cell, seed),
        bitcast<u32>(noise_fbm(point, seed, 0u, 2.0, 0.5, 1u)));
    outputs[id.x].value = vec4f(noise_value2_d(point, seed), noise_value2(point, seed));
    outputs[id.x].perlin = vec4f(noise_perlin2_d(point, seed), noise_perlin2(point, seed));
    outputs[id.x].fractal = vec4f(noise_fbm(point, seed, 4u, 2.0, 0.5, 0u),
        noise_fbm(point, seed, 4u, 2.0, 0.5, 1u), noise_curl2(point, seed));
    outputs[id.x].advanced = vec4f(noise_simplex2(point, seed), noise_worley2(point, seed), noise_perlin3(input.point.xyz, seed));
    outputs[id.x].warp = vec4f(noise_domain_warp(point, seed, 0.5, 1.0, 4u, 2.0, 0.5),
        noise_fbm(point, seed, 4u, 2.0, 0.5, 2u), noise_fbm(point, seed, 4u, 2.0, 0.5, 3u));
    outputs[id.x].extended = vec4f(noise_film_grain(abs(point) + vec2f(0.5, 0.25), seed ^ input.cells.z),
        noise_fbm(point, seed, 4u, 2.0, 0.5, 4u), noise_domain_warp(point, seed, -1.0, 1.0, 4u, 2.0, 0.5));
}
"#;

fn read_samples(context: &GpuContext, words: &[[u32; 8]]) -> motion_core::Result<Vec<[u32; 28]>> {
    let source = [include_str!("../../shaders/noise.wgsl"), KERNEL].concat();
    let shader = context
        .device
        .create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("Shared noise numerical fixture"),
            source: wgpu::ShaderSource::Wgsl(source.into()),
        });
    let pipeline = context
        .device
        .create_compute_pipeline(&wgpu::ComputePipelineDescriptor {
            label: Some("Shared noise numerical fixture"),
            layout: None,
            module: &shader,
            entry_point: Some("sample_noise"),
            compilation_options: Default::default(),
            cache: None,
        });
    let input = context
        .device
        .create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("Noise sample inputs"),
            contents: bytemuck::cast_slice(words),
            usage: wgpu::BufferUsages::STORAGE,
        });
    let bytes = words.len() as u64 * std::mem::size_of::<[u32; 28]>() as u64;
    let output = context.device.create_buffer(&wgpu::BufferDescriptor {
        label: Some("Noise sample outputs"),
        size: bytes,
        mapped_at_creation: false,
        usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_SRC,
    });
    let readback = context.device.create_buffer(&wgpu::BufferDescriptor {
        label: Some("Noise numerical readback"),
        size: bytes,
        mapped_at_creation: false,
        usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
    });
    let bindings = context
        .device
        .create_bind_group(&wgpu::BindGroupDescriptor {
            label: None,
            layout: &pipeline.get_bind_group_layout(0),
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: input.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: output.as_entire_binding(),
                },
            ],
        });
    let mut encoder = context.device.create_command_encoder(&Default::default());
    {
        let mut pass = encoder.begin_compute_pass(&Default::default());
        pass.set_pipeline(&pipeline);
        pass.set_bind_group(0, &bindings, &[]);
        pass.dispatch_workgroups((words.len() as u32).div_ceil(64), 1, 1);
    }
    encoder.copy_buffer_to_buffer(&output, 0, &readback, 0, bytes);
    context.queue.submit([encoder.finish()]);
    let (sender, receiver) = std::sync::mpsc::channel();
    readback
        .slice(..)
        .map_async(wgpu::MapMode::Read, move |result| {
            sender.send(result).unwrap();
        });
    context.device.poll(wgpu::Maintain::Wait);
    receiver.recv().unwrap().unwrap();
    let mapped = readback.slice(..).get_mapped_range();
    let samples = bytemuck::cast_slice::<u8, [u32; 28]>(&mapped).to_vec();
    drop(mapped);
    readback.unmap();
    context.check_device()?;
    Ok(samples)
}

#[test]
#[ignore = "requires native GPU; exact shared hashes and numerical noise twins, no timing"]
fn shared_noise_cpu_gpu_numerical_parity() -> motion_core::Result {
    let context = GpuContext::new()?;
    let words: Vec<_> = (0..257i32)
        .map(|index| {
            let point = [
                ((index % 17) as f32 - 8.0) * 0.125,
                ((index / 17) as f32 - 7.0) * 0.25,
            ];
            let horizontal = match index % 3 {
                0 => i32::MIN,
                1 => i32::MAX,
                _ => -index,
            };
            [
                horizontal as u32,
                index.wrapping_mul(7919) as u32,
                (-index) as u32,
                (index as u32).wrapping_mul(0x9e37_79b9),
                point[0].to_bits(),
                point[1].to_bits(),
                0.375f32.to_bits(),
                0,
            ]
        })
        .collect();
    let actual = read_samples(&context, &words)?;
    assert_eq!(actual, read_samples(&context, &words)?);
    for (input, actual) in words.iter().zip(actual) {
        let seed = input[3];
        assert_eq!(
            &actual[..4],
            &[
                noise::hash32(seed),
                noise::hash2(input[0] as i32, input[1] as i32, seed),
                noise::hash3(input[0] as i32, input[1] as i32, input[2] as i32, seed),
                0,
            ]
        );
        let point = [
            f64::from(f32::from_bits(input[4])),
            f64::from(f32::from_bits(input[5])),
        ];
        let value = noise::value2_d(point, seed);
        let perlin = noise::perlin2_d(point, seed);
        let worley = noise::worley2(point, seed);
        let warp = noise::domain_warp(
            point,
            noise::DomainWarp {
                seed,
                ..noise::DomainWarp::default()
            },
        )?;
        let expected = [
            value.0,
            value.1[0],
            value.1[1],
            value.0,
            perlin.0,
            perlin.1[0],
            perlin.1[1],
            perlin.0,
            noise::fbm(
                |point| noise::value2(point, seed),
                point,
                noise::Fractal::default(),
            )?,
            noise::fbm(
                |point| noise::perlin2(point, seed),
                point,
                noise::Fractal::default(),
            )?,
            perlin.1[1],
            -perlin.1[0],
            noise::simplex2(point, seed),
            worley.0,
            worley.1,
            noise::perlin3(
                [point[0], point[1], f64::from(f32::from_bits(input[6]))],
                seed,
            ),
            warp[0],
            warp[1],
            noise::turbulence2(point, seed, noise::Fractal::default())?,
            noise::ridged2(point, seed, noise::Fractal::default())?,
            f64::from(noise::film_grain(
                [point[0].abs() as f32 + 0.5, point[1].abs() as f32 + 0.25],
                u64::from(seed) | (u64::from(input[2]) << 32),
            )),
            noise::fbm(
                |point| noise::simplex2(point, seed),
                point,
                noise::Fractal::default(),
            )?,
            point[0],
            point[1],
        ];
        for (bits, expected) in actual[4..].iter().zip(expected) {
            let actual = f64::from(f32::from_bits(*bits));
            assert!(
                (actual - expected).abs() <= 2e-6,
                "{point:?} seed={seed}: {actual} != {expected}"
            );
        }
    }
    Ok(())
}
