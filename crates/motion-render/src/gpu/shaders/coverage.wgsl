// Compute f32 fill coverage from flattened edges into the shared frame coverage buffer.
struct Params {
    // x: fill count, y: f32 samples to write.
    counts: vec2u,
}

@group(0) @binding(0)
var<uniform> params: Params;

// Eight words per fill: x, y, width, height, coverage word, row table start, rule, first sample.
@group(0) @binding(1)
var<storage, read> fills: array<u32>;

// Directed edges relative to the fill origin: xy is the start and zw the end.
@group(0) @binding(2)
var<storage, read> edges: array<vec4f>;

// Per fill: height + 1 absolute offsets, then the edge indices crossing each row.
@group(0) @binding(3)
var<storage, read> rows: array<u32>;

@group(0) @binding(4)
var<storage, read_write> coverage: array<u32>;

const FILL_WORDS = 8u;

// Integral of clamp(right - x, 0, 1) over a line whose x runs linearly from xa to xb, times its height.
fn column_area(xa: f32, xb: f32, left: f32, height: f32) -> f32 {
    let span = xb - xa;
    var first = 0.0;
    var second = 0.0;
    if abs(span) > 1e-12 {
        first = clamp((left - xa) / span, 0.0, 1.0);
        second = clamp((left + 1.0 - xa) / span, 0.0, 1.0);
    }
    let cuts = vec4f(0.0, min(first, second), max(first, second), 1.0);
    var area = 0.0;
    for (var piece = 0u; piece < 3u; piece++) {
        let start = cuts[piece];
        let end = cuts[piece + 1u];
        let a = clamp(left + 1.0 - (xa + span * start), 0.0, 1.0);
        let b = clamp(left + 1.0 - (xa + span * end), 0.0, 1.0);
        area += (a + b) * 0.5 * (end - start);
    }
    return area * height;
}

// Signed area of the pixel covered to the right of every edge crossing its row.
fn signed_area(rows_start: u32, pixel: vec2u) -> f32 {
    let left = f32(pixel.x);
    let top = f32(pixel.y);
    let first = rows[rows_start + pixel.y];
    let last = rows[rows_start + pixel.y + 1u];
    var total = 0.0;
    for (var item = first; item < last; item++) {
        let edge = edges[rows[item]];
        let downward = edge.w > edge.y;
        let upper = select(edge.zw, edge.xy, downward);
        let lower = select(edge.xy, edge.zw, downward);
        let rise = lower.y - upper.y;
        let y_top = max(upper.y, top);
        let y_bottom = min(lower.y, top + 1.0);
        if rise <= 0.0 || y_bottom <= y_top {
            continue;
        }
        let slope = (lower.x - upper.x) / rise;
        let xa = upper.x + slope * (y_top - upper.y);
        let xb = upper.x + slope * (y_bottom - upper.y);
        let area = column_area(xa, xb, left, y_bottom - y_top);
        total += select(-area, area, downward);
    }
    return total;
}

// Map signed winding area to coverage under the fill rule.
fn rule_coverage(total: f32, rule: u32) -> f32 {
    let magnitude = abs(total);
    if rule == 0u {
        return min(magnitude, 1.0);
    }
    let folded = magnitude - 2.0 * floor(magnitude * 0.5);
    return select(folded, 2.0 - folded, folded > 1.0);
}

@compute @workgroup_size(256)
fn coverage_pass(@builtin(global_invocation_id) id: vec3u, @builtin(num_workgroups) groups: vec3u) {
    let sample = id.x + id.y * groups.x * 256u;
    if sample >= params.counts.y {
        return;
    }

    // Each invocation owns one sample; large frames span multiple workgroup rows.
    var low = 0u;
    var high = params.counts.x - 1u;
    while low < high {
        let middle = low + (high - low + 1u) / 2u;
        if fills[middle * FILL_WORDS + 7u] <= sample {
            low = middle;
        } else {
            high = middle - 1u;
        }
    }

    let base = low * FILL_WORDS;
    let size = vec2u(fills[base + 2u], fills[base + 3u]);
    let index = sample - fills[base + 7u];
    let area = signed_area(fills[base + 5u], vec2u(index % size.x, index / size.x));
    let value = clamp(rule_coverage(area, fills[base + 6u]), 0.0, 1.0);
    coverage[fills[base + 4u] + index] = bitcast<u32>(value);
}
