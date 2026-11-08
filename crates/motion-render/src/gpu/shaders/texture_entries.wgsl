// Entry points appended to the texture header and shared composition arithmetic.

// The third source bounds the global output; invocation.xy is texture-local.
@compute @workgroup_size(8, 8)
fn texture_main(@builtin(global_invocation_id) invocation: vec3u) {
    let extent = max(
        vec2u(settings.sources[2].z - settings.sources[2].x, settings.sources[2].w - settings.sources[2].y),
        vec2u(1u)
    );

    // The host rounds dispatch dimensions up to 8x8 workgroups.
    if any(invocation.xy >= extent) {
        return;
    }

    // Evaluation and input reads remain global even when the output is cropped.
    let coordinate = invocation.xy + vec2u(settings.sources[2].x, settings.sources[2].y);
    let operation = settings.operation.code;
    let inside = all(coordinate >= vec2u(settings.bounds.x, settings.bounds.y))
        && all(coordinate < vec2u(settings.bounds.z, settings.bounds.w));

    var result = vec4f(0.0);
    if inside {
        result = checked(evaluate(coordinate, coordinate.y * settings.frame.width + coordinate.x, operation));
    } else if operation >= OP_FILL && operation <= OP_MASK {
        result = first_pixel(coordinate);
    }

    // Filter and particle operations discard hidden RGB when alpha is exactly zero.
    if operation >= OP_BLUR && result.a == 0.0 {
        result = vec4f(0.0);
    }

    // Each in-bounds invocation owns one output texel.
    textureStore(output_texture, vec2i(invocation.xy), result);
}

// In-place batches keep the backdrop's complete finite check without copying its pixels.
@compute @workgroup_size(8, 8)
fn texture_check(@builtin(global_invocation_id) invocation: vec3u) {
    let origin = vec2u(settings.sources[0].x, settings.sources[0].y);
    let extent = vec2u(settings.sources[0].z, settings.sources[0].w) - origin;
    if all(invocation.xy < extent) {
        _ = first_pixel(invocation.xy + origin);
    }
}

// Each instance supplies a global-space rectangle expanded into two triangles.
@vertex
fn texture_vertex(
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

    let point = rect.xy + (rect.zw - rect.xy) * corners[vertex]
        - vec2f(vec2u(settings.sources[2].x, settings.sources[2].y));
    let extent = vec2f(max(
        vec2u(settings.sources[2].z - settings.sources[2].x, settings.sources[2].w - settings.sources[2].y),
        vec2u(1u)
    ));

    // Map output-local pixels to clip space, flipping the top-down image Y axis.
    return StripVertex(
        vec4f(
            point.x / extent.x * 2.0 - 1.0,
            1.0 - point.y / extent.y * 2.0,
            0.0,
            1.0
        ),
        record
    );
}

// Seed the render target from the first input before ordered source draws.
@fragment
fn texture_backdrop(@builtin(position) position: vec4f) -> @location(0) vec4f {
    return first_pixel(vec2u(position.xy) + vec2u(settings.sources[2].x, settings.sources[2].y));
}

// Return a premultiplied source; the pipeline applies SourceOver or additive Plus blending.
@fragment
fn texture_source(
    @builtin(position) position: vec4f,
    @interpolate(flat) @location(0) record: u32,
) -> @location(0) vec4f {
    // Fragment positions are output-local; shared source helpers expect global coordinates.
    let coordinate = vec2u(position.xy) + vec2u(settings.sources[2].x, settings.sources[2].y);
    if settings.operation.code == OP_FILL {
        return fill_source(coordinate, record);
    }

    return checked(second_pixel(coordinate) * (value(0u) * coverage_at(coordinate)));
}

// Resolve only the final output boundary to a tightly packed, row-major vec4f buffer.
@compute @workgroup_size(8, 8)
fn texture_resolve(@builtin(global_invocation_id) invocation: vec3u) {
    let extent = max(
        vec2u(settings.sources[2].z - settings.sources[2].x, settings.sources[2].w - settings.sources[2].y),
        vec2u(1u)
    );

    if any(invocation.xy >= extent) {
        return;
    }

    let coordinate = invocation.xy + vec2u(settings.sources[2].x, settings.sources[2].y);

    // Local indexing gives each invocation a distinct buffer element, independent of crop origin.
    output_pixels.values[invocation.y * extent.x + invocation.x] = first_pixel(coordinate);
}
