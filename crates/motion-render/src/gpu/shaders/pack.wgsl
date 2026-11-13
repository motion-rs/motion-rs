// Pack the final linear premultiplied texture into RGBA8 or planar 8/10-bit YUV420.
struct Settings {
    // xyzw: global crop left/top, output width/height.
    first: vec4u,
    // xyzw: half-open global bounds of the texture's logical contents.
    second: vec4u,
    // xyzw: format mode, workgroups in x, deferred alpha upper bound, scan outside crop.
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
var<storage, read_write> output: array<u32>;

@group(0) @binding(3)
var<storage, read_write> failure_status: FailureStatus;

@group(0) @binding(4)
var<storage, read> alpha_thresholds: array<f32>;

// CPU-generated thresholds preserve half-up alpha rounding at byte boundaries.
fn alpha_byte(alpha: f32) -> u32 {
    var low = 0u;
    var high = 255u;

    while low < high {
        let middle = low + (high - low) / 2u;
        if alpha < alpha_thresholds[middle] {
            high = middle;
        } else {
            low = middle + 1u;
        }
    }

    return low;
}

// Apply the piecewise sRGB transfer function to clamped linear RGB.
fn gamma(value: vec3f) -> vec3f {
    let linear = clamp(value, vec3f(0.0), vec3f(1.0));

    return select(
            1.055 * pow(linear, vec3f(1.0 / 2.4)) - 0.055,
            linear * 12.92,
            linear <= vec3f(0.0031308),
        );
}

// The first byte occupies the low bits; RGBA becomes R | G<<8 | B<<16 | A<<24.
fn word(value: vec4u) -> u32 {
    return value.x | (value.y << 8u) | (value.z << 16u) | (value.w << 24u);
}

// Match the linear resolve's finite check and alpha-only correction without clamping HDR RGB.
fn resolved_source(coordinate: vec2u) -> vec4f {
    if any(coordinate < settings.second.xy) || any(coordinate >= settings.second.zw) {
        return vec4f(0.0);
    }

    let pixel = textureLoad(pixels, vec2i(coordinate - settings.second.xy), 0);
    if any((bitcast<vec4u>(pixel) & vec4u(0x7f800000u)) == vec4u(0x7f800000u)) {
        atomicOr(&failure_status.code, 1u);
        return vec4f(0.0);
    }

    return vec4f(pixel.rgb, select(pixel.a, min(pixel.a, 1.0), settings.third.z != 0u));
}

// Crop coordinates are global; textureLoad uses the final image's local origin.
fn source(x: u32, y: u32) -> vec4f {
    return resolved_source(vec2u(x, y) + settings.first.xy);
}

// BT.709 luma and centered chroma; limited-range scaling happens during packing.
fn sample_yuv(x: u32, y: u32) -> vec3f {
    let pixel = source(x, y);
    if pixel.a != 1.0 {
        // YUV output requires opaque input; preserve other shared failure bits.
        atomicOr(&failure_status.code, 2u);
    }

    let linear = display_linear(pixel.rgb, settings.display);
    var rgb = gamma(linear);
    if settings.display[4].x != 0u && settings.display[4].y != 0u {
        rgb = pow(clamp(linear, vec3f(0.0), vec3f(1.0)), vec3f(1.0 / 2.4));
    }
    rgb = display_grain(rgb, vec2u(x, y) + settings.first.xy, settings.display);
    let luma = 0.2126 * rgb.r + 0.7152 * rgb.g + 0.0722 * rgb.b;
    return vec3f(luma, (rgb.b - luma) / 1.8556, (rgb.r - luma) / 1.5748);
}

// Limited-range luma code for one pixel, matching the tiled path.
fn yuv_luma(x: u32, y: u32, ten: bool) -> u32 {
    let scale = select(1.0, 4.0, ten);
    let offset = select(
            0.0,
            display_dither(vec2u(x, y) + settings.first.xy),
            settings.display[4].x != 0u,
        );
    let color = sample_yuv(x, y);
    return u32(
            clamp(
                floor((16.0 + 219.0 * color.x) * scale + offset + 0.5),
                16.0 * scale,
                235.0 * scale,
            ),
        );
}

// Limited-range Cb and Cr codes averaged over the 2x2 block at chroma coordinate (x, y).
fn yuv_chroma(x: u32, y: u32, ten: bool) -> vec2u {
    let scale = select(1.0, 4.0, ten);
    var chroma = vec2f(0.0);
    for(var vertical = 0u; vertical < 2u; vertical++) {
        for(var horizontal = 0u; horizontal < 2u; horizontal++) {
            chroma += sample_yuv(x * 2u + horizontal, y * 2u + vertical).yz * 0.25;
        }
    }
    let code = clamp(
            floor((128.0 + 224.0 * chroma) * scale + 0.5),
            vec2f(16.0 * scale),
            vec2f(240.0 * scale),
        );
    return vec2u(code);
}

@compute @workgroup_size(64)
fn pack(@builtin(global_invocation_id) id: vec3u) {
    let width = settings.first.z;
    let height = settings.first.w;
    // Modes: 0 = straight RGBA8, 1 = premultiplied RGBA8, 2 = YUV420, 3 = YUV420P10LE.
    let mode = settings.third.x;

    // Flatten the 2D dispatch, accounting for 64 invocations per workgroup.
    let index = id.x + id.y * settings.third.y * 64u;

    // The old resolve checked the whole final image, including pixels outside the crop.
    // Cropped output keeps that error contract in this dispatch; full output needs no extra read.
    if settings.third.w != 0u {
        let extent = max(settings.second.zw - settings.second.xy, vec2u(1u));
        if index < extent.x * extent.y {
            let coordinate = settings.second.xy + vec2u(index % extent.x, index / extent.x);
            if any(coordinate < settings.first.xy)
                || any(coordinate >= settings.first.xy + vec2u(width, height)) {
                let checked_pixel = resolved_source(coordinate);
            }
        }
    }

    // One invocation owns one RGBA word, including fully transparent pixels.
    if mode < 2u {
        if index >= width * height {
            return;
        }

        let x = index % width;
        let y = index / width;
        let pixel = source(x, y);
        let alpha = alpha_byte(pixel.a);
        if alpha == 0u {
            output[index] = 0u;
            return;
        }

        // Output-relative 4x4 Bayer ranks add less than half a byte of dither.
        let rank = array<u32, 16>(
                0u,
                8u,
                2u,
                10u,
                12u,
                4u,
                14u,
                6u,
                3u,
                11u,
                1u,
                9u,
                15u,
                7u,
                13u,
                5u,
            );
        let dither_point = select(
                vec2u(x, y),
                vec2u(x, y) + settings.first.xy,
                settings.display[4].x != 0u,
            );
        let offset = (f32(rank[(dither_point.y % 4u) * 4u + dither_point.x % 4u]) + 0.5) / 16.0
            - 0.5;
        let encoded = display_grain(
                gamma(display_linear(pixel.rgb / pixel.a, settings.display)),
                vec2u(x, y) + settings.first.xy,
                settings.display,
            );

        // Output premultiplication applies to encoded sRGB, not linear RGB.
        let rgb = select(encoded, encoded * pixel.a, mode == 1u);
        var quantized =
            vec3u(clamp(floor(rgb * 255.0 + vec3f(offset + 0.5)), vec3f(0.0), vec3f(255.0)));

        if mode == 1u {
            // Quantized premultiplied RGB must not exceed the rounded alpha.
            quantized = min(quantized, vec3u(alpha));
        }

        output[index] = word(vec4u(quantized, alpha));
        return;
    }

    // Modes 4 and 5 are 8-bit and 10-bit YUV420 for any even size. One invocation owns one
    // output word, so planes that straddle rows or end mid-word never race.
    if mode >= 4u {
        let ten = mode == 5u;
        let samples_per_word = select(4u, 2u, ten);
        let count = width * height;
        let chroma_width = width / 2u;
        let chroma_count = count / 4u;
        var packed = 0u;
        for(var lane = 0u; lane < samples_per_word; lane++) {
            let sample = index * samples_per_word + lane;
            if sample >= count + 2u * chroma_count {
                break;
            }
            var value = 0u;
            if sample < count {
                value = yuv_luma(sample % width, sample / width, ten);
            } else {
                let plane = sample - count;
                let chroma = plane % chroma_count;
                let color = yuv_chroma(chroma % chroma_width, chroma / chroma_width, ten);
                value = select(color.x, color.y, plane >= chroma_count);
            }
            packed |= value << (lane * select(8u, 16u, ten));
        }
        if index * samples_per_word < count + 2u * chroma_count {
            output[index] = packed;
        }
        return;
    }

    // Each 8x2 tile owns four luma words and one word for each chroma plane.
    // The host requires width % 8 == 0 and height % 2 == 0 to avoid word-write races.
    let blocks = width / 8u;
    if index >= blocks * (height / 2u) {
        return;
    }

    let x = (index % blocks) * 8u;
    let y = (index / blocks) * 2u;
    var luma: array<u32, 16>;
    var cb = vec4u(0u);
    var cr = vec4u(0u);
    let scale = select(1.0, 4.0, mode == 3u);

    // Average chroma over each 2x2 block; limited-range Y is [16, 235].
    for(var block = 0u; block < 4u; block++) {
        var chroma = vec2f(0.0);
        for(var vertical = 0u; vertical < 2u; vertical++) {
            for(var horizontal = 0u; horizontal < 2u; horizontal++) {
                let column = block * 2u + horizontal;
                let color = sample_yuv(x + column, y + vertical);
                let offset = select(
                        0.0,
                        display_dither(vec2u(x + column, y + vertical) + settings.first.xy),
                        settings.display[4].x != 0u,
                    );
                luma[vertical * 8u + column] = u32(
                        clamp(
                            floor((16.0 + 219.0 * color.x) * scale + offset + 0.5),
                            16.0 * scale,
                            235.0 * scale,
                        ),
                    );
                chroma += color.yz * 0.25;
            }
        }

        // Center Cb/Cr at 128 and quantize to the limited range [16, 240].
        cb[block] = u32(
                clamp(floor((128.0 + 224.0 * chroma.x) * scale + 0.5), 16.0 * scale, 240.0 * scale),
            );
        cr[block] = u32(
                clamp(floor((128.0 + 224.0 * chroma.y) * scale + 0.5), 16.0 * scale, 240.0 * scale),
            );
    }

    if mode == 3u {
        let base = y * width / 2u + x / 2u;
        for(var pair = 0u; pair < 4u; pair++) {
            output[base + pair] = luma[pair * 2u] | (luma[pair * 2u + 1u] << 16u);
            output[base + width / 2u + pair] = luma[8u + pair * 2u] | (luma[9u + pair * 2u] << 16u);
        }
        let count = width * height;
        let chroma_index = (y / 2u) * (width / 4u) + x / 4u;
        output[count / 2u + chroma_index] = cb.x | (cb.y << 16u);
        output[count / 2u + chroma_index + 1u] = cb.z | (cb.w << 16u);
        output[count / 2u + count / 8u + chroma_index] = cr.x | (cr.y << 16u);
        output[count / 2u + count / 8u + chroma_index + 1u] = cr.z | (cr.w << 16u);
        return;
    }

    // Write the tile's two luma rows, with four adjacent samples per word.
    let base = y * width / 4u + x / 4u;
    output[base] = word(vec4u(luma[0], luma[1], luma[2], luma[3]));
    output[base + 1u] = word(vec4u(luma[4], luma[5], luma[6], luma[7]));
    output[base + width / 4u] = word(vec4u(luma[8], luma[9], luma[10], luma[11]));
    output[base + width / 4u + 1u] = word(vec4u(luma[12], luma[13], luma[14], luma[15]));

    // Contiguous Y, Cb and Cr planes contain N, N/4 and N/4 bytes respectively.
    let count = width * height;
    let chroma_index = (y / 2u) * (width / 8u) + x / 8u;
    output[count / 4u + chroma_index] = word(cb);
    output[count / 4u + count / 16u + chroma_index] = word(cr);
}
