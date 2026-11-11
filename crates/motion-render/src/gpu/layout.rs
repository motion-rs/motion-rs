//! Compile-time checks that host uniform structs keep the layout the WGSL `GpuUniforms` expects.

use super::{FrameHeader, GpuUniforms, OperationHeader, SourceFlags};
use std::mem::{offset_of, size_of};

use super::abi::{CoverageEdge, CoverageFill, CoverageParams, CoverageTile};
use crate::raster::device::{COVERAGE_EDGE_BYTES, COVERAGE_FILL_BYTES, COVERAGE_PARAMS_BYTES};

const _: () = {
    assert!(size_of::<CoverageParams>() as u64 == COVERAGE_PARAMS_BYTES);
    assert!(offset_of!(CoverageParams, fills) == 0);
    assert!(offset_of!(CoverageParams, edges) == 4);
    assert!(offset_of!(CoverageParams, tiles) == 8);
    assert!(offset_of!(CoverageParams, rows) == 12);
    assert!(size_of::<CoverageFill>() as u64 == COVERAGE_FILL_BYTES);
    assert!(offset_of!(CoverageFill, size) == 0);
    assert!(offset_of!(CoverageFill, coverage) == 8);
    assert!(offset_of!(CoverageFill, rule) == 12);
    assert!(offset_of!(CoverageFill, first_edge) == 16);
    assert!(offset_of!(CoverageFill, edge_count) == 20);
    assert!(offset_of!(CoverageFill, first_tile) == 24);
    assert!(offset_of!(CoverageFill, columns) == 28);
    assert!(offset_of!(CoverageFill, first_index) == 32);
    assert!(offset_of!(CoverageFill, index_capacity) == 36);
    assert!(offset_of!(CoverageFill, first_row) == 40);
    assert!(offset_of!(CoverageFill, tile_rows) == 44);
    assert!(size_of::<CoverageEdge>() as u64 == COVERAGE_EDGE_BYTES);
    assert!(offset_of!(CoverageEdge, points) == 0);
    assert!(offset_of!(CoverageEdge, fill) == 16);
    assert!(offset_of!(CoverageEdge, reserved) == 20);
    assert!(size_of::<CoverageTile>() == 16);
    assert!(offset_of!(CoverageTile, count) == 0);
    assert!(offset_of!(CoverageTile, offset) == 4);
    assert!(offset_of!(CoverageTile, cursor) == 8);
    assert!(offset_of!(CoverageTile, reserved) == 12);
};

const _: () = assert!(size_of::<FrameHeader>() == 16);

const _: () = assert!(offset_of!(FrameHeader, width) == 0);

const _: () = assert!(offset_of!(FrameHeader, height) == 4);

const _: () = assert!(offset_of!(FrameHeader, left) == 8);

const _: () = assert!(offset_of!(FrameHeader, top) == 12);

const _: () = assert!(size_of::<OperationHeader>() == 32);

const _: () = assert!(offset_of!(OperationHeader, reserved) == 0);

const _: () = assert!(offset_of!(OperationHeader, blend_space) == 12);

const _: () = assert!(offset_of!(OperationHeader, code) == 16);

const _: () = assert!(offset_of!(OperationHeader, kind) == 20);

const _: () = assert!(offset_of!(OperationHeader, detail) == 24);

const _: () = assert!(offset_of!(OperationHeader, extent) == 28);

const _: () = assert!(size_of::<SourceFlags>() == 16);

const _: () = assert!(offset_of!(SourceFlags, particle_tiles) == 0);

const _: () = assert!(offset_of!(SourceFlags, reserved) == 4);

const _: () = assert!(offset_of!(SourceFlags, first_deferred) == 8);

const _: () = assert!(offset_of!(SourceFlags, second_deferred) == 12);

const _: () = assert!(size_of::<GpuUniforms>() == 256);

const _: () = assert!(offset_of!(GpuUniforms, frame) == 0);

const _: () = assert!(offset_of!(GpuUniforms, bounds) == 16);

const _: () = assert!(offset_of!(GpuUniforms, operation) == 32);

const _: () = assert!(offset_of!(GpuUniforms, values) == 64);

const _: () = assert!(offset_of!(GpuUniforms, sources) == 192);

const _: () = assert!(offset_of!(GpuUniforms, flags) == 240);
