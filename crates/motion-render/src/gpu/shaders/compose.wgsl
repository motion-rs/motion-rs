// Named composition uniforms preserve the original host byte offsets.
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

@group(0) @binding(0)
var<uniform> settings: GpuUniforms;

// Color buffers contain premultiplied RGBA; particle passes reuse arrays for state and metadata.
@group(0) @binding(1)
var<storage, read> first_pixels: Pixels;

@group(0) @binding(2)
var<storage, read> second_pixels: Pixels;

@group(0) @binding(3)
var<storage, read_write> output_pixels: Pixels;

// Per-operation side data (coverage masks, image levels and particle records); the layout depends on the operation.
@group(0) @binding(4)
var<storage, read> operation_data: OperationData;

@group(0) @binding(5)
var<storage, read_write> failure_status: FailureStatus;

@group(0) @binding(6)
var layer_texture: texture_2d<f32>;

// One frame payload: counts and alpha base, six-word strips, four-word spans, packed alpha bytes.
@group(0) @binding(10)
var<storage, read> frame_coverage: FrameCoverage;

// Values are host-narrowed f32 lanes; indexing retains their stored bits.
fn value(index: u32) -> f32 {
    return settings.values[index / 4u][index % 4u];
}

// Inputs are tightly packed inside half-open global bounds; pixels outside are transparent.
fn first_pixel(coordinate: vec2u) -> vec4f {
    let origin = vec2u(settings.sources[0].x, settings.sources[0].y);
    if any(coordinate < origin)
        || any(coordinate >= vec2u(settings.sources[0].z, settings.sources[0].w)) {
        return vec4f(0.0);
    }

    let local = coordinate - origin;
    return first_pixels.values[local.y * (settings.sources[0].z - settings.sources[0].x) + local.x];
}

// The second source has independent half-open global bounds and row width.
fn second_pixel(coordinate: vec2u) -> vec4f {
    let origin = vec2u(settings.sources[1].x, settings.sources[1].y);
    if any(coordinate < origin)
        || any(coordinate >= vec2u(settings.sources[1].z, settings.sources[1].w)) {
        return vec4f(0.0);
    }

    let local = coordinate - origin;
    return second_pixels.values[
            local.y * (settings.sources[1].z - settings.sources[1].x) + local.x
        ];
}

// An all-ones f32 exponent catches both NaN and infinity in any channel.
// Latch the shared failure flag and substitute transparent black.
fn checked(pixel: vec4f) -> vec4f {
    if any((bitcast<vec4u>(pixel) & vec4u(0x7f800000u)) == vec4u(0x7f800000u)) {
        atomicStore(&failure_status.code, 1u);
        return vec4f(0.0);
    }
    return pixel;
}

fn straight(pixel: vec4f) -> vec4f {
    if pixel.a == 0.0 {
        return vec4f(0.0);
    }

    return checked(vec4f(pixel.rgb / pixel.a, pixel.a));
}

// Clamp alpha only; RGB is not restricted to the unit interval before premultiplication.
fn premultiplied(input_color: vec4f) -> vec4f {
    let pixel = checked(input_color);
    let alpha = clamp(pixel.a, 0.0, 1.0);
    return checked(vec4f(pixel.rgb * alpha, alpha));
}

fn hard_light(backdrop: f32, source: f32) -> f32 {
    if source <= 0.5 {
        return 2.0 * backdrop * source;
    }
    return 1.0 - 2.0 * (1.0 - backdrop) * (1.0 - source);
}

fn additional_blend(backdrop: f32, source: f32, blend_mode: u32) -> f32 {
    switch blend_mode {
        case BLEND_OVERLAY { return hard_light(source, backdrop); }
        case BLEND_HARD_LIGHT { return hard_light(backdrop, source); }
        case BLEND_SOFT_LIGHT {
            if source <= 0.5 {
                return backdrop - (1.0 - 2.0 * source) * backdrop * (1.0 - backdrop);
            }

            var curve: f32;
            if backdrop <= 0.25 {
                curve = ((16.0 * backdrop - 12.0) * backdrop + 4.0) * backdrop;
            } else {
                curve = sqrt(backdrop);
            }
            return backdrop + (2.0 * source - 1.0) * (curve - backdrop);
        }
        case BLEND_COLOR_DODGE {
            if backdrop <= 0.0 {
                return 0.0;
            }

            if source >= 1.0 {
                return 1.0;
            }
            return min(1.0, backdrop / (1.0 - source));
        }
        case BLEND_COLOR_BURN {
            if backdrop >= 1.0 {
                return 1.0;
            }

            if source <= 0.0 {
                return 0.0;
            }
            return 1.0 - min(1.0, (1.0 - backdrop) / source);
        }
        default { return 0.0; }
    }
}

// Nonseparable blend modes use these blend-specific weights, not a transfer-function conversion.
fn blend_luminosity(color: vec3f) -> f32 {
    return dot(color, vec3f(0.3, 0.59, 0.11));
}

fn blend_saturation(color: vec3f) -> f32 {
    return max(max(color.r, color.g), color.b) - min(min(color.r, color.g), color.b);
}

fn set_saturation(color: vec3f, saturation: f32) -> vec3f {
    let minimum = min(min(color.r, color.g), color.b);
    let range = blend_saturation(color);
    if range == 0.0 {
        return vec3f(0.0);
    }

    return (color - minimum) / range * saturation;
}

fn set_luminosity(input_color: vec3f, luminosity: f32) -> vec3f {
    var color = input_color + luminosity - blend_luminosity(input_color);

    // Preserve out-of-range luminosity rather than forcing HDR values into [0, 1].
    if luminosity < 0.0 || luminosity > 1.0 {
        return color;
    }

    let minimum = min(min(color.r, color.g), color.b);
    let maximum = max(max(color.r, color.g), color.b);
    if minimum < 0.0 && luminosity != minimum {
        color = luminosity + (color - luminosity) * luminosity / (luminosity - minimum);
    }
    if maximum > 1.0 && maximum != luminosity {
        color = luminosity + (color - luminosity) * (1.0 - luminosity) / (maximum - luminosity);
    }
    return color;
}

// Inputs and output are premultiplied; nonlinear blend functions use straight RGB.
fn composite_linear(backdrop: vec4f, source: vec4f, blend_mode: u32) -> vec4f {
    if blend_mode == BLEND_SOURCE_OVER {
        return checked(source + backdrop * (1.0 - source.a));
    }
    if blend_mode == BLEND_PLUS || blend_mode == BLEND_LINEAR_PLUS {
        return checked(vec4f(source.rgb + backdrop.rgb, min(source.a + backdrop.a, 1.0)));
    }

    let source_rgb = straight(source).rgb;
    let backdrop_rgb = straight(backdrop).rgb;
    var mixed = vec3f(0.0);

    switch blend_mode {
        case BLEND_MULTIPLY { mixed = source_rgb * backdrop_rgb; }
        case BLEND_SCREEN { mixed = source_rgb + backdrop_rgb - source_rgb * backdrop_rgb; }
        case BLEND_DARKEN { mixed = min(source_rgb, backdrop_rgb); }
        case BLEND_LIGHTEN { mixed = max(source_rgb, backdrop_rgb); }
        case BLEND_DIFFERENCE { mixed = abs(backdrop_rgb - source_rgb); }
        case BLEND_EXCLUSION {
            mixed = source_rgb + backdrop_rgb - 2.0 * source_rgb * backdrop_rgb;
        }
        case BLEND_HUE {
            mixed = set_luminosity(
                    set_saturation(source_rgb, blend_saturation(backdrop_rgb)),
                    blend_luminosity(backdrop_rgb),
                );
        }
        case BLEND_SATURATION {
            mixed = set_luminosity(
                    set_saturation(backdrop_rgb, blend_saturation(source_rgb)),
                    blend_luminosity(backdrop_rgb),
                );
        }
        case BLEND_COLOR { mixed = set_luminosity(source_rgb, blend_luminosity(backdrop_rgb)); }
        case BLEND_LUMINOSITY {
            mixed = set_luminosity(backdrop_rgb, blend_luminosity(source_rgb));
        }
        default {
            mixed = vec3f(
                    additional_blend(backdrop_rgb.r, source_rgb.r, blend_mode),
                    additional_blend(backdrop_rgb.g, source_rgb.g, blend_mode),
                    additional_blend(backdrop_rgb.b, source_rgb.b, blend_mode),
                );
        }
    }

    return checked(
            vec4f(
                source.rgb * (1.0 - backdrop.a) + backdrop.rgb * (1.0 - source.a)
                + source.a * backdrop.a * mixed,
                source.a + backdrop.a * (1.0 - source.a),
            ),
        );
}

// Apply the sRGB transfer to straight RGB, preserving sign and then premultiplying again.
fn encode_blend(pixel: vec4f) -> vec4f {
    if pixel.a == 0.0 {
        return vec4f(0.0);
    }

    let rgb = pixel.rgb / pixel.a;
    let magnitude = abs(rgb);
    let encoded = select(
            1.055 * pow(magnitude, vec3f(1.0 / 2.4)) - 0.055,
            magnitude * 12.92,
            magnitude <= vec3f(0.0031308),
        );
    return vec4f(sign(rgb) * encoded * pixel.a, pixel.a);
}

fn composite(backdrop: vec4f, source: vec4f, blend_mode: u32) -> vec4f {
    if all(source == vec4f(0.0)) {
        return backdrop;
    }

    // LinearPlus always bypasses the encoded-space path, regardless of blend space.
    if settings.operation.blend_space == 0u || blend_mode == BLEND_LINEAR_PLUS {
        return composite_linear(backdrop, source, blend_mode);
    }

    let result = composite_linear(encode_blend(backdrop), encode_blend(source), blend_mode);
    if result.a == 0.0 {
        return vec4f(0.0);
    }

    // Decode straight RGB back to linear light without clamping HDR magnitudes.
    let rgb = result.rgb / result.a;
    let magnitude = abs(rgb);
    let decoded = select(
            pow((magnitude + 0.055) / 1.055, vec3f(2.4)),
            magnitude / 12.92,
            magnitude <= vec3f(0.04045),
        );
    return checked(vec4f(sign(rgb) * decoded * result.a, result.a));
}

fn at_pixel(coordinate: vec2i) -> vec4f {
    if any(coordinate < vec2i(0)) || coordinate.x >= i32(settings.frame.width)
        || coordinate.y >= i32(settings.frame.height) {
        return vec4f(0.0);
    }
    return first_pixel(vec2u(coordinate));
}

fn sample_pixel(coordinate: vec2f) -> vec4f {
    let base = floor(coordinate);
    let fraction = coordinate - base;
    let origin = vec2i(base);
    return mix(
            mix(at_pixel(origin), at_pixel(origin + vec2i(1, 0)), fraction.x),
            mix(at_pixel(origin + vec2i(0, 1)), at_pixel(origin + vec2i(1, 1)), fraction.x),
            fraction.y,
        );
}

// Second-source pixel with transparent black at negative coordinates; second_pixel already
// clears coordinates outside the second source.
fn second_at(coordinate: vec2i) -> vec4f {
    if any(coordinate < vec2i(0)) {
        return vec4f(0.0);
    }
    return second_pixel(vec2u(coordinate));
}

// Bilinear second-source read with the same pixel-corner convention as sample_pixel.
fn second_sample(coordinate: vec2f) -> vec4f {
    let base = floor(coordinate);
    let fraction = coordinate - base;
    let origin = vec2i(base);
    return mix(
            mix(second_at(origin), second_at(origin + vec2i(1, 0)), fraction.x),
            mix(second_at(origin + vec2i(0, 1)), second_at(origin + vec2i(1, 1)), fraction.x),
            fraction.y,
        );
}

// White noise in [-amplitude, amplitude) hashed from the output-relative pixel. The operation
// detail/extent words hold the seed halves, which fold together exactly as on the CPU.
fn noise_delta(coordinate: vec2u) -> f32 {
    let relative = vec2i(coordinate) - vec2i(vec2u(settings.frame.left, settings.frame.top));
    let hash = noise_hash2(relative, settings.operation.detail ^ settings.operation.extent);
    return (f32(hash >> 8u) / 16777216.0 * 2.0 - 1.0) * value(0u);
}

fn extend_paint(position: f32, mode: u32) -> f32 {
    if mode == 0u {
        return clamp(position, 0.0, 1.0);
    }

    if mode == 1u {
        return position - floor(position);
    }

    let repeated = position - floor(position / 2.0) * 2.0;
    return select(repeated, 2.0 - repeated, repeated > 1.0);
}

fn paint_texel(column: i32, row: i32, width: i32, height: i32, base: u32) -> vec4f {
    var coordinate = vec2i(column, row);
    let size = vec2i(width, height);

    // Correct negative indices when repeating; reflection uses a two-image period.
    if settings.operation.extent == 0u {
        coordinate = clamp(coordinate, vec2i(0), size - vec2i(1));
    } else if settings.operation.extent == 1u {
        coordinate = ((coordinate % size) + size) % size;
    } else {
        let period = size * 2;
        let repeated = ((coordinate % period) + period) % period;
        coordinate = min(repeated, period - vec2i(1) - repeated);
    }

    let offset = base + 4u * u32(coordinate.y * width + coordinate.x);
    return vec4f(
            operation_data.values[offset],
            operation_data.values[offset + 1u],
            operation_data.values[offset + 2u],
            operation_data.values[offset + 3u],
        );
}

// Bilinear coordinates are texel-centered; nearest sampling uses the containing texel.
fn image_level(uv: vec2f, size: vec2f, base: u32, nearest: bool) -> vec4f {
    let position = uv * size - vec2f(0.5);
    let origin = vec2i(floor(position));
    let amount = fract(position);
    if nearest {
        let coordinate = vec2i(floor(uv * size));
        return paint_texel(coordinate.x, coordinate.y, i32(size.x), i32(size.y), base);
    }

    let first = mix(
            paint_texel(origin.x, origin.y, i32(size.x), i32(size.y), base),
            paint_texel(origin.x + 1, origin.y, i32(size.x), i32(size.y), base),
            amount.x,
        );
    let second = mix(
            paint_texel(origin.x, origin.y + 1, i32(size.x), i32(size.y), base),
            paint_texel(origin.x + 1, origin.y + 1, i32(size.x), i32(size.y), base),
            amount.x,
        );
    return mix(first, second, amount.y);
}

// Conditional clips retain three independent payloads; child/ancestor division stays f32.
fn coverage_at(coordinate: vec2u) -> f32 {
    let first = bitcast<u32>(operation_data.values[0u]);
    if first == 0xfffffffeu {
        let bounds = vec4u(
                bitcast<u32>(operation_data.values[2u]),
                bitcast<u32>(operation_data.values[3u]),
                bitcast<u32>(operation_data.values[4u]),
                bitcast<u32>(operation_data.values[5u]),
            );
        if any(coordinate < bounds.xy) || any(coordinate >= bounds.zw) {
            return 0.0;
        }

        // Header: tag, total words, bounds, alpha words, child words; offsets are payload-local.
        let alpha_length = bitcast<u32>(operation_data.values[6u]);
        let child_length = bitcast<u32>(operation_data.values[7u]);
        let child_base = 8u + alpha_length;
        let ancestor_base = child_base + child_length;
        let alpha = coverage_payload_at(8u, alpha_length, coordinate);
        let numerator = coverage_payload_at(child_base, child_length, coordinate);
        let denominator = coverage_payload_at(
                ancestor_base,
                bitcast<u32>(operation_data.values[1u]) - ancestor_base,
                coordinate,
            );
        if denominator == 0.0 {
            return 0.0;
        }
        return alpha * min(numerator / denominator, 1.0);
    }

    return coverage_payload_at(0u, arrayLength(&operation_data.values), coordinate);
}

// Direct descriptors retain original frame ranges, including inside conditional clips.
fn frame_record_coverage_at(base: u32, record: u32, coordinate: vec2u) -> f32 {
    let bounds = vec4u(
            bitcast<u32>(operation_data.values[base + 2u]),
            bitcast<u32>(operation_data.values[base + 3u]),
            bitcast<u32>(operation_data.values[base + 4u]),
            bitcast<u32>(operation_data.values[base + 5u]),
        );
    if any(coordinate < bounds.xy) || any(coordinate >= bounds.zw) {
        return 0.0;
    }

    let strip_start = bitcast<u32>(operation_data.values[base + 6u]);
    let strip_count = bitcast<u32>(operation_data.values[base + 7u]) - strip_start;
    if record < strip_count {
        let offset = 4u + (strip_start + record) * 6u;
        let origin = vec2u(frame_coverage.values[offset], frame_coverage.values[offset + 1u]);
        let size = vec2u(frame_coverage.values[offset + 2u], frame_coverage.values[offset + 3u]);
        if any(coordinate < origin) || any(coordinate >= origin + size) {
            return 0.0;
        }

        let local = coordinate - origin;
        let index = frame_coverage.values[offset + 4u] + local.y * size.x + local.x;
        // The sixth strip word selects absolute f32 samples or packed CPU alpha bytes.
        if frame_coverage.values[offset + 5u] != 0u {
            return bitcast<f32>(frame_coverage.values[index]);
        }
        let packed = frame_coverage.values[frame_coverage.values[3u] + index / 4u];
        return coverage_alpha((packed >> ((index % 4u) * 8u)) & 255u);
    }

    let span_start = bitcast<u32>(operation_data.values[base + 8u]);
    let offset = 4u + frame_coverage.values[0u] * 6u + (span_start + record - strip_count) * 4u;
    let origin = vec2u(frame_coverage.values[offset], frame_coverage.values[offset + 1u]);
    let end = vec2u(
            frame_coverage.values[offset + 2u],
            origin.y + frame_coverage.values[offset + 3u],
        );
    return select(0.0, 1.0, all(coordinate >= origin) && all(coordinate < end));
}

// Coverage may be scalar, direct frame ranges, sparse records, or bounded samples.
fn coverage_payload_at(base: u32, length: u32, coordinate: vec2u) -> f32 {
    if length == 1u {
        return operation_data.values[base];
    }

    let first = bitcast<u32>(operation_data.values[base]);
    if first == 0xfffffffdu {
        let count =
            bitcast<u32>(operation_data.values[base + 7u])
            - bitcast<u32>(operation_data.values[base + 6u])
            + bitcast<u32>(operation_data.values[base + 9u])
            - bitcast<u32>(operation_data.values[base + 8u]);
        for(var record = 0u; record < count; record++) {
            let coverage = frame_record_coverage_at(base, record, coordinate);
            if coverage > 0.0 {
                return coverage;
            }
        }
        return 0.0;
    }

    if first == 0xffffffffu {
        for(var record = 0u; record < bitcast<u32>(operation_data.values[base + 6u]); record++) {
            let coverage = sparse_payload_at(base, record, coordinate);
            if coverage > 0.0 {
                return coverage;
            }
        }
        return 0.0;
    }

    let second = bitcast<u32>(operation_data.values[base + 1u]);

    // Left's high bit selects constant coverage; top's high bit selects packed bytes.
    let bounds = vec4u(
            first & 0x7fffffffu,
            second & 0x7fffffffu,
            bitcast<u32>(operation_data.values[base + 2u]),
            bitcast<u32>(operation_data.values[base + 3u]),
        );
    if any(coordinate < bounds.xy) || any(coordinate >= bounds.zw) {
        return 0.0;
    }

    if (first & 0x80000000u) != 0u {
        return operation_data.values[base + 4u];
    }

    let local = coordinate - bounds.xy;
    let index = local.y * (bounds.z - bounds.x) + local.x;
    if (second & 0x80000000u) == 0u {
        return operation_data.values[base + 4u + index];
    }

    // Four coverage bytes per word, starting with the least significant byte.
    let alpha = (bitcast<u32>(operation_data.values[base + 4u + index / 4u]) >> ((index % 4u) * 8u))
        & 255u;
    return coverage_alpha(alpha);
}

// Construct normalized f32 alpha using integer rounding and IEEE exponent bits.
fn coverage_alpha(alpha: u32) -> f32 {
    if alpha == 0u {
        return 0.0;
    }

    if alpha == 255u {
        return 1.0;
    }

    let power = 31u - countLeadingZeros(alpha);
    let significand = ((alpha << (31u - power)) + 127u) / 255u;
    return bitcast<f32>(((power + 119u) << 23u) | (significand & 0x7fffffu));
}

// Locate the paint payload appended after the coverage header and samples.
fn coverage_length() -> u32 {
    let first = bitcast<u32>(operation_data.values[0u]);
    if first == 0xffffffffu || first == 0xfffffffeu || first == 0xfffffffdu {
        return bitcast<u32>(operation_data.values[1u]);
    }

    let second = bitcast<u32>(operation_data.values[1u]);
    if (first & 0x80000000u) != 0u {
        return 5u;
    }

    let count = (bitcast<u32>(operation_data.values[2u]) - (first & 0x7fffffffu))
        * (bitcast<u32>(operation_data.values[3u]) - (second & 0x7fffffffu));
    if (second & 0x80000000u) != 0u {
        return 4u + count / 4u + min(count % 4u, 1u);
    }

    return 4u + count;
}

// Seven header words precede eight-word records: bounds, payload, source origin and row width.
fn sparse_coverage_at(record: u32, coordinate: vec2u) -> f32 {
    return sparse_payload_at(0u, record, coordinate);
}

// Sparse sample offsets are relative to the embedded payload, never the combined clip header.
fn sparse_payload_at(payload_base: u32, record: u32, coordinate: vec2u) -> f32 {
    let base = payload_base + 7u + record * 8u;
    let origin = vec2u(
            bitcast<u32>(operation_data.values[base]),
            bitcast<u32>(operation_data.values[base + 1u]),
        );
    let end = vec2u(
            bitcast<u32>(operation_data.values[base + 2u]),
            bitcast<u32>(operation_data.values[base + 3u]),
        );
    if any(coordinate < origin) || any(coordinate >= end) {
        return 0.0;
    }

    let payload = bitcast<u32>(operation_data.values[base + 4u]);

    // The all-ones payload offset denotes fully opaque coverage without sample storage.
    if payload == 0xffffffffu {
        return 1.0;
    }

    let source = vec2u(
            bitcast<u32>(operation_data.values[base + 5u]),
            bitcast<u32>(operation_data.values[base + 6u]),
        );
    let width = bitcast<u32>(operation_data.values[base + 7u]);
    let local = coordinate - source;
    let index = local.y * width + local.x;
    let alpha =
        (bitcast<u32>(operation_data.values[payload_base + payload + index / 4u])
            >> ((index % 4u) * 8u)) & 255u;
    return coverage_alpha(alpha);
}

fn fill_source(coordinate: vec2u, record: u32) -> vec4f {
    var coverage = 0.0;

    // A raster instance already identifies its sparse coverage record.
    if bitcast<u32>(operation_data.values[0u]) == 0xffffffffu {
        coverage = sparse_coverage_at(record, coordinate);
    } else if bitcast<u32>(operation_data.values[0u]) == 0xfffffffdu {
        coverage = frame_record_coverage_at(0u, record, coordinate);
    } else {
        coverage = coverage_at(coordinate);
    }

    if coverage == 0.0 {
        return vec4f(0.0);
    }

    var color = vec4f(value(0u), value(1u), value(2u), value(3u));
    if settings.operation.kind != 0u {
        color = spatial_paint(coordinate);
    }

    return checked(color * coverage);
}

// Evaluate paints at pixel centers after mapping global coordinates into paint space.
fn spatial_paint(coordinate: vec2u) -> vec4f {
    let point = vec2f(coordinate) + vec2f(0.5) - vec2f(value(14u), value(15u));
    var local = vec2f(
            value(0u) * point.x + value(2u) * point.y + value(4u),
            value(1u) * point.x + value(3u) * point.y + value(5u),
        );
    let base = coverage_length();
    let geometry = vec4f(value(10u), value(11u), value(12u), value(13u));

    // Pattern data contains two image levels; a zero blend reads only the first.
    if settings.operation.kind == 4u {
        let size = geometry.xy;
        let uv = vec2f(
                extend_paint(local.x / size.x, settings.operation.extent),
                extend_paint(local.y / size.y, settings.operation.extent),
            );
        let first_size = vec2f(operation_data.values[base], operation_data.values[base + 1u]);
        let second_size = vec2f(operation_data.values[base + 2u], operation_data.values[base + 3u]);
        let blend = operation_data.values[base + 4u];
        let first = image_level(uv, first_size, base + 6u, operation_data.values[base + 5u] != 0.0);
        if blend == 0.0 {
            return first;
        }

        let second = image_level(
                uv,
                second_size,
                base + 6u + u32(first_size.x * first_size.y) * 4u,
                false,
            );
        return checked(mix(first, second, blend));
    }

    if settings.operation.detail == 0u {
        local = (local - vec2f(value(6u), value(7u))) / vec2f(value(8u), value(9u));
    }

    var position = 0.0;
    if settings.operation.kind == 1u {
        let direction = geometry.zw - geometry.xy;
        position = dot(local - geometry.xy, direction) / dot(direction, direction);
    } else if settings.operation.kind == 2u {
        position = distance(local, geometry.xy) / geometry.z;
    } else if settings.operation.kind == 5u {
        let focus = vec2f(value(16u), value(17u));
        let offset = local - focus;
        let displacement = focus - geometry.xy;
        let square = dot(offset, offset);
        position = 0.0;

        // The focal radial position follows the ray from focus to the circle boundary.
        if square != 0.0 {
            let projection = dot(offset, displacement);
            position = square / (-projection + sqrt(
                        projection * projection
                        + square * (geometry.z * geometry.z - dot(displacement, displacement)),
                    ));
        }
    } else {
        var angle = 0.0;
        if any(local != geometry.xy) {
            angle = atan2(local.y - geometry.y, local.x - geometry.x);
        }

        angle -= geometry.z;
        position = (angle - floor(angle / 6.283185307179586) * 6.283185307179586)
            / (geometry.w - geometry.z);
    }

    position = extend_paint(position, settings.operation.extent);

    // Find the first stop above the position before interpolating neighboring table samples.
    let count = u32(operation_data.values[base]);
    var upper = 0u;
    loop {
        if upper >= count {
            break;
        }
        if operation_data.values[base + 1u + upper] > position {
            break;
        }
        upper += 1u;
    }

    let colors = base + 1u + count;
    var entry = 0u;
    var amount = 0.0;
    if upper == count {
        entry = (count - 1u) * 257u - 1u;
    } else {
        let lower = upper - 1u;
        let start = operation_data.values[base + 1u + lower];
        let end = operation_data.values[base + 1u + upper];
        let sample = clamp((position - start) / (end - start), 0.0, 1.0) * 256.0;
        let index = min(u32(sample), 255u);
        amount = sample - f32(index);
        entry = lower * 257u + index;
    }

    let offset = colors + entry * 4u;
    let first = vec4f(
            operation_data.values[offset],
            operation_data.values[offset + 1u],
            operation_data.values[offset + 2u],
            operation_data.values[offset + 3u],
        );
    if upper == count {
        return first;
    }

    let second = vec4f(
            operation_data.values[offset + 4u],
            operation_data.values[offset + 5u],
            operation_data.values[offset + 6u],
            operation_data.values[offset + 7u],
        );
    return mix(first, second, amount);
}

// Operation IDs follow host GpuOp; helpers keep cropped image reads in global coordinates.
fn optical_height(position: vec2f) -> f32 {
    let sampled = sample_pixel(position);
    if value(1u) != 0.0 {
        return sampled.a;
    }
    return dot(straight(sampled).rgb, vec3f(0.2126, 0.7152, 0.0722));
}

fn optical_pixel(position: vec2f, pixel: vec4f) -> vec4f {
    let kind = u32(value(0u));
    if kind == 5u {
        let center = vec2f(value(1u), value(2u));
        let delta = position + vec2f(0.5) - center;
        var result = pixel;
        let count = u32(value(3u));
        let tint = vec3f(value(6u), value(7u), value(8u));
        for(var ghost = 0u; ghost < count; ghost++) {
            let scale = -(0.25 + f32(ghost) * 0.3);
            let sampled = sample_pixel(center + delta / scale - vec2f(0.5));
            let luma = dot(straight(sampled).rgb, vec3f(0.2126, 0.7152, 0.0722));
            let extracted = max(luma - value(4u), 0.0) / max(luma, 1e-6);
            let ring = clamp(1.0 - abs(length(delta) / value(9u) - 0.55) / 0.08, 0.0, 1.0);
            let weight = value(5u) * extracted / f32(count) * (0.75 + ring * 0.25);
            result = vec4f(
                    result.rgb + sampled.rgb * tint * weight,
                    min(1.0, result.a + sampled.a * value(10u) * weight),
                );
        }
        return result;
    }
    if kind == 0u {
        if value(1u) == 0.0 && value(2u) == 0.0 {
            return pixel;
        }
        let center = vec2f(value(4u), value(5u));
        var delta = position + vec2f(0.5) - center;
        if value(3u) != 0.0 {
            delta /= (1.0 + max(value(1u), 0.0)) * (1.0 + abs(value(2u)));
        }
        let squared = dot(delta, delta) / (value(6u) * value(6u));
        var result = vec4f(0.0);
        for(var channel = 0u; channel < 3u; channel++) {
            let scale = (1.0 + value(1u) * squared)
                * (1.0 + (f32(channel) - 1.0) * value(2u) * squared);
            let sampled = sample_pixel(center + delta * scale - vec2f(0.5));
            result[channel] = sampled[channel];
            result.a = max(result.a, sampled.a);
        }
        return result;
    }
    if kind == 1u {
        let radius = max(value(6u), value(7u));
        let axes = vec2f(value(6u), value(7u))
            + (vec2f(radius) - vec2f(value(6u), value(7u))) * value(3u);
        let relative = (position + vec2f(0.5) - vec2f(value(4u), value(5u))) / axes;
        let cosine_squared = 1.0 / (1.0 + dot(relative, relative) * value(1u) * value(1u));
        let factor = 1.0 - value(2u) + value(2u) * cosine_squared * cosine_squared;
        return vec4f(pixel.rgb * factor, pixel.a);
    }
    if kind == 2u {
        if value(1u) == 0.0 {
            return pixel;
        }
        var result = vec4f(0.0);
        var total = 0.0;
        // Rust supplies the same aperture offsets to both backends; value(3) is the
        // radius-scaled tap count (8..=64) and the remaining uploaded offsets are zero.
        let count = min(u32(value(3u)), 64u);
        for(var tap = 0u; tap < count; tap++) {
            let offset = vec2f(
                    operation_data.values[tap * 2u],
                    operation_data.values[tap * 2u + 1u],
                );
            let sampled = sample_pixel(position + offset);
            let luma = dot(straight(sampled).rgb, vec3f(0.2126, 0.7152, 0.0722));
            let weight = 1.0 + max(luma, 0.0) * value(2u);
            result += sampled * weight;
            total += weight;
        }
        return result / total;
    }
    if kind == 3u {
        var result = vec4f(0.0);
        var total = 0.0;
        var weight = 1.0;
        let count = u32(value(4u));
        for(var tap = 0u; tap < count; tap++) {
            let fraction = f32(tap) / f32(count) * value(3u);
            let sampled = sample_pixel(
                    position + (vec2f(value(1u), value(2u)) - position - vec2f(0.5)) * fraction,
                );
            result += sampled * weight;
            total += weight;
            weight *= value(5u);
        }
        return result / total;
    }
    var gradient = vec2f(0.0);
    for(var vertical = -1; vertical <= 1; vertical++) {
        for(var horizontal = -1; horizontal <= 1; horizontal++) {
            let height = optical_height(position + vec2f(f32(horizontal), f32(vertical)));
            gradient.x += height * f32(horizontal) * select(1.0, 2.0, vertical == 0) / 8.0;
            gradient.y += height * f32(vertical) * select(1.0, 2.0, horizontal == 0) / 8.0;
        }
    }
    let normal = normalize(vec3f(-gradient, 1.0));
    let light = vec3f(value(2u), value(3u), value(4u));
    let diffuse = max(dot(normal, light), 0.0);
    let half_vector = light + vec3f(0.0, 0.0, 1.0);
    var highlight = 0.0;
    if dot(half_vector, half_vector) > 0.0 {
        highlight = pow(max(dot(normal, normalize(half_vector)), 0.0), 32.0) * value(9u);
    }
    let color = vec3f(value(5u), value(6u), value(7u));
    return vec4f(
            pixel.rgb * (vec3f(value(8u)) + color * diffuse) + pixel.a * color * highlight,
            pixel.a,
        );
}

fn evaluate(coordinate: vec2u, index: u32, operation: u32) -> vec4f {
    let position = vec2f(coordinate);
    let pixel = first_pixel(coordinate);

    switch operation {
        case OP_PARTICLES { return particle_batch(pixel, coordinate); }
        case OP_DOWNSAMPLE {
            // Bloom pyramid: coarse pixel c averages the 2x2 block at 2c of the finer level, in
            // device coordinates halved once per level.
            let origin = vec2i(coordinate) * 2;
            return 0.25 * (at_pixel(origin) + at_pixel(origin + vec2i(1, 0))
                + at_pixel(origin + vec2i(0, 1)) + at_pixel(origin + vec2i(1, 1)));
        }
        case OP_UPSAMPLE {
            // Bloom pyramid: the weighted finer level plus the coarser sum, sampled bilinearly at
            // this pixel's center mapped into the coarser grid.
            return pixel * value(0u) + second_sample((position + vec2f(0.5)) * 0.5 - vec2f(0.5));
        }
        case OP_CLEAR { return vec4f(value(0u), value(1u), value(2u), value(3u)); }
        case OP_FILL {
            let coverage = coverage_at(coordinate);
            if coverage == 0.0 {
                return pixel;
            }

            var color = vec4f(value(0u), value(1u), value(2u), value(3u));
            if settings.operation.kind != 0u {
                color = spatial_paint(coordinate);
            }

            return composite(pixel, color * coverage, 0u);
        }
        case OP_COMPOSITE {
            return composite(
                    pixel,
                    second_pixel(coordinate) * (value(0u) * coverage_at(coordinate)),
                    settings.operation.kind,
                );
        }
        case OP_MASK {
            let mask = second_pixel(coordinate);
            var coverage = mask.a;

            // Luminance masks use premultiplied RGB, so mask alpha already contributes.
            if settings.operation.detail >= 2u {
                coverage = dot(mask.rgb, vec3f(0.2126, 0.7152, 0.0722));
            }

            if mask.a == 0.0 {
                coverage = 0.0;
            }

            coverage = clamp(coverage, 0.0, 1.0);
            if (settings.operation.detail & 1u) != 0u {
                coverage = 1.0 - coverage;
            }
            return pixel * coverage;
        }
        case OP_BLUR {
            // One separable blur pass uses the uploaded kernel along the selected axis.
            var result = vec4f(0.0);
            for(var tap = 0u; tap <= settings.operation.extent * 2u; tap++) {
                let displacement = i32(tap) - i32(settings.operation.extent);
                let delta = select(
                        vec2i(0, displacement),
                        vec2i(displacement, 0),
                        settings.operation.detail != 0u,
                    );
                result += at_pixel(vec2i(coordinate) + delta) * operation_data.values[tap];
            }
            return result;
        }
        case OP_OFFSET {
            // Affine inverse maps global pixel centers; transparent borders remain unchanged.
            if value(31u) != 0.0 {
                let center = position + vec2f(0.5);
                let mapped = vec2f(
                        value(0u) * center.x + value(2u) * center.y + value(4u),
                        value(1u) * center.x + value(3u) * center.y + value(5u),
                    );
                return sample_pixel(mapped - vec2f(0.5));
            }
            return sample_pixel(position - vec2f(value(0u), value(1u)));
        }
        case OP_COLOR_MATRIX {
            // Apply the 4x5 matrix to straight RGBA, then clamp alpha and premultiply.
            let source = straight(pixel);
            var result = vec4f(0.0);
            for(var channel = 0u; channel < 4u; channel++) {
                result[channel] = value(channel * 5u + 4u);
                for(var component = 0u; component < 4u; component++) {
                    result[channel] += value(channel * 5u + component) * source[component];
                }
            }
            return premultiplied(result);
        }
        case OP_THRESHOLD {
            let luminance = dot(straight(pixel).rgb, vec3f(0.2126, 0.7152, 0.0722));
            // Bloom soft knee caps extracted energy, not the original source pixel.
            if value(2u) != 0.0 {
                let limited = min(max(luminance, 0.0), 1000.0);
                let soft = clamp(limited - value(0u) + value(1u), 0.0, 2.0 * value(1u));
                let contribution =
                    max(max(soft * soft / (4.0 * value(1u) + 1e-6), limited - value(0u)), 0.0)
                    / max(luminance, 1e-6);
                return pixel * contribution;
            }
            if luminance > value(0u) {
                return pixel * (1.0 - value(0u) / luminance);
            }
            return vec4f(0.0);
        }
        case OP_NOISE {
            let source = straight(pixel);
            return premultiplied(vec4f(source.rgb + vec3f(noise_delta(coordinate)), source.a));
        }
        case OP_CHANNEL_OFFSET {
            let red = sample_pixel(position - vec2f(value(0u), value(1u)));
            let green = sample_pixel(position - vec2f(value(2u), value(3u)));
            let blue = sample_pixel(position - vec2f(value(4u), value(5u)));
            return vec4f(red.r, green.g, blue.b, max(red.a, max(green.a, blue.a)));
        }
        case OP_COLOR_ADJUST {
            // Color adjustments operate on straight RGB before alpha is reapplied.
            let source = straight(pixel);
            var rgb = checked(
                    vec4f(
                        (source.rgb * value(0u) - vec3f(0.18)) * value(1u) + vec3f(0.18),
                        source.a,
                    ),
                ).rgb;
            let luminance = dot(rgb, vec3f(0.2126, 0.7152, 0.0722));
            rgb = checked(vec4f(vec3f(luminance) + (rgb - vec3f(luminance)) * value(2u), source.a))
                    .rgb;
            let gray = (rgb.r + rgb.g + rgb.b) / 3.0;
            rgb = rgb * value(3u) + (rgb.brg - rgb.gbr) * value(4u)
                + vec3f(gray * (1.0 - value(3u)));
            return premultiplied(vec4f(rgb, source.a));
        }
        case OP_COLORIZE { return vec4f(value(0u), value(1u), value(2u), value(3u)) * pixel.a; }
        case OP_SCALE {
            let result = checked(pixel * value(0u));
            return vec4f(result.rgb, clamp(result.a, 0.0, 1.0));
        }
        case OP_CDL {
            let source = straight(pixel);
            var rgb = source.rgb;
            let slope = vec3f(value(0u), value(1u), value(2u));
            let offset = vec3f(value(4u), value(5u), value(6u));
            let power = vec3f(value(8u), value(9u), value(10u));
            if any(slope != vec3f(1.0)) || any(offset != vec3f(0.0)) || any(power != vec3f(1.0))
                || value(12u) != 1.0 {
                rgb = pow(max(rgb * slope + offset, vec3f(0.0)), power);
            }
            let luma = dot(rgb, vec3f(0.2126, 0.7152, 0.0722));
            if value(12u) != 1.0 {
                rgb = vec3f(luma) + (rgb - vec3f(luma)) * value(12u);
            }
            return premultiplied(vec4f(rgb, source.a));
        }
        case OP_LUT {
            let source = straight(pixel);
            return premultiplied(vec4f(cube_apply(source.rgb, 0u), source.a));
        }
        case OP_OPTICS { return optical_pixel(position, pixel); }
        default { return pixel; }
    }
}

// Header word 3 packs the record stride in the low five bits and the InstanceBlend above them.
fn particle_stride() -> u32 {
    return u32(operation_data.values[3u]) & 31u;
}

// Maps InstanceBlend (SourceOver, Plus, Screen, Multiply) to the shared blend-mode constants.
fn particle_blend() -> u32 {
    switch u32(operation_data.values[3u]) >> 5u {
        case 1u { return 17u; }
        case 2u { return 3u; }
        case 3u { return 2u; }
        default { return 0u; }
    }
}

// Input records follow four header words and contain twelve or sixteen words.
// Rust has already evaluated trajectories and sampled both endpoints of each trail.
// Output uses three vec4 slots: position/velocity, trail start, and support bounds.
@compute @workgroup_size(64)
fn prepare_particle_states(@builtin(global_invocation_id) invocation: vec3u) {
    let index = invocation.x;
    if index >= u32(operation_data.values[0u]) {
        return;
    }

    // Four header words precede twelve-word evaluated primitive records.
    let base = 4u + index * particle_stride();
    let state = checked(
            vec4f(
                operation_data.values[base],
                operation_data.values[base + 1u],
                operation_data.values[base + 2u],
                operation_data.values[base + 3u],
            ),
        );
    let start = state.zw;

    let kind = u32(operation_data.values[base + 11u]);
    if kind == 3u || kind == 5u || kind == 12u {
        let delta = state.xy - start;
        _ = checked(vec4f(delta, dot(delta, delta), 0.0));
    }

    output_pixels.values[index * 3u] = state;
    output_pixels.values[index * 3u + 1u] = checked(vec4f(start, 0.0, 0.0));
    output_pixels.values[index * 3u + 2u] = particle_support(index);
}

fn particle_support(index: u32) -> vec4f {
    let base = 4u + index * particle_stride();
    let state = output_pixels.values[index * 3u];
    let radius = operation_data.values[base + 8u];
    let kind = u32(operation_data.values[base + 11u]);

    // An inverted box marks particles with no visible support.
    let line = kind == 3u || kind == 5u || kind == 12u;
    var start_alpha = 0.0;
    if kind == 5u || kind == 12u {
        start_alpha = operation_data.values[base + 15u];
    }
    if (radius == 0.0 && (!line || operation_data.values[base + 10u] == 0.0))
        || (operation_data.values[base + 7u] == 0.0 && start_alpha == 0.0) {
        return vec4f(1.0, 1.0, -1.0, -1.0);
    }

    var low = state.xy;
    var high = state.xy;
    var reach = radius;
    if line {
        let start = output_pixels.values[index * 3u + 1u].xy;
        low = min(start, state.xy);
        high = max(start, state.xy);
        reach = max(radius, operation_data.values[base + 10u] * 0.5);
    } else if kind == 1u {
        reach *= 3.0;
    } else if kind == 2u {
        reach *= 2.0;
        if particle_stride() == 16u {
            reach *= max(1.0, operation_data.values[base + 14u]);
        }
    } else if kind == 4u {
        reach *= max(1.0, operation_data.values[base + 10u]) * 2.0;
    } else if kind == 8u || kind == 10u {
        reach *= max(1.0, operation_data.values[base + 14u]) * 2.0;
    } else if particle_stride() == 16u {
        reach *= 2.0;
    }

    // Pad support for coordinate magnitude and antialiasing instead of testing the exact edge.
    let guard = max(max(abs(low), abs(high)), vec2f(1.0)) * 0.000244140625
        + vec2f(abs(value(6u)) * 4.0 + 0.01);
    return vec4f(low - vec2f(reach) - guard, high + vec2f(reach) + guard);
}

// Tiles are 16x16 output pixels; the last row and column may be partial.
fn particle_tile_count() -> u32 {
    let extent = vec2u(
            settings.sources[2].z - settings.sources[2].x,
            settings.sources[2].w - settings.sources[2].y,
        );
    return ((extent.x + 15u) / 16u) * ((extent.y + 15u) / 16u);
}

fn particle_tile_rect(tile: u32) -> vec4u {
    let columns = (settings.sources[2].z - settings.sources[2].x + 15u) / 16u;
    let low = vec2u(settings.sources[2].x, settings.sources[2].y)
        + vec2u(tile % columns, tile / columns) * 16u;
    let high = min(low + vec2u(16u), vec2u(settings.sources[2].z, settings.sources[2].w));
    return vec4u(low, high);
}

fn particle_tile_support(tile: u32) -> vec4f {
    // Transform the four extreme pixel centers into particle space before building the box.
    let rect = particle_tile_rect(tile);
    let low = vec2f(rect.xy) + vec2f(0.5);
    let high = vec2f(rect.zw) - vec2f(0.5);
    let first = checked(
            vec4f(
                value(0u) * low.x + value(2u) * low.y + value(4u),
                value(1u) * low.x + value(3u) * low.y + value(5u),
                0.0,
                0.0,
            ),
        ).xy;
    let second = checked(
            vec4f(
                value(0u) * high.x + value(2u) * low.y + value(4u),
                value(1u) * high.x + value(3u) * low.y + value(5u),
                0.0,
                0.0,
            ),
        ).xy;
    let third = checked(
            vec4f(
                value(0u) * low.x + value(2u) * high.y + value(4u),
                value(1u) * low.x + value(3u) * high.y + value(5u),
                0.0,
                0.0,
            ),
        ).xy;
    let fourth = checked(
            vec4f(
                value(0u) * high.x + value(2u) * high.y + value(4u),
                value(1u) * high.x + value(3u) * high.y + value(5u),
                0.0,
                0.0,
            ),
        ).xy;

    let lower = min(min(first, second), min(third, fourth));
    let upper = max(max(first, second), max(third, fourth));
    let device_extent = max(abs(low), abs(high));
    let arithmetic_scale = vec2f(
            abs(value(0u)) * device_extent.x + abs(value(2u)) * device_extent.y + abs(value(4u)),
            abs(value(1u)) * device_extent.x + abs(value(3u)) * device_extent.y + abs(value(5u)),
        );

    // Include transform arithmetic scale in the padding, not just the resulting coordinates.
    let guard = max(max(max(abs(lower), abs(upper)), arithmetic_scale), vec2f(1.0)) * 0.000244140625
        + vec2f(0.01);
    return vec4f(lower - guard, upper + guard);
}

fn particle_intersects(support: vec4f, tile: vec4f) -> bool {
    return all(support.xy <= support.zw) && all(support.xy <= tile.zw)
        && all(tile.xy <= support.zw);
}

// One invocation owns each tile's metadata vec4; integer fields travel as f32 bit patterns.
@compute @workgroup_size(64)
fn count_particle_tiles(@builtin(global_invocation_id) invocation: vec3u) {
    let tile = invocation.x;
    if tile >= particle_tile_count() {
        return;
    }

    let count = u32(operation_data.values[0u]);
    let area = particle_tile_support(tile);
    var matches = 0u;
    for(var index = 0u; index < count; index++) {
        matches += u32(particle_intersects(output_pixels.values[index * 3u + 2u], area));
    }

    output_pixels.values[count * 3u + tile] = bitcast<vec4f>(vec4u(matches, 0u, 0u, 0u));
}

// Serial exclusive prefixes assign each tile a disjoint range in the index buffer.
@compute @workgroup_size(1)
fn prefix_particle_tiles() {
    if atomicLoad(&failure_status.code) != 0u {
        return;
    }

    let count = u32(operation_data.values[0u]);
    let tiles = particle_tile_count();
    var total = 0u;
    var work = vec2u(0u);
    var overflow = 0u;
    for(var tile = 0u; tile < tiles; tile++) {
        let matches = bitcast<u32>(output_pixels.values[count * 3u + tile].x);
        output_pixels.values[count * 3u + tile].y = bitcast<f32>(total);
        let next = total + matches;
        overflow |= u32(next < total);
        total = next;

        let rect = particle_tile_rect(tile);
        let pixels = (rect.z - rect.x) * (rect.w - rect.y);
        let amount = pixels * (matches * 64u + 1u);
        let low = work.x + amount;
        work.y += u32(low < work.x);
        work.x = low;
    }

    // The host reads total references, work low/high words, and the prefix overflow flag.
    output_pixels.values[count * 3u + tiles] = bitcast<vec4f>(vec4u(total, work, overflow));
}

// Scan particles in original order so tiled compositing preserves that order.
@compute @workgroup_size(64)
fn fill_particle_tiles(@builtin(global_invocation_id) invocation: vec3u) {
    let tile = invocation.x;
    if tile >= particle_tile_count() {
        return;
    }

    let count = u32(operation_data.values[0u]);
    let area = particle_tile_support(tile);
    var cursor = bitcast<u32>(output_pixels.values[count * 3u + tile].y);
    let base = count * 3u + particle_tile_count() + 1u;
    for(var index = 0u; index < count; index++) {
        if particle_intersects(output_pixels.values[index * 3u + 2u], area) {
            output_pixels.values[base + cursor] = bitcast<vec4f>(vec4u(index, 0u, 0u, 0u));
            cursor += 1u;
        }
    }
}

fn particle_texel(column: i32, row: i32, base: u32, width: i32, height: i32) -> vec4f {
    let offset = base + u32(clamp(row, 0, height - 1) * width + clamp(column, 0, width - 1)) * 4u;
    return vec4f(
            operation_data.values[offset],
            operation_data.values[offset + 1u],
            operation_data.values[offset + 2u],
            operation_data.values[offset + 3u],
        );
}

// Sprite RGBA follows the particle records and is bilinearly sampled at texel centers.
fn particle_sprite(uv: vec2f, count: u32, cell: vec4f) -> vec4f {
    let width = i32(operation_data.values[1u]);
    let height = i32(operation_data.values[2u]);
    let dimensions = vec2f(f32(width), f32(height));
    var position = uv * dimensions - vec2f(0.5);
    if particle_stride() == 16u {
        position = clamp(position, cell.xy * dimensions, cell.zw * dimensions - vec2f(1.0));
    }
    let origin = vec2i(floor(position));
    let fraction = position - floor(position);
    let base = 4u + count * particle_stride();
    let high = min(origin + vec2i(1), vec2i(cell.zw * dimensions - vec2f(1.0)));
    let top_left = particle_texel(origin.x, origin.y, base, width, height);
    let top_right = particle_texel(high.x, origin.y, base, width, height);
    let bottom_left = particle_texel(origin.x, high.y, base, width, height);
    let bottom_right = particle_texel(high.x, high.y, base, width, height);
    return (top_left * (1.0 - fraction.x) + top_right * fraction.x) * (1.0 - fraction.y)
        + (bottom_left * (1.0 - fraction.x) + bottom_right * fraction.x) * fraction.y;
}

fn particle_batch(backdrop: vec4f, coordinate: vec2u) -> vec4f {
    let device = vec2f(coordinate) + vec2f(0.5);
    let point = vec2f(
            value(0u) * device.x + value(2u) * device.y + value(4u),
            value(1u) * device.x + value(3u) * device.y + value(5u),
        );
    let antialias = value(6u);
    let count = u32(operation_data.values[0u]);
    var result = backdrop;
    var metadata = vec4u(count, 0u, 0u, 0u);

    // Untiled mode visits every particle; tiled mode reads count/offset metadata from the state buffer.
    if PARTICLE_TILES {
        let origin = vec2u(settings.sources[2].x, settings.sources[2].y);
        let columns = (settings.sources[2].z - origin.x + 15u) / 16u;
        let tile = ((coordinate.y - origin.y) / 16u) * columns + (coordinate.x - origin.x) / 16u;
        metadata = bitcast<vec4u>(second_pixels.values[count * 3u + tile]);
    }

    let indices = count * 3u + particle_tile_count() + 1u;
    for(var cursor = 0u; cursor < metadata.x; cursor++) {
        var index = cursor;
        if PARTICLE_TILES {
            index = bitcast<u32>(second_pixels.values[indices + metadata.y + cursor].x);
        }

        let base = 4u + index * particle_stride();
        let kind = u32(operation_data.values[base + 11u]);
        let state = second_pixels.values[index * 3u];
        let position = state.xy;
        let radius = operation_data.values[base + 8u];
        var color = vec4f(
                operation_data.values[base + 4u],
                operation_data.values[base + 5u],
                operation_data.values[base + 6u],
                operation_data.values[base + 7u],
            );
        var extra = vec4f(0.0, 0.0, 1.0, 0.0);
        if particle_stride() == 16u {
            extra = vec4f(
                    operation_data.values[base + 12u],
                    operation_data.values[base + 13u],
                    operation_data.values[base + 14u],
                    operation_data.values[base + 15u],
                );
        }
        let line = kind == 3u || kind == 5u || kind == 12u;
        if (radius == 0.0 && (!line || operation_data.values[base + 10u] == 0.0))
            || (color.a == 0.0 && (!line || extra.a == 0.0)) {
            continue;
        }

        let relative = point - position;
        var coverage: f32;
        if line {
            // A trail uses distance to the clamped segment between its start and current position.
            let start = second_pixels.values[index * 3u + 1u].xy;
            let delta = position - start;
            let square = dot(delta, delta);
            var progress = 0.0;
            var raw_progress = 0.0;
            if square != 0.0 {
                raw_progress = dot(point - start, delta) / square;
                progress = clamp(raw_progress, 0.0, 1.0);
            }
            var width = operation_data.values[base + 10u] * 0.5;
            if kind == 5u || kind == 12u {
                width = mix(width, radius, progress);
                color = mix(extra, color, progress);
            }
            coverage = clamp(
                    (width - distance(point, start + delta * progress)) / antialias + 0.5,
                    0.0,
                    1.0,
                );
            if kind == 12u && (raw_progress < 0.0 || raw_progress > 1.0) {
                coverage = 0.0;
            }
        } else if kind == 1u {
            let square = dot(relative, relative) / (radius * radius);
            coverage = select(exp(-2.0 * square), 0.0, square > 9.0);
        } else if kind == 2u || kind == 4u {
            let angle = operation_data.values[base + 9u];
            let cosine = cos(angle);
            let sine = sin(angle);
            let local = vec2f(
                    relative.x * cosine + relative.y * sine,
                    -relative.x * sine + relative.y * cosine,
                );
            var aspect = 1.0;
            if kind == 4u {
                aspect = max(operation_data.values[base + 10u], 0.000001);
            } else if particle_stride() == 16u {
                aspect = max(extra.z, 1.0);
            }
            if any(abs(local) >= vec2f(radius * aspect, radius)) {
                coverage = 0.0;
            } else {
                let cell = select(vec4f(0.0, 0.0, 1.0, 1.0), extra, kind == 4u);
                let uv = local / (2.0 * vec2f(radius * aspect, radius)) + vec2f(0.5);
                color *= particle_sprite(cell.xy + uv * (cell.zw - cell.xy), count, cell);
                coverage = 1.0;
            }
        } else if kind >= 6u && kind <= 11u {
            let angle = operation_data.values[base + 9u];
            let local = vec2f(
                    relative.x * cos(angle) + relative.y * sin(angle),
                    -relative.x * sin(angle) + relative.y * cos(angle),
                );
            var sdf = length(local) - radius;
            if kind == 6u || (kind == 11u && extra.x >= 3.0) {
                let sector = 6.283185307179586 / extra.x;
                let polar = atan2(local.y, local.x);
                sdf = cos(floor(polar / sector + 0.5) * sector - polar) * length(local)
                    - radius * cos(3.141592653589793 / extra.x);
            } else if kind == 7u {
                sdf = abs(length(local) - radius * (1.0 - extra.x * 0.5)) - radius * extra.x * 0.5;
            } else if kind == 8u {
                sdf = length(vec2f(max(abs(local.x) - radius * (extra.z - 1.0), 0.0), local.y))
                    - radius;
            } else if kind == 10u {
                sdf = max(abs(local.x) - radius * extra.z, abs(local.y) - radius);
            }
            coverage = clamp(-sdf / max(antialias, 0.000000000001) + 0.5, 0.0, 1.0);
            if kind == 9u {
                coverage *= clamp(
                        (radius - length(local)) / max(radius * (1.0 - extra.x), antialias),
                        0.0,
                        1.0,
                    );
            }
            if kind == 11u {
                let rim =
                    exp(-pow((length(local) / max(radius, 0.000000000001) - 0.9) * 12.0, 2.0));
                color = vec4f(color.rgb * (1.0 + extra.y * rim) / (1.0 + extra.y * 0.3), color.a);
                coverage *= clamp(-sdf / max((1.0 - extra.z) * radius, antialias) + 0.5, 0.0, 1.0);
            }
        } else {
            coverage = clamp((radius - length(relative)) / antialias + 0.5, 0.0, 1.0);
        }

        // The batch-wide blend rule applies to every primitive; Plus is the always-linear additive mode.
        result = composite(result, color * coverage, particle_blend());
    }

    return checked(result);
}

struct StripVertex {
    @builtin(position) position: vec4f,
    @interpolate(flat)
    @location(0) record: u32,
}

// Each instance supplies a global rectangle expanded into two triangles.
@vertex
fn strip_vertex(
    @builtin(vertex_index) vertex: u32,
    @builtin(instance_index) record: u32,
    @location(0) rect: vec4f,
) -> StripVertex {
    let corners = array<vec2f, 6>(
            vec2f(0.0, 0.0),
            vec2f(1.0, 0.0),
            vec2f(0.0, 1.0),
            vec2f(0.0, 1.0),
            vec2f(1.0, 0.0),
            vec2f(1.0, 1.0),
        );

    var point = rect.xy + (rect.zw - rect.xy) * corners[vertex];
    var origin = vec2u(settings.sources[2].x, settings.sources[2].y);
    var end = vec2u(settings.sources[2].z, settings.sources[2].w);

    // Composite and mask capture targets use operation bounds rather than output bounds.
    if settings.operation.code == OP_COMPOSITE || settings.operation.code == OP_MASK {
        origin = vec2u(settings.bounds.x, settings.bounds.y);
        end = vec2u(settings.bounds.z, settings.bounds.w);
    }

    point -= vec2f(origin);
    let extent = vec2f(max(end - origin, vec2u(1u)));

    // Convert target-local pixels to clip space and flip the top-down image Y axis.
    return StripVertex(
            vec4f(point.x / extent.x * 2.0 - 1.0, 1.0 - point.y / extent.y * 2.0, 0.0, 1.0),
            record,
        );
}

// The flat instance record selects sparse coverage without interpolation across the rectangle.
@fragment
fn strip_fragment(
    @builtin(position) position: vec4f,
    @interpolate(flat)
    @location(0) record: u32,
) -> @location(0) vec4f {
    let coordinate = vec2u(position.xy) + vec2u(settings.sources[2].x, settings.sources[2].y);
    return checked(composite(first_pixel(coordinate), fill_source(coordinate, record), 0u));
}

// Capture coordinates are relative to the operation rectangle, not the final output bounds.
@fragment
fn layer_capture(@builtin(position) position: vec4f) -> @location(0) vec4f {
    let coordinate = vec2u(position.xy) + vec2u(settings.bounds.x, settings.bounds.y);
    return checked(second_pixel(coordinate));
}

// Seed a batch target from the first input before drawing its source instances.
@fragment
fn batch_backdrop(@builtin(position) position: vec4f) -> @location(0) vec4f {
    let coordinate = vec2u(position.xy) + vec2u(settings.sources[2].x, settings.sources[2].y);
    return checked(first_pixel(coordinate));
}

@fragment
fn batch_source(
    @builtin(position) position: vec4f,
    @interpolate(flat)
    @location(0) record: u32,
) -> @location(0) vec4f {
    let coordinate = vec2u(position.xy) + vec2u(settings.sources[2].x, settings.sources[2].y);
    return fill_source(coordinate, record);
}

// Copy the batch texture into a tightly packed buffer; each local invocation owns one element.
@compute @workgroup_size(8, 8)
fn batch_resolve(@builtin(global_invocation_id) invocation: vec3u) {
    let extent = max(
            vec2u(
                settings.sources[2].z - settings.sources[2].x,
                settings.sources[2].w - settings.sources[2].y,
            ),
            vec2u(1u),
        );

    if any(invocation.xy >= extent) {
        return;
    }

    output_pixels.values[invocation.y * extent.x + invocation.x] =
        checked(textureLoad(layer_texture, vec2i(invocation.xy), 0));
}

// Combine a layer texture with the first input while preserving pixels outside operation bounds.
@compute @workgroup_size(8, 8)
fn layer_main(@builtin(global_invocation_id) invocation: vec3u) {
    let extent = max(
            vec2u(
                settings.sources[2].z - settings.sources[2].x,
                settings.sources[2].w - settings.sources[2].y,
            ),
            vec2u(1u),
        );

    if any(invocation.xy >= extent) {
        return;
    }

    let coordinate = invocation.xy + vec2u(settings.sources[2].x, settings.sources[2].y);
    let index = coordinate.y * settings.frame.width + coordinate.x;
    let pixel = first_pixel(coordinate);

    var result = pixel;

    if all(coordinate >= vec2u(settings.bounds.x, settings.bounds.y))
        && all(coordinate < vec2u(settings.bounds.z, settings.bounds.w)) {
        var source = textureLoad(
                layer_texture,
                vec2i(coordinate - vec2u(settings.bounds.x, settings.bounds.y)),
                0,
            );
        let full_precision = second_pixel(coordinate);

        // Restore buffer alpha while keeping the captured texture's RGB/alpha ratio.
        if source.a > 0.0 {
            source = vec4f(source.rgb * (full_precision.a / source.a), full_precision.a);
        } else {
            source = full_precision;
        }

        if settings.operation.code == OP_COMPOSITE {
            result = composite(
                    pixel,
                    source * (value(0u) * coverage_at(coordinate)),
                    settings.operation.kind,
                );
        } else {
            let mask = full_precision;
            var coverage = mask.a;
            if settings.operation.detail >= 2u {
                coverage = dot(mask.rgb, vec3f(0.2126, 0.7152, 0.0722));
            }

            if mask.a == 0.0 {
                coverage = 0.0;
            }

            coverage = clamp(coverage, 0.0, 1.0);
            if (settings.operation.detail & 1u) != 0u {
                coverage = 1.0 - coverage;
            }
            result = pixel * coverage;
        }
    }

    output_pixels.values[invocation.y * extent.x + invocation.x] = checked(result);
}

// Dispatch uses output-local indices; shared evaluation and input reads use global coordinates.
@compute @workgroup_size(8, 8)
fn main(@builtin(global_invocation_id) invocation: vec3u) {
    let extent = max(
            vec2u(
                settings.sources[2].z - settings.sources[2].x,
                settings.sources[2].w - settings.sources[2].y,
            ),
            vec2u(1u),
        );

    // Ignore padding from the host's rounded-up 8x8 workgroups.
    if any(invocation.xy >= extent) {
        return;
    }

    let coordinate = invocation.xy + vec2u(settings.sources[2].x, settings.sources[2].y);
    let index = coordinate.y * settings.frame.width + coordinate.x;
    let operation = settings.operation.code;
    let inside = all(coordinate >= vec2u(settings.bounds.x, settings.bounds.y))
        && all(coordinate < vec2u(settings.bounds.z, settings.bounds.w));

    var result = vec4f(0.0);

    if inside {
        result = checked(evaluate(coordinate, index, operation));
    } else if operation >= OP_FILL && operation <= OP_MASK {
        // Fill, composite and mask operations retain the backdrop outside their operation rectangle.
        result = first_pixel(coordinate);
    }

    // Filter and particle results discard hidden RGB when alpha is exactly zero.
    if operation >= OP_BLUR && result.a == 0.0 {
        result = vec4f(0.0);
    }

    // The output row width is the cropped extent, not the full frame width.
    output_pixels.values[invocation.y * extent.x + invocation.x] = result;
}
