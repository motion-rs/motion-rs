// GPU edge binning and f32 area coverage. Host layouts are asserted in gpu/layout.rs.
struct CoverageParams {
    fills: u32,
    edges: u32,
    tiles: u32,
    rows: u32,
}

struct CoverageFill {
    size: vec2u,
    coverage: u32,
    rule: u32,
    first_edge: u32,
    edge_count: u32,
    first_tile: u32,
    columns: u32,
    first_index: u32,
    index_capacity: u32,
    first_row: u32,
    tile_rows: u32,
}

struct CoverageEdge {
    points: vec4f,
    fill: u32,
    reserved: array<u32, 3>,
}

struct CoverageTile {
    count: atomic<u32>,
    offset: u32,
    cursor: atomic<u32>,
    reserved: u32,
}

@group(0) @binding(0) var<uniform> params: CoverageParams;
@group(0) @binding(1) var<storage, read> fills: array<CoverageFill>;
@group(0) @binding(2) var<storage, read> edges: array<CoverageEdge>;
@group(0) @binding(3) var<storage, read_write> tiles: array<CoverageTile>;
@group(0) @binding(4) var<storage, read_write> indices: array<u32>;
@group(0) @binding(5) var<storage, read_write> carries: array<f32>;
@group(0) @binding(6) var<storage, read_write> coverage: array<u32>;

const TILE_SIZE = 16u;

fn upper_lower(points: vec4f) -> vec4f {
    return select(points.zwxy, points, points.w > points.y);
}

// Edge x bounds after clipping to one tile band. Offscreen-left edges stay in
// column zero: their signed row height must propagate into the interior carry.
fn tile_columns(points: vec4f, row: u32, columns: u32) -> vec2u {
    let edge = upper_lower(points);
    let top = max(edge.y, f32(row * TILE_SIZE));
    let bottom = min(edge.w, f32((row + 1u) * TILE_SIZE));
    let xa = edge.x + (edge.z - edge.x) * ((top - edge.y) / (edge.w - edge.y));
    let xb = edge.x + (edge.z - edge.x) * ((bottom - edge.y) / (edge.w - edge.y));
    return vec2u(clamp(
        floor(vec2f(min(xa, xb), max(xa, xb)) / f32(TILE_SIZE)),
        vec2f(0.0), vec2f(f32(columns - 1u))
    ));
}

fn bin_edge(index: u32, scatter: bool) {
    let record = edges[index];
    let edge = upper_lower(record.points);
    if edge.w <= edge.y {
        return;
    }
    let fill = fills[record.fill];
    let first = u32(floor(max(edge.y, 0.0) / f32(TILE_SIZE)));
    let last = min(fill.tile_rows, u32(ceil(max(0.0, min(edge.w, f32(fill.size.y))) / f32(TILE_SIZE))));
    for (var row = first; row < last; row++) {
        let columns = tile_columns(record.points, row, fill.columns);
        for (var column = columns.x; column <= columns.y; column++) {
            let tile = fill.first_tile + row * fill.columns + column;
            if scatter {
                let slot = atomicAdd(&tiles[tile].cursor, 1u);
                indices[tiles[tile].offset + slot] = index;
            } else {
                atomicAdd(&tiles[tile].count, 1u);
            }
        }
    }
}

@compute @workgroup_size(64)
fn count_tiles(@builtin(global_invocation_id) id: vec3u, @builtin(num_workgroups) groups: vec3u) {
    let index = id.x + id.y * groups.x * 64u;
    if index < params.edges {
        bin_edge(index, false);
    }
}

// Fills own disjoint statically admitted index ranges; no readback or resizing.
@compute @workgroup_size(64)
fn prefix_tiles(@builtin(global_invocation_id) id: vec3u, @builtin(num_workgroups) groups: vec3u) {
    let index = id.x + id.y * groups.x * 64u;
    if index >= params.fills {
        return;
    }
    let fill = fills[index];
    var offset = fill.first_index;
    let end = fill.first_tile + fill.columns * fill.tile_rows;
    for (var tile = fill.first_tile; tile < end; tile++) {
        tiles[tile].offset = offset;
        offset += atomicLoad(&tiles[tile].count);
    }
}

@compute @workgroup_size(64)
fn fill_tiles(@builtin(global_invocation_id) id: vec3u, @builtin(num_workgroups) groups: vec3u) {
    let index = id.x + id.y * groups.x * 64u;
    if index < params.edges {
        bin_edge(index, true);
    }
}

fn row_height(points: vec4f, row: u32) -> f32 {
    let edge = upper_lower(points);
    let height = max(0.0, min(edge.w, f32(row + 1u)) - max(edge.y, f32(row)));
    return select(-height, height, points.w > points.y);
}

// Prefix signed row heights only after an edge's final tile. In its other tiles
// the edge is still evaluated locally, so long sloping edges are never doubled.
@compute @workgroup_size(64)
fn carry_rows(@builtin(global_invocation_id) id: vec3u, @builtin(num_workgroups) groups: vec3u) {
    let index = id.x + id.y * groups.x * 64u;
    if index >= params.rows {
        return;
    }
    var low = 0u;
    var high = params.fills - 1u;
    while low < high {
        let middle = low + (high - low + 1u) / 2u;
        if fills[middle].first_row <= index {
            low = middle;
        } else {
            high = middle - 1u;
        }
    }
    let fill = fills[low];
    let row = index - fill.first_row;
    let band = row / TILE_SIZE;
    var total = 0.0;
    for (var column = 0u; column < fill.columns; column++) {
        let tile = fill.first_tile + band * fill.columns + column;
        carries[tile * TILE_SIZE + row % TILE_SIZE] = total;
        let first = tiles[tile].offset;
        let last = first + atomicLoad(&tiles[tile].count);
        for (var item = first; item < last; item++) {
            let edge = edges[indices[item]].points;
            if tile_columns(edge, band, fill.columns).y == column {
                total += row_height(edge, row);
            }
        }
    }
}

// Integral of clamp(right - x, 0, 1) along a linear edge, times its height.
// This is the CPU trapezoid-area formula, evaluated without u8 quantization.
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

fn signed_area(tile: u32, pixel: vec2u) -> f32 {
    let origin = vec2f((pixel / TILE_SIZE) * TILE_SIZE);
    let top = f32(pixel.y % TILE_SIZE);
    let first = tiles[tile].offset;
    let last = first + atomicLoad(&tiles[tile].count);
    var total = carries[tile * TILE_SIZE + pixel.y % TILE_SIZE];
    for (var item = first; item < last; item++) {
        let points = edges[indices[item]].points;
        let edge = upper_lower(points) - origin.xyxy;
        let y_top = max(edge.y, top);
        let y_bottom = min(edge.w, top + 1.0);
        if edge.w <= edge.y || y_bottom <= y_top {
            continue;
        }
        let xa = edge.x + (edge.z - edge.x) * ((y_top - edge.y) / (edge.w - edge.y));
        let xb = edge.x + (edge.z - edge.x) * ((y_bottom - edge.y) / (edge.w - edge.y));
        let area = column_area(xa, xb, f32(pixel.x % TILE_SIZE), y_bottom - y_top);
        total += select(-area, area, points.w > points.y);
    }
    return total;
}

fn rule_coverage(total: f32, rule: u32) -> f32 {
    let magnitude = abs(total);
    if rule == 0u {
        return min(magnitude, 1.0);
    }
    let folded = magnitude - 2.0 * floor(magnitude * 0.5);
    return select(folded, 2.0 - folded, folded > 1.0);
}

@compute @workgroup_size(16, 16)
fn coverage_pass(
    @builtin(workgroup_id) group: vec3u,
    @builtin(local_invocation_id) local: vec3u,
    @builtin(num_workgroups) groups: vec3u,
) {
    let tile = group.x + group.y * groups.x;
    if tile >= params.tiles {
        return;
    }
    var low = 0u;
    var high = params.fills - 1u;
    while low < high {
        let middle = low + (high - low + 1u) / 2u;
        if fills[middle].first_tile <= tile {
            low = middle;
        } else {
            high = middle - 1u;
        }
    }
    let fill = fills[low];
    let index = tile - fill.first_tile;
    let pixel = vec2u(index % fill.columns, index / fill.columns) * TILE_SIZE + local.xy;
    if any(pixel >= fill.size) {
        return;
    }
    let value = clamp(rule_coverage(signed_area(tile, pixel), fill.rule), 0.0, 1.0);
    coverage[fill.coverage + pixel.y * fill.size.x + pixel.x] = bitcast<u32>(value);
}
