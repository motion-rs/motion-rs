// Version-one core analytic protocol: 20 header words followed by eight-word seeded initial facts.
// The output is the ordinary four-word instance header followed by twelve-word primitive records.
//
// `evaluate` mirrors `TrajectoryDefinition::evaluate` in motion-content (particles/trajectory.rs)
// in f32: kind 1 is ballistic with drag, 2 is converge (smoothstep), 3 is orbit and 4 is noise
// drift. `value_noise` mirrors `motion_core::animation::value_noise`. Keep both implementations in sync.
@group(0) @binding(0) var<storage, read> program: array<f32>;
@group(0) @binding(1) var<storage, read_write> primitives: array<f32>;
@group(0) @binding(2) var<storage, read_write> status: atomic<u32>;

fn hash32(input: u32) -> u32 {
    var value = input ^ (input >> 16u);
    value *= 0x7feb352du;
    value ^= value >> 15u;
    value *= 0x846ca68bu;
    return value ^ (value >> 16u);
}

fn value_noise(position: f32, seed: u32) -> vec2f {
    let cell = i32(floor(position));
    let time = position - floor(position);
    let amount = time * time * time * (time * (time * 6.0 - 15.0) + 10.0);
    let derivative = 30.0 * time * time * (time - 1.0) * (time - 1.0);
    let a = f32(hash32(bitcast<u32>(cell) * 0x9e3779b9u ^ seed)) / 4294967295.0 * 2.0 - 1.0;
    let b = f32(hash32(bitcast<u32>(cell + 1) * 0x9e3779b9u ^ seed)) / 4294967295.0 * 2.0 - 1.0;
    return vec2f(a * (1.0 - amount) + b * amount, derivative * (b - a));
}

@compute @workgroup_size(64)
fn evaluate(@builtin(global_invocation_id) id: vec3u, @builtin(num_workgroups) groups: vec3u) {
    let index = id.x + id.y * groups.x * 64u;
    if index >= u32(program[5]) {
        return;
    }
    let source = 20u + index * 8u;
    let destination = 4u + index * 12u;
    let age = program[1];
    let lifetime = program[2];
    let kind = u32(program[0]);
    var position = vec2f(program[source], program[source + 1u]);
    var velocity = vec2f(program[source + 2u], program[source + 3u]);
    let custom = vec2f(program[source + 4u], program[source + 5u]);
    if kind == 1u {
        let drag = program[12];
        let product = drag * age;
        var advance: f32;
        var gravity: f32;
        if abs(product) < 0.01 {
            let square = product * product;
            let cube = square * product;
            let fourth = cube * product;
            advance = age * (1.0 - product * 0.5 + square / 6.0 - cube / 24.0 + fourth / 120.0);
            gravity = age * age * (0.5 - product / 6.0 + square / 24.0 - cube / 120.0 + fourth / 720.0);
        } else {
            advance = (1.0 - exp(-product)) / drag;
            gravity = (age - advance) / drag;
        }
        let acceleration = vec2f(program[10], program[11]);
        position += velocity * advance + acceleration * gravity;
        velocity = velocity * exp(-product) + acceleration * advance;
    } else if kind == 2u {
        let progress = clamp(age / lifetime, 0.0, 1.0);
        let delta = custom - position;
        position += delta * progress * progress * (3.0 - 2.0 * progress);
        velocity = delta * (6.0 * progress * (1.0 - progress) / lifetime);
    } else if kind == 3u {
        let angle = custom.x + program[11] * age;
        let radius = program[10];
        position = vec2f(program[8], program[9]) + radius * vec2f(cos(angle), sin(angle));
        velocity = radius * program[11] * vec2f(-sin(angle), cos(angle));
    } else {
        let amplitude = program[12];
        let frequency = program[13];
        let key = bitcast<u32>(program[source + 6u]);
        for (var axis = 0u; axis < 2u; axis++) {
            let value = value_noise(custom[axis] + age * frequency, key + axis);
            let origin = value_noise(custom[axis], key + axis);
            position[axis] += velocity[axis] * age + amplitude * (value.x - origin.x);
            velocity[axis] += amplitude * frequency * value.y;
        }
    }
    var rotation = program[source + 7u];
    if program[4] != 0.0 && any(velocity != vec2f(0.0)) {
        rotation += atan2(velocity.y, velocity.x);
    }
    // Fail the existing output transaction instead of publishing nonfinite GPU geometry.
    if any(abs(position) > vec2f(3.402823e38)) || abs(rotation) > 3.402823e38
        || any(position != position) || rotation != rotation {
        atomicOr(&status, 1u);
        return;
    }
    primitives[destination] = position.x;
    primitives[destination + 1u] = position.y;
    primitives[destination + 2u] = position.x;
    primitives[destination + 3u] = position.y;
    primitives[destination + 9u] = rotation;
    primitives[destination + 11u] = program[3];
}
