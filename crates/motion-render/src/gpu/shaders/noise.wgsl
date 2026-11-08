// N0 twins: wrapping u32 hashes, quintic interpolation, twelve unit gradients.
fn noise_hash32(source: u32) -> u32 {
    var value = source;
    value ^= value >> 16u;
    value *= 0x7feb352du;
    value ^= value >> 15u;
    value *= 0x846ca68bu;
    return value ^ (value >> 16u);
}

fn noise_hash2(cell: vec2i, seed: u32) -> u32 {
    let words = bitcast<vec2u>(cell);
    return noise_hash32(words.x * 0x9e3779b9u ^ words.y * 0x85ebca6bu ^ seed);
}

fn noise_hash3(cell: vec3i, seed: u32) -> u32 {
    let words = bitcast<vec3u>(cell);
    return noise_hash32(
            words.x * 0x9e3779b9u ^ words.y * 0x85ebca6bu ^ words.z * 0xc2b2ae35u ^ seed,
        );
}

fn noise_fade(value: f32) -> vec2f {
    return vec2f(
            value * value * value * (value * (value * 6.0 - 15.0) + 10.0),
            30.0 * value * value * (value - 1.0) * (value - 1.0),
        );
}

fn noise_supported(point: vec2f) -> bool {
    return all(point >= vec2f(-2147483648.0)) && all(point < vec2f(2147483648.0));
}

fn noise_lattice_value(cell: vec2i, seed: u32) -> f32 {
    return f32(noise_hash2(cell, seed)) / 4294967295.0 * 2.0 - 1.0;
}

// Return value, derivative x, derivative y.
fn noise_value2_d(point: vec2f, seed: u32) -> vec3f {
    if !noise_supported(point) {
        return vec3f(0.0);
    }
    let cell = vec2i(floor(point));
    let fraction = point - floor(point);
    let horizontal = noise_fade(fraction.x);
    let vertical = noise_fade(fraction.y);
    let samples = vec4f(
            noise_lattice_value(cell, seed),
            noise_lattice_value(cell + vec2i(1, 0), seed),
            noise_lattice_value(cell + vec2i(0, 1), seed),
            noise_lattice_value(cell + vec2i(1), seed),
        );
    let top = samples.x * (1.0 - horizontal.x) + samples.y * horizontal.x;
    let bottom = samples.z * (1.0 - horizontal.x) + samples.w * horizontal.x;
    return vec3f(
            top * (1.0 - vertical.x) + bottom * vertical.x,
            horizontal.y
            * ((samples.y - samples.x) * (1.0 - vertical.x) + (samples.w - samples.z) * vertical.x),
            vertical.y * (bottom - top),
        );
}

fn noise_value2(point: vec2f, seed: u32) -> f32 {
    return noise_value2_d(point, seed).x;
}

fn noise_gradient(cell: vec2i, seed: u32) -> vec2f {
    let gradients = array<vec2f, 12>(
            vec2f(1.0, 0.0),
            vec2f(0.8660254037844386, 0.5),
            vec2f(0.5, 0.8660254037844386),
            vec2f(0.0, 1.0),
            vec2f(-0.5, 0.8660254037844386),
            vec2f(-0.8660254037844386, 0.5),
            vec2f(-1.0, 0.0),
            vec2f(-0.8660254037844386, -0.5),
            vec2f(-0.5, -0.8660254037844386),
            vec2f(0.0, -1.0),
            vec2f(0.5, -0.8660254037844386),
            vec2f(0.8660254037844386, -0.5),
        );
    return gradients[noise_hash2(cell, seed) % 12u];
}

fn noise_perlin_corner(cell: vec2i, offset: vec2i, fraction: vec2f, seed: u32) -> vec3f {
    let gradient = noise_gradient(cell + offset, seed);
    return vec3f(dot(gradient, fraction - vec2f(offset)), gradient);
}

fn noise_perlin_row(first: vec3f, second: vec3f, horizontal: vec2f) -> vec3f {
    return first * (1.0 - horizontal.x) + second * horizontal.x
        + vec3f(0.0, horizontal.y * (second.x - first.x), 0.0);
}

fn noise_perlin2_d(point: vec2f, seed: u32) -> vec3f {
    if !noise_supported(point) {
        return vec3f(0.0);
    }
    let cell = vec2i(floor(point));
    let fraction = point - floor(point);
    let horizontal = noise_fade(fraction.x);
    let vertical = noise_fade(fraction.y);
    let top = noise_perlin_row(
            noise_perlin_corner(cell, vec2i(0), fraction, seed),
            noise_perlin_corner(cell, vec2i(1, 0), fraction, seed),
            horizontal,
        );
    let bottom = noise_perlin_row(
            noise_perlin_corner(cell, vec2i(0, 1), fraction, seed),
            noise_perlin_corner(cell, vec2i(1), fraction, seed),
            horizontal,
        );
    return top * (1.0 - vertical.x) + bottom * vertical.x
        + vec3f(0.0, 0.0, vertical.y * (bottom.x - top.x));
}

fn noise_perlin2(point: vec2f, seed: u32) -> f32 {
    return noise_perlin2_d(point, seed).x;
}

fn noise_curl2(point: vec2f, seed: u32) -> vec2f {
    let derivative = noise_perlin2_d(point, seed);
    return vec2f(derivative.z, -derivative.y);
}

// Algorithms: 0 value, 1 Perlin, 2 turbulence, 3 ridged, 4 simplex.
fn noise_fbm(
    source: vec2f,
    seed: u32,
    octaves: u32,
    lacunarity: f32,
    gain: f32,
    algorithm: u32,
) -> f32 {
    if octaves < 1u || octaves > 8u || !(lacunarity >= 1.0 && lacunarity <= 8.0)
        || !(gain >= 0.0 && gain <= 1.0) || algorithm > 4u {
        return 0.0;
    }
    var point = source;
    var amplitude = 1.0;
    var total = 0.0;
    var weight = 0.0;
    for(var octave = 0u; octave < octaves; octave++) {
        if !noise_supported(point) {
            return 0.0;
        }
        var value = noise_perlin2(point, seed);
        if algorithm == 0u {
            value = noise_value2(point, seed);
        }
        if algorithm == 2u {
            value = abs(value);
        }
        if algorithm == 3u {
            value = max(1.0 - abs(value), 0.0) * max(1.0 - abs(value), 0.0);
        }
        if algorithm == 4u {
            value = noise_simplex2(point, seed);
        }
        total += value * amplitude;
        weight += amplitude;
        amplitude *= gain;
        point *= lacunarity;
    }
    return total / weight;
}

fn noise_worley2(point: vec2f, seed: u32) -> vec2f {
    if !noise_supported(point) {
        return vec2f(0.0);
    }
    let cell = vec2i(floor(point));
    let fraction = point - floor(point);
    var nearest = vec2f(1e30);
    for(var vertical = -2; vertical <= 2; vertical++) {
        for(var horizontal = -2; horizontal <= 2; horizontal++) {
            let offset = vec2i(horizontal, vertical);
            let hash = noise_hash2(cell + offset, seed);
            let feature =
                vec2f(f32(noise_hash32(hash ^ 0xa511e9b3u)), f32(noise_hash32(hash ^ 0x63d83595u)))
                / 4294967296.0;
            let delta = vec2f(offset) + feature - fraction;
            let distance = dot(delta, delta);
            if distance < nearest.x {
                nearest = vec2f(distance, nearest.x);
            } else if distance < nearest.y {
                nearest.y = distance;
            }
        }
    }
    return sqrt(nearest);
}

fn noise_simplex_corner(cell: vec2i, offset: vec2i, local: vec2f, shift: f32, seed: u32) -> f32 {
    let delta = local - vec2f(offset) + vec2f(shift);
    let weight = max(0.5 - dot(delta, delta), 0.0);
    let squared = weight * weight;
    return squared * squared * dot(noise_gradient(cell + offset, seed), delta);
}

fn noise_simplex2(point: vec2f, seed: u32) -> f32 {
    if !noise_supported(point) {
        return 0.0;
    }
    let skew = (point.x + point.y) * 0.3660254037844386;
    let cell = floor(point + vec2f(skew));
    if !noise_supported(cell) {
        return 0.0;
    }
    let unskew = (cell.x + cell.y) * 0.21132486540518713;
    let local = point - (cell - vec2f(unskew));
    let middle = select(vec2i(0, 1), vec2i(1, 0), local.x > local.y);
    return 70.0
        * (noise_simplex_corner(vec2i(cell), vec2i(0), local, 0.0, seed)
            + noise_simplex_corner(vec2i(cell), middle, local, 0.21132486540518713, seed)
            + noise_simplex_corner(vec2i(cell), vec2i(1), local, 0.42264973081037427, seed));
}

fn noise_gradient3(cell: vec3i, seed: u32) -> vec3f {
    let gradients = array<vec3f, 12>(
            vec3f(1.0, 1.0, 0.0),
            vec3f(-1.0, 1.0, 0.0),
            vec3f(1.0, -1.0, 0.0),
            vec3f(-1.0, -1.0, 0.0),
            vec3f(1.0, 0.0, 1.0),
            vec3f(-1.0, 0.0, 1.0),
            vec3f(1.0, 0.0, -1.0),
            vec3f(-1.0, 0.0, -1.0),
            vec3f(0.0, 1.0, 1.0),
            vec3f(0.0, -1.0, 1.0),
            vec3f(0.0, 1.0, -1.0),
            vec3f(0.0, -1.0, -1.0),
        );
    return gradients[noise_hash3(cell, seed) % 12u] * 0.7071067811865476;
}

fn noise_perlin3(point: vec3f, seed: u32) -> f32 {
    if !all(point >= vec3f(-2147483648.0)) || !all(point < vec3f(2147483648.0)) {
        return 0.0;
    }
    let cell = vec3i(floor(point));
    let fraction = point - floor(point);
    let amount = vec3f(
            noise_fade(fraction.x).x,
            noise_fade(fraction.y).x,
            noise_fade(fraction.z).x,
        );
    var result = 0.0;
    for(var depth = 0; depth <= 1; depth++) {
        for(var vertical = 0; vertical <= 1; vertical++) {
            for(var horizontal = 0; horizontal <= 1; horizontal++) {
                let offset = vec3i(horizontal, vertical, depth);
                let weights = select(vec3f(1.0) - amount, amount, offset == vec3i(1));
                result += dot(noise_gradient3(cell + offset, seed), fraction - vec3f(offset))
                    * weights.x * weights.y * weights.z;
            }
        }
    }
    return result;
}

// Valid configurations mirror DomainWarp; invalid input returns the source point.
fn noise_domain_warp(
    point: vec2f,
    seed: u32,
    strength: f32,
    frequency: f32,
    octaves: u32,
    lacunarity: f32,
    gain: f32,
) -> vec2f {
    if !noise_supported(point) || !(strength >= 0.0 && strength <= 1000000.0)
        || !(frequency > 0.0 && frequency <= 1000000.0) || octaves < 1u || octaves > 8u
        || !(lacunarity >= 1.0 && lacunarity <= 8.0) || !(gain >= 0.0 && gain <= 1.0) {
        return point;
    }
    if strength == 0.0 {
        return point;
    }
    let source = point * frequency;
    var last = source;
    for(var octave = 1u; octave < octaves; octave++) {
        last *= lacunarity;
    }
    if !noise_supported(source) || !noise_supported(last) {
        return point;
    }
    let displacement = vec2f(
            noise_fbm(source, seed, octaves, lacunarity, gain, 1u),
            noise_fbm(source, seed ^ 0xa511e9b3u, octaves, lacunarity, gain, 1u),
        );
    let result = point + strength * displacement;
    if !noise_supported(result) {
        return point;
    }
    return result;
}

// FilmGrain uniforms reuse the N0 lattice hash; unsigned display cells reinterpret as signed words.
fn noise_film_hash(cell: vec2u, seed: u32) -> f32 {
    return f32(noise_hash2(bitcast<vec2i>(cell), seed) >> 8u) / 16777216.0;
}

fn noise_film_gaussian(cell: vec2u, seed: u32) -> f32 {
    return (noise_film_hash(cell, seed) + noise_film_hash(cell, seed ^ 0xa511e9b3u) - 1.0)
        * 2.4494898;
}

fn noise_film_grain(position: vec2f, seed: u32) -> f32 {
    let cell = vec2u(floor(position));
    let fraction = fract(position);
    let top = noise_film_gaussian(cell, seed) * (1.0 - fraction.x)
        + noise_film_gaussian(cell + vec2u(1u, 0u), seed) * fraction.x;
    let bottom = noise_film_gaussian(cell + vec2u(0u, 1u), seed) * (1.0 - fraction.x)
        + noise_film_gaussian(cell + vec2u(1u), seed) * fraction.x;
    let variance = (vec2f(1.0) - fraction) * (vec2f(1.0) - fraction) + fraction * fraction;
    return (top * (1.0 - fraction.y) + bottom * fraction.y) / sqrt(variance.x * variance.y);
}
