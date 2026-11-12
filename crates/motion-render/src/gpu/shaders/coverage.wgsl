// GPU curve flattening and winding-selected analytic boundary integration.
// Host layouts are asserted in gpu/layout.rs.
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

struct CoverageCurve {
    first: vec4f,
    second: vec4f,
    first_edge: u32,
    segments: u32,
    fill: u32,
    reserved: u32,
}

struct CoverageTile {
    count: atomic<u32>,
    offset: u32,
    cursor: atomic<u32>,
    reserved: u32,
}

@group(0) @binding(0) var<uniform> params: CoverageParams;
@group(0) @binding(1) var<storage, read> fills: array<CoverageFill>;
@group(0) @binding(2) var<storage, read_write> edges: array<CoverageEdge>;
@group(0) @binding(3) var<storage, read_write> tiles: array<CoverageTile>;
@group(0) @binding(4) var<storage, read_write> indices: array<u32>;
@group(0) @binding(5) var<storage, read_write> row_areas: array<f32>;
@group(0) @binding(6) var<storage, read_write> coverage: array<u32>;
@group(0) @binding(7) var<storage, read> curves: array<CoverageCurve>;

const TILE_SIZE = 16u;
// Integrated column areas per tile row, mirrored by the host planner.
const BOUNDARY_STRIDE = 16u;

var<workgroup> prefix_values: array<u32, 64>;
var<workgroup> prefix_fills: array<u32, 64>;
var<workgroup> prefix_base: u32;
var<workgroup> boundary_areas: array<f32, 1024>;

fn curve_point(curve: CoverageCurve, time: f32) -> vec2f {
    let a = mix(curve.first.xy, curve.first.zw, time);
    let b = mix(curve.first.zw, curve.second.xy, time);
    let c = mix(curve.second.xy, curve.second.zw, time);
    return mix(mix(a, b, time), mix(b, c, time), time);
}

// Wang counts and control hull tile capacities are admitted before upload.
// Each invocation exclusively owns its curve's statically sized output range.
@compute @workgroup_size(64)
fn flatten_curves(@builtin(global_invocation_id) id: vec3u, @builtin(num_workgroups) groups: vec3u) {
    let index = id.x + id.y * groups.x * 64u;
    if index >= arrayLength(&curves) {
        return;
    }
    let curve = curves[index];
    var previous = curve.first.xy;
    for (var segment = 0u; segment < curve.segments; segment++) {
        let point = select(
            curve_point(curve, f32(segment + 1u) / f32(curve.segments)),
            curve.second.zw, segment + 1u == curve.segments,
        );
        edges[curve.first_edge + segment] = CoverageEdge(vec4f(previous, point), curve.fill, array<u32, 3>(0u, 0u, 0u));
        previous = point;
    }
}

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
        // Winding selection needs every crossing to the left, including
        // offscreen contours, not just its integrated signed height.
        for (var column = columns.x; column < fill.columns; column++) {
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

fn tile_fill(index: u32) -> u32 {
    var low = 0u;
    var high = params.fills - 1u;
    while low < high {
        let middle = low + (high - low + 1u) / 2u;
        if fills[middle].first_tile <= index {
            low = middle;
        } else {
            high = middle - 1u;
        }
    }
    return low;
}

// Each global block may contain several fills. The last lane of each segment
// writes its sum to the reserved word of that segment's first tile.
@compute @workgroup_size(64)
fn prefix_tiles(
    @builtin(workgroup_id) group: vec3u,
    @builtin(local_invocation_index) lane: u32,
    @builtin(num_workgroups) groups: vec3u,
) {
    let first = (group.x + group.y * groups.x) * 64u;
    let tile = first + lane;
    var fill = 0xffffffffu;
    var count = 0u;
    if tile < params.tiles {
        fill = tile_fill(tile);
        count = atomicLoad(&tiles[tile].count);
    }
    prefix_fills[lane] = fill;
    prefix_values[lane] = count;
    workgroupBarrier();
    for (var step = 1u; step < 64u; step *= 2u) {
        var value = prefix_values[lane];
        if lane >= step && prefix_fills[lane - step] == fill {
            value += prefix_values[lane - step];
        }
        workgroupBarrier();
        prefix_values[lane] = value;
        workgroupBarrier();
    }
    if tile < params.tiles {
        tiles[tile].offset = prefix_values[lane] - count;
        if lane == 63u || tile + 1u == params.tiles || prefix_fills[lane + 1u] != fill {
            let start = max(first, fills[fill].first_tile);
            tiles[start].reserved = prefix_values[lane];
        }
    }
}

// One workgroup per fill scans its block sums in bounded batches. No group
// waits for another group: all block sums come from the preceding dispatch.
@compute @workgroup_size(64)
fn prefix_block_sums(
    @builtin(workgroup_id) group: vec3u,
    @builtin(local_invocation_index) lane: u32,
    @builtin(num_workgroups) groups: vec3u,
) {
    let index = group.x + group.y * groups.x;
    if index >= params.fills {
        return;
    }
    let fill = fills[index];
    let end = fill.first_tile + fill.columns * fill.tile_rows;
    let first_block = fill.first_tile / 64u;
    let blocks = (end - 1u) / 64u + 1u - first_block;
    if lane == 0u {
        prefix_base = fill.first_index;
    }
    workgroupBarrier();
    for (var batch = 0u; batch < blocks; batch += 64u) {
        let block = batch + lane;
        var sum = 0u;
        var start = 0u;
        if block < blocks {
            start = max(fill.first_tile, (first_block + block) * 64u);
            sum = tiles[start].reserved;
        }
        prefix_values[lane] = sum;
        workgroupBarrier();
        for (var step = 1u; step < 64u; step *= 2u) {
            var value = prefix_values[lane];
            if lane >= step {
                value += prefix_values[lane - step];
            }
            workgroupBarrier();
            prefix_values[lane] = value;
            workgroupBarrier();
        }
        if block < blocks {
            tiles[start].reserved = prefix_base + prefix_values[lane] - sum;
        }
        workgroupBarrier();
        if lane == 0u {
            prefix_base += prefix_values[63u];
        }
        workgroupBarrier();
    }
}

@compute @workgroup_size(64)
fn prefix_add_bases(@builtin(global_invocation_id) id: vec3u, @builtin(num_workgroups) groups: vec3u) {
    let tile = id.x + id.y * groups.x * 64u;
    if tile < params.tiles {
        let fill = fills[tile_fill(tile)];
        let start = max(tile / 64u * 64u, fill.first_tile);
        tiles[tile].offset += tiles[start].reserved;
    }
}

@compute @workgroup_size(64)
fn fill_tiles(@builtin(global_invocation_id) id: vec3u, @builtin(num_workgroups) groups: vec3u) {
    let index = id.x + id.y * groups.x * 64u;
    if index < params.edges {
        bin_edge(index, true);
    }
}

fn edge_x(edge: vec4f, y: f32) -> f32 {
    return edge.x + (edge.z - edge.x) * ((y - edge.y) / (edge.w - edge.y));
}

// Winding along a boundary changes only at an intersection or an endpoint
// of a coincident overlap. No subpixel sampling grid is involved.
fn next_boundary_cut(edge: vec4f, other: vec4f, start: f32, end: f32) -> f32 {
    // Evaluate each pair at fixed endpoints. Re-solving from the advancing
    // start can rediscover one intersection a few ulps later on every iteration.
    let top = max(edge.y, other.y);
    let bottom = min(edge.w, other.w);
    if top >= bottom {
        return end;
    }
    let first = edge_x(edge, top) - edge_x(other, top);
    let last = edge_x(edge, bottom) - edge_x(other, bottom);
    if first == 0.0 && last == 0.0 {
        if top > start {
            return min(end, top);
        }
        return select(end, min(end, bottom), bottom > start);
    }
    if first == 0.0 && top > start {
        return min(end, top);
    }
    if last == 0.0 && bottom > start {
        return min(end, bottom);
    }
    if (first < 0.0 && last > 0.0) || (first > 0.0 && last < 0.0) {
        let scale = max(abs(first), abs(last));
        let fraction = (abs(first) / scale) / (abs(first) / scale + abs(last) / scale);
        let crossing = top + (bottom - top) * fraction;
        if crossing > start && crossing < end {
            return crossing;
        }
    }
    return end;
}

fn inside(winding: i32, rule: u32) -> bool {
    return select(winding != 0, winding % 2 != 0, rule != 0u);
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

fn integrate_boundary(tile: u32, row: u32, edge_index: u32, origin: vec2f, rule: u32) -> array<f32, 16> {
    var areas: array<f32, 16>;
    let points = edges[edge_index].points;
    let edge = upper_lower(points) - origin.xyxy;
    var start = max(edge.y, f32(row));
    let bottom = min(edge.w, f32(row + 1u));
    if edge.w <= edge.y || start >= bottom {
        return areas;
    }
    let first = tiles[tile].offset;
    let last = first + atomicLoad(&tiles[tile].count);
    // At most two events per other edge, plus the final interval. Host
    // admission bounds candidate pairs and both scans before dispatch.
    for (var event = 0u; event <= 2u * (last - first) && start < bottom; event++) {
        var end = bottom;
        for (var item = first; item < last; item++) {
            let other_index = indices[item];
            // A separately optimized projection of this same sloping edge
            // need not round identically to the register-held boundary.
            if other_index == edge_index {
                continue;
            }
            let other = upper_lower(edges[other_index].points) - origin.xyxy;
            end = min(end, next_boundary_cut(edge, other, start, bottom));
        }
        let middle = start + (end - start) * 0.5;
        let horizontal = edge_x(edge, middle);
        var winding = 0i;
        for (var item = first; item < last; item++) {
            let other_index = indices[item];
            if other_index == edge_index {
                continue;
            }
            let other_points = edges[other_index].points;
            let other = upper_lower(other_points) - origin.xyxy;
            if other.y <= middle && middle < other.w {
                let x = edge_x(other, middle);
                // Stable source-edge order resolves coincident boundaries,
                // independently of the atomic scatter order.
                let coincident = all(other == edge);
                if select(x < horizontal, other_index < edge_index, coincident || x == horizontal) {
                    winding += select(-1i, 1i, other_points.w > other_points.y);
                }
            }
        }
        let next = winding + select(-1i, 1i, points.w > points.y);
        let direction = f32(i32(inside(next, rule)) - i32(inside(winding, rule)));
        if direction != 0.0 {
            let xa = edge_x(edge, start);
            let xb = edge_x(edge, end);
            for (var column = 0u; column < TILE_SIZE; column++) {
                areas[column] += direction * column_area(xa, xb, f32(column), end - start);
            }
        }
        start = end;
    }
    return areas;
}

// One workgroup owns a tile row. Fixed batches bound scratch independently
// of the number of intersections, without materializing a crossing list.
@compute @workgroup_size(64)
fn boundary_rows(
    @builtin(workgroup_id) group: vec3u,
    @builtin(local_invocation_index) lane: u32,
    @builtin(num_workgroups) groups: vec3u,
) {
    let index = group.x + group.y * groups.x;
    let tile = index / TILE_SIZE;
    let row = index % TILE_SIZE;
    if tile >= params.tiles {
        return;
    }
    let fill = fills[tile_fill(tile)];
    let local_tile = tile - fill.first_tile;
    if local_tile / fill.columns * TILE_SIZE + row >= fill.size.y {
        return;
    }
    let origin = vec2f(vec2u(local_tile % fill.columns, local_tile / fill.columns)) * f32(TILE_SIZE);
    let first = tiles[tile].offset;
    let count = atomicLoad(&tiles[tile].count);
    var total = 0.0;
    for (var batch = 0u; batch < count; batch += 64u) {
        var areas: array<f32, 16>;
        if batch + lane < count {
            areas = integrate_boundary(tile, row, indices[first + batch + lane], origin, fill.rule);
        }
        for (var column = 0u; column < TILE_SIZE; column++) {
            boundary_areas[lane * TILE_SIZE + column] = areas[column];
        }
        workgroupBarrier();
        if lane < TILE_SIZE {
            for (var item = 0u; item < min(64u, count - batch); item++) {
                total += boundary_areas[item * TILE_SIZE + lane];
            }
        }
        workgroupBarrier();
    }
    if lane < TILE_SIZE {
        row_areas[(tile * TILE_SIZE + row) * BOUNDARY_STRIDE + lane] = total;
    }
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
    let value = clamp(row_areas[(tile * TILE_SIZE + local.y) * BOUNDARY_STRIDE + local.x], 0.0, 1.0);
    coverage[fill.coverage + pixel.y * fill.size.x + pixel.x] = bitcast<u32>(value);
}
