// Present linear premultiplied pixels into RGBA16F or explicitly encoded sRGB8 targets.
struct Settings {
    // xy: global crop origin; zw: target width and height.
    first: vec4u,
    // Half-open global logical bounds of the local working texture.
    second: vec4u,
    // xyz: linear output, premultiplied sRGB output, deferred alpha; w: reserved.
    third: vec4u,
    // Scene gain/saturation/curve/white, CDL, enable/transfer/seeds, frame/grain parameters.
    display: array<vec4u, 8>,
}

struct FailureStatus {
    code: atomic<u32>,
}

@group(0) @binding(0)
var pixels: texture_2d<f32>;

@group(0) @binding(1)
var<uniform> settings: Settings;

@group(0) @binding(2)
var<storage, read_write> failure_status: FailureStatus;

fn source(coordinate: vec2u) -> vec4f {
    if any(coordinate < settings.second.xy) || any(coordinate >= settings.second.zw) {
        return vec4f(0.0);
    }

    let pixel = textureLoad(pixels, vec2i(coordinate - settings.second.xy), 0);

    // Match final resolve before alpha correction: NaN/Inf cannot be hidden by saturation.
    if any((bitcast<vec4u>(pixel) & vec4u(0x7f800000u)) == vec4u(0x7f800000u)) {
        atomicOr(&failure_status.code, 1u);
        return vec4f(0.0);
    }

    return vec4f(pixel.rgb, select(pixel.a, min(pixel.a, 1.0), settings.third.z != 0u));
}

// Fragments validate the crop; a cropped output must also retain whole-image finite checking.
@compute @workgroup_size(8, 8)
fn check_outside(@builtin(global_invocation_id) invocation: vec3u) {
    let coordinate = invocation.xy + settings.second.xy;
    if any(coordinate >= settings.second.zw) {
        return;
    }
    if any(coordinate < settings.first.xy) || any(coordinate >= settings.first.xy + settings.first.zw) {
        let checked_pixel = source(coordinate);
    }
}

@vertex
fn vertex(@builtin(vertex_index) index: u32) -> @builtin(position) vec4f {
    // One oversized triangle covers the viewport without a shared diagonal.
    let positions = array<vec2f, 3>(
        vec2f(-1.0, -1.0),
        vec2f(3.0, -1.0),
        vec2f(-1.0, 3.0)
    );

    return vec4f(positions[index], 0.0, 1.0);
}

@fragment
fn fragment(@builtin(position) position: vec4f) -> @location(0) vec4f {
    // Output-relative dither stays local; source reads retain the global crop origin.
    let point = vec2u(position.xy);
    let pixel = source(point + settings.first.xy);

    // Preserve linear HDR values, rejecting magnitudes beyond the finite half-float limit.
    if settings.third.x != 0u {
        if any(abs(pixel) > vec4f(65504.0)) {
            atomicStore(&failure_status.code, 1u);
            return vec4f(0.0);
        }
        return pixel;
    }

    // Alpha is quantized without dither, matching alpha_to_byte in cpu/output.rs.
    // Transparent output must have zero RGB.
    let alpha = round(clamp(pixel.a, 0.0, 1.0) * 255.0) / 255.0;
    if alpha == 0.0 {
        return vec4f(0.0);
    }

    // Unpremultiply before sRGB encoding; the UNORM target does not apply the transfer function.
    let straight = clamp(display_linear(pixel.rgb / pixel.a, settings.display), vec3f(0.0), vec3f(1.0));
    let encoded = display_grain(select(
        1.055 * pow(straight, vec3f(1.0 / 2.4)) - 0.055,
        straight * 12.92,
        straight <= vec3f(0.0031308),
    ), point + settings.first.xy, settings.display);

    // Output-relative 4x4 Bayer ranks reduce banding with sub-byte ordered dither.
    let rank = array<u32, 16>(
         0u,  8u,  2u, 10u,
        12u,  4u, 14u,  6u,
         3u, 11u,  1u,  9u,
        15u,  7u, 13u,  5u
    );
    let dither_point = select(point, point + settings.first.xy, settings.display[4].x != 0u);
    let dither = (f32(rank[(dither_point.y % 4u) * 4u + dither_point.x % 4u]) + 0.5) / 16.0 - 0.5;

    // Optional premultiplication uses the source alpha in encoded sRGB space.
    let rgb = select(encoded, encoded * pixel.a, settings.third.y != 0u);
    let quantized = clamp(
        round(rgb * 255.0 + vec3f(dither)) / 255.0,
        vec3f(0.0),
        vec3f(1.0)
    );

    // Rounded premultiplied RGB must not exceed the quantized alpha.
    return vec4f(
        select(
            quantized,
            min(quantized, vec3f(alpha)),
            settings.third.y != 0u
        ),
        alpha
    );
}
