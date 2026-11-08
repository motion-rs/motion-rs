// Cube metadata (10 words), shaper size/domain (7), cube RGB, then shaper RGB.
fn cube_load(index: u32) -> f32 {
    return operation_data.values[index];
}

fn cube_at(cell: vec3u, base: u32, edge: u32) -> vec3f {
    let index = base + 17u + (cell.x + edge * (cell.y + edge * cell.z)) * 3u;
    return vec3f(cube_load(index), cube_load(index + 1u), cube_load(index + 2u));
}

fn cube_apply(source: vec3f, base: u32) -> vec3f {
    let edge = u32(cube_load(base));
    if edge == 0u {
        return source;
    }
    var rgb = source;
    let encoding = u32(cube_load(base + 7u));
    let minimum = cube_load(base + 8u);
    let maximum = cube_load(base + 9u);
    if encoding == 1u {
        rgb = max(rgb, vec3f(0.0));
        rgb = select(
                1.055 * pow(rgb, vec3f(1.0 / 2.4)) - 0.055,
                rgb * 12.92,
                rgb <= vec3f(0.0031308),
            );
    } else if encoding == 2u {
        rgb = (log2(max(rgb, vec3f(exp2(minimum)))) - minimum) / (maximum - minimum);
    }
    let shaper_size = u32(cube_load(base + 10u));
    if shaper_size != 0u {
        let shaper_min = vec3f(cube_load(base + 11u), cube_load(base + 12u), cube_load(base + 13u));
        let shaper_max = vec3f(cube_load(base + 14u), cube_load(base + 15u), cube_load(base + 16u));
        let position = clamp((rgb - shaper_min) / (shaper_max - shaper_min), vec3f(0.0), vec3f(1.0))
            * f32(shaper_size - 1u);
        let cell = min(vec3u(position), vec3u(shaper_size - 2u));
        let fraction = position - vec3f(cell);
        let start = base + 17u + edge * edge * edge * 3u;
        for(var channel = 0u; channel < 3u; channel++) {
            let first = cube_load(start + cell[channel] * 3u + channel);
            let second = cube_load(start + (cell[channel] + 1u) * 3u + channel);
            rgb[channel] = first * (1.0 - fraction[channel]) + second * fraction[channel];
        }
    }
    let domain_min = vec3f(cube_load(base + 1u), cube_load(base + 2u), cube_load(base + 3u));
    let domain_max = vec3f(cube_load(base + 4u), cube_load(base + 5u), cube_load(base + 6u));
    let position = clamp((rgb - domain_min) / (domain_max - domain_min), vec3f(0.0), vec3f(1.0))
        * f32(edge - 1u);
    var cell = min(vec3u(position), vec3u(edge - 2u));
    let fraction = position - vec3f(cell);
    var order = vec3u(0u, 1u, 2u);
    if fraction[order.x] < fraction[order.y] {
        order = order.yxz;
    }
    if fraction[order.y] < fraction[order.z] {
        order = order.xzy;
    }
    if fraction[order.x] < fraction[order.y] {
        order = order.yxz;
    }
    let start = cube_at(cell, base, edge);
    cell[order.x] += 1u;
    let first = cube_at(cell, base, edge);
    cell[order.y] += 1u;
    let second = cube_at(cell, base, edge);
    cell[order.z] += 1u;
    let end = cube_at(cell, base, edge);
    rgb = start + fraction[order.x] * (first - start) + fraction[order.y] * (second - first)
        + fraction[order.z] * (end - second);
    if encoding == 1u {
        rgb = max(rgb, vec3f(0.0));
        rgb = select(pow((rgb + 0.055) / 1.055, vec3f(2.4)), rgb / 12.92, rgb <= vec3f(0.04045));
    } else if encoding == 2u {
        rgb = exp2(rgb * (maximum - minimum) + minimum);
    }
    return rgb;
}
