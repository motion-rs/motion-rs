// GPU curve flattening, edge binning and f32 analytic area coverage: signed-area accumulation
// folded by the fill rule, as in Skia's analytic AA and Vello's fine rasterizer, with 4 x 4
// sampled refinement where contours overlap inside a pixel.
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
@group(0) @binding(5) var<storage, read_write> carries: array<i32>;
@group(0) @binding(6) var<storage, read_write> coverage: array<u32>;
@group(0) @binding(7) var<storage, read> curves: array<CoverageCurve>;

const TILE_SIZE = 16u;
// Sample rows and columns per pixel in the overlap refinement.
const WINDING_SAMPLES = 4u;
// Signed row height plus one winding per sample row carried into each tile row;
// mirrored by the host planner.
const CARRY_STRIDE = 5u;
// Areas are summed in wrapping fixed point, so totals do not depend on the atomic order
// in which edges were scattered into a tile.
const AREA_SCALE = 1048576.0;

var<workgroup> prefix_values: array<u32, 64>;
var<workgroup> prefix_fills: array<u32, 64>;
var<workgroup> prefix_base: u32;

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

fn fixed_area(area: f32) -> i32 {
    return i32(round(area * AREA_SCALE));
}

fn row_height(points: vec4f, row: u32) -> f32 {
    let edge = upper_lower(points);
    let height = max(0.0, min(edge.w, f32(row + 1u)) - max(edge.y, f32(row)));
    return select(-height, height, points.w > points.y);
}

// Sample rows per pixel row used to refine pixels where contours overlap.
fn sample_rows(row: u32) -> vec4f {
    return f32(row) + (vec4f(0.0, 1.0, 2.0, 3.0) + 0.5) / f32(WINDING_SAMPLES);
}

// Signed row height and sample-row windings carried into each tile row. An edge is
// carried only past its final column in the band; in its other tiles it is evaluated
// locally, so long sloping edges are never doubled.
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
    let y = sample_rows(row);
    var total = 0i;
    var winding = vec4i(0i);
    for (var column = 0u; column < fill.columns; column++) {
        let tile = fill.first_tile + band * fill.columns + column;
        let carry = (tile * TILE_SIZE + row % TILE_SIZE) * CARRY_STRIDE;
        carries[carry] = total;
        for (var sample = 0u; sample < WINDING_SAMPLES; sample++) {
            carries[carry + 1u + sample] = winding[sample];
        }
        let first = tiles[tile].offset;
        let last = first + atomicLoad(&tiles[tile].count);
        for (var item = first; item < last; item++) {
            let points = edges[indices[item]].points;
            if tile_columns(points, band, fill.columns).y == column {
                let edge = upper_lower(points);
                let inside = vec4i((vec4f(edge.y) <= y) & (y < vec4f(edge.w)));
                total += fixed_area(row_height(points, row));
                winding += inside * select(-1i, 1i, points.w > points.y);
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

// Accumulated signed area and a conservative count of winding changes inside the pixel:
// one per edge crossing it, two per edge that starts or ends inside the row at or left of it.
fn signed_area(tile: u32, pixel: vec2u) -> vec2f {
    let origin = vec2f((pixel / TILE_SIZE) * TILE_SIZE);
    let top = f32(pixel.y % TILE_SIZE);
    let column = f32(pixel.x % TILE_SIZE);
    let first = tiles[tile].offset;
    let last = first + atomicLoad(&tiles[tile].count);
    var total = carries[(tile * TILE_SIZE + pixel.y % TILE_SIZE) * CARRY_STRIDE];
    var boundaries = 0.0;
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
        if max(xa, xb) > column && min(xa, xb) < column + 1.0 {
            boundaries += 1.0;
        }
        if (edge.y > top || edge.w < top + 1.0) && min(xa, xb) < column + 1.0 {
            boundaries += 2.0;
        }
        let area = column_area(xa, xb, column, y_bottom - y_top);
        total += fixed_area(select(-area, area, points.w > points.y));
    }
    return vec2f(f32(total) / AREA_SCALE, boundaries);
}

fn rule_coverage(total: f32, rule: u32) -> f32 {
    let magnitude = abs(total);
    if rule == 0u {
        return min(magnitude, 1.0);
    }
    let folded = magnitude - 2.0 * floor(magnitude * 0.5);
    return select(folded, 2.0 - folded, folded > 1.0);
}

// Folding the integrated winding equals integrating the fill rule only while the winding
// varies by at most one inside the pixel. Otherwise, where contours overlap inside it, the
// rule is applied to a 4 x 4 grid of sampled windings instead.
fn refined_coverage(tile: u32, pixel: vec2u, rule: u32, area: vec2f) -> f32 {
    let analytic = rule_coverage(area.x, rule);
    let carry = (tile * TILE_SIZE + pixel.y % TILE_SIZE) * CARRY_STRIDE;
    // Edges of earlier tiles: the winding left of this tile at each sample row.
    let left = vec4f(vec4i(
        carries[carry + 1u], carries[carry + 2u], carries[carry + 3u], carries[carry + 4u],
    ));
    let uniform_carry = all(left == vec4f(left.x));
    if uniform_carry && abs(left.x) <= 1.0 && abs(area.x) <= 1.0 && area.y <= 1.0 {
        return analytic;
    }
    let y = sample_rows(pixel.y);
    let x = sample_rows(pixel.x);
    // windings[row][column] for sample (x[column], y[row]).
    var windings = mat4x4f(vec4f(left.x), vec4f(left.y), vec4f(left.z), vec4f(left.w));
    let first = tiles[tile].offset;
    let last = first + atomicLoad(&tiles[tile].count);
    for (var item = first; item < last; item++) {
        let points = edges[indices[item]].points;
        let edge = upper_lower(points);
        if edge.w <= edge.y || edge.w <= y.x || edge.y > y.w {
            continue;
        }
        let direction = select(-1.0, 1.0, points.w > points.y);
        let slope = (edge.z - edge.x) / (edge.w - edge.y);
        for (var row = 0u; row < WINDING_SAMPLES; row++) {
            if edge.y <= y[row] && y[row] < edge.w {
                let crossing = edge.x + slope * (y[row] - edge.y);
                windings[row] += vec4f(vec4f(crossing) <= x) * direction;
            }
        }
    }
    var minimum = windings[0].x;
    var maximum = windings[0].x;
    var covered = 0.0;
    for (var row = 0u; row < WINDING_SAMPLES; row++) {
        let values = windings[row];
        minimum = min(minimum, min(min(values.x, values.y), min(values.z, values.w)));
        maximum = max(maximum, max(max(values.x, values.y), max(values.z, values.w)));
        for (var column = 0u; column < WINDING_SAMPLES; column++) {
            covered += rule_coverage(values[column], rule);
        }
    }
    // Only the two ranges adjacent to zero can reuse area without interpreting a
    // potentially wrapped fixed-point total at high winding multiplicities.
    if (minimum >= 0.0 && maximum <= 1.0) || (minimum >= -1.0 && maximum <= 0.0) {
        return analytic;
    }
    return covered / f32(WINDING_SAMPLES * WINDING_SAMPLES);
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
    let fill = fills[tile_fill(tile)];
    let index = tile - fill.first_tile;
    let pixel = vec2u(index % fill.columns, index / fill.columns) * TILE_SIZE + local.xy;
    if any(pixel >= fill.size) {
        return;
    }
    let value = refined_coverage(tile, pixel, fill.rule, signed_area(tile, pixel));
    coverage[fill.coverage + pixel.y * fill.size.x + pixel.x] = bitcast<u32>(clamp(value, 0.0, 1.0));
}
