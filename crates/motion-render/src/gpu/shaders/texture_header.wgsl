// Texture-backed header assembled with composition arithmetic and texture entry points.
// The shared body supplies checked(); this fragment is not a standalone shader.
struct FrameHeader {
    width: u32,
    height: u32,
    left: u32,
    top: u32,
}

struct OperationHeader {
    reserved: vec3u,
    blend_space: u32,
    code: u32,
    kind: u32,
    detail: u32,
    extent: u32,
}

struct SourceFlags {
    particle_tiles: u32,
    reserved: u32,
    first_deferred: u32,
    second_deferred: u32,
}

// Matches the host GpuUniforms: 256 bytes, including aligned value and source rows.
struct GpuUniforms {
    frame: FrameHeader,
    bounds: vec4u,
    operation: OperationHeader,
    values: array<vec4f, 8>,
    sources: array<vec4u, 3>,
    flags: SourceFlags,
}

// Pipeline specialization selects tiled metadata in the shared particle helpers.
override PARTICLE_TILES: bool = false;

struct Pixels {
    values: array<vec4f>,
}

struct OperationData {
    values: array<f32>,
}

struct FrameCoverage {
    values: array<u32>,
}

struct FailureStatus {
    code: atomic<u32>,
}

struct ParticleIndexMask {
    words: array<atomic<u32>>,
}

// Keep the shared composition binding numbers while replacing color inputs with textures.
@group(0) @binding(0)
var<uniform> settings: GpuUniforms;

// Particle states and ordered tile metadata retain the buffer-backed interface.
@group(0) @binding(2)
var<storage, read> second_pixels: Pixels;

@group(0) @binding(3)
var<storage, read_write> output_pixels: Pixels;

@group(0) @binding(4)
var<storage, read> operation_data: OperationData;

// Immutable image mip texels are independent of per-draw coverage and mappings.
@group(0) @binding(11)
var<storage, read> resource_data: OperationData;

@group(0) @binding(5)
var<storage, read_write> failure_status: FailureStatus;

@group(0) @binding(6)
var layer_texture: texture_2d<f32>;

// Working color images contain linear premultiplied RGBA, not encoded sRGB.
@group(0) @binding(7)
var first_texture: texture_2d<f32>;

@group(0) @binding(8)
var second_texture: texture_2d<f32>;

@group(0) @binding(9)
var output_texture: texture_storage_2d<rgba32float, write>;

// Shared original strip/span records and packed u8 alpha are uploaded once per frame.
@group(0) @binding(10)
var<storage, read> frame_coverage: FrameCoverage;

// Generated f32 device coverage; fills address it by their strip's alpha offset.
@group(0) @binding(12)
var<storage, read> device_coverage: array<f32>;

@group(0) @binding(13)
var<storage, read_write> particle_index_mask: ParticleIndexMask;

// Value rows preserve host-narrowed f32 bit patterns.
fn value(index: u32) -> f32 {
    return settings.values[index / 4u][index % 4u];
}

// The first source has half-open global bounds; texture coordinates are origin-relative.
fn first_pixel(coordinate: vec2u) -> vec4f {
    let origin = vec2u(settings.sources[0].x, settings.sources[0].y);
    if any(coordinate < origin) || any(coordinate >= vec2u(settings.sources[0].z, settings.sources[0].w)) {
        return vec4f(0.0);
    }

    let pixel = checked(textureLoad(first_texture, vec2i(coordinate - origin), 0));

    // Resolve a deferred alpha upper bound without rescaling HDR RGB.
    return vec4f(
        pixel.rgb,
        select(pixel.a, min(pixel.a, 1.0), settings.flags.first_deferred != 0u)
    );
}

// The second source has independent global bounds and its own deferred-alpha flag.
fn second_pixel(coordinate: vec2u) -> vec4f {
    let origin = vec2u(settings.sources[1].x, settings.sources[1].y);
    if any(coordinate < origin) || any(coordinate >= vec2u(settings.sources[1].z, settings.sources[1].w)) {
        return vec4f(0.0);
    }

    let pixel = checked(textureLoad(second_texture, vec2i(coordinate - origin), 0));

    return vec4f(
        pixel.rgb,
        select(pixel.a, min(pixel.a, 1.0), settings.flags.second_deferred != 0u)
    );
}
