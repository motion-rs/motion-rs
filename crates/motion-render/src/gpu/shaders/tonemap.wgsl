// Shared by present and pack. Inputs and outputs are straight display-linear RGB.
struct OutputLut {
    values: array<f32>,
}

@group(0) @binding(5)
var<storage, read> output_lut: OutputLut;

fn display_saturation(rgb: vec3f, saturation: f32) -> vec3f {
    if saturation == 1.0 {
        return rgb;
    }
    let luma = dot(rgb, vec3f(0.2126, 0.7152, 0.0722));
    return vec3f(luma) + (rgb - vec3f(luma)) * saturation;
}

fn display_dither(coordinate: vec2u) -> f32 {
    let ranks = array<u32, 16>(
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
    return (f32(ranks[(coordinate.y % 4u) * 4u + coordinate.x % 4u]) + 0.5) / 16.0 - 0.5;
}

fn display_hable(value: vec3f) -> vec3f {
    return (value * (0.15 * value + 0.05) + 0.004) / (value * (0.15 * value + 0.5) + 0.06)
        - 0.004 / 0.06;
}

fn display_linear(source_rgb: vec3f, display: array<vec4u, 8>) -> vec3f {
    if display[4].x == 0u {
        return source_rgb;
    }
    let configuration = bitcast<vec4f>(display[0]);
    let slope = bitcast<vec4f>(display[1]);
    let offset = bitcast<vec4f>(display[2]);
    let power = bitcast<vec4f>(display[3]);
    var rgb = source_rgb * configuration.x;
    if any(slope.xyz != vec3f(1.0)) || any(offset.xyz != vec3f(0.0)) || any(power.xyz != vec3f(1.0))
        || slope.w != 1.0 {
        rgb = pow(max(rgb * slope.xyz + offset.xyz, vec3f(0.0)), power.xyz);
    }
    rgb = display_saturation(rgb, slope.w);
    rgb = cube_apply(rgb, 0u);
    rgb *= bitcast<f32>(display[7].w);
    let curve = u32(configuration.z);
    if curve == 1u {
        rgb = max(rgb, vec3f(0.0));
        let minimum = min(rgb.r, min(rgb.g, rgb.b));
        let subtraction = select(0.04, minimum - 6.25 * minimum * minimum, minimum < 0.08);
        rgb -= vec3f(subtraction);
        let peak = max(rgb.r, max(rgb.g, rgb.b));
        if peak >= 0.76 {
            let compressed = 1.0 - 0.24 * 0.24 / (peak + 0.24 - 0.76);
            let desaturation = 1.0 - 1.0 / (0.15 * (peak - compressed) + 1.0);
            rgb *= compressed / peak;
            rgb += (vec3f(compressed) - rgb) * desaturation;
        }
    } else if curve == 2u {
        rgb = vec3f(
                dot(rgb, vec3f(0.59719, 0.35458, 0.04823)),
                dot(rgb, vec3f(0.076, 0.90834, 0.01566)),
                dot(rgb, vec3f(0.0284, 0.13383, 0.83777)),
            );
        rgb = max(rgb, vec3f(0.0));
        rgb = max(
                (rgb * (rgb + 0.0245786) - 0.000090537)
                / (rgb * (0.983729 * rgb + 0.432951) + 0.238081),
                vec3f(0.0),
            );
        rgb = max(
                vec3f(
                    dot(rgb, vec3f(1.60475, -0.53108, -0.07367)),
                    dot(rgb, vec3f(-0.10208, 1.10813, -0.00605)),
                    dot(rgb, vec3f(-0.00327, -0.07276, 1.07602)),
                ),
                vec3f(0.0),
            );
    } else if curve == 3u {
        rgb = vec3f(
                dot(rgb, vec3f(0.842479, 0.0784336, 0.079224)),
                dot(rgb, vec3f(0.042328, 0.878468, 0.079166)),
                dot(rgb, vec3f(0.042375, 0.0784336, 0.879143)),
            );
        rgb = clamp((log2(max(rgb, vec3f(1e-10))) + 12.47393) / 16.5, vec3f(0.0), vec3f(1.0));
        let squared = rgb * rgb;
        rgb = clamp(
                15.5 * squared * squared * squared - 40.14 * squared * squared * rgb
                + 31.96 * squared * squared - 6.868 * squared * rgb + 0.4298 * squared
                + 0.1191 * rgb - 0.00232,
                vec3f(0.0),
                vec3f(1.0),
            );
        // AgX's polynomial is encoded gamma 2.2; linearize before the output OETF.
        rgb = pow(rgb, vec3f(2.2));
        rgb = max(
                vec3f(
                    dot(rgb, vec3f(1.196879, -0.0980209, -0.0990297)),
                    dot(rgb, vec3f(-0.0528969, 1.151903, -0.0989612)),
                    dot(rgb, vec3f(-0.0529716, -0.0980435, 1.151074)),
                ),
                vec3f(0.0),
            );
    } else if curve == 4u {
        rgb = display_hable(max(rgb, vec3f(0.0))) / display_hable(vec3f(configuration.w));
    }
    rgb /= bitcast<vec3f>(display[7].xyz);
    return cube_apply(display_saturation(rgb, configuration.y), display[6].w);
}

fn display_grain(rgb: vec3f, coordinate: vec2u, display: array<vec4u, 8>) -> vec3f {
    let parameters = bitcast<vec4f>(display[5]);
    if display[4].x == 0u || parameters.z == 0.0 {
        return rgb;
    }
    let response = bitcast<vec4f>(display[6]);
    let luma = clamp(dot(rgb, vec3f(0.2126, 0.7152, 0.0722)), 0.0, 1.0);
    let amplitude = parameters.z * pow(4.0 * luma * (1.0 - luma), response.x);
    let position = vec2f(coordinate) / parameters.w;
    let seed = display[4].z ^ display[4].w ^ display[5].x ^ display[5].y;
    let monochrome = noise_film_grain(position, seed);
    var result = rgb;
    for(var channel = 0u; channel < 3u; channel++) {
        let independent = noise_film_grain(position, seed ^ ((channel + 1u) * 0x9e3779b9u));
        result[channel] += amplitude * (monochrome + (independent - monochrome) * response.y);
    }
    return clamp(result, vec3f(0.0), vec3f(1.0));
}
