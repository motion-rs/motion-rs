//! Compile-time checks that host uniform structs keep the layout the WGSL `GpuUniforms` expects.

use super::{FrameHeader, GpuUniforms, OperationHeader, SourceFlags};
use std::mem::{offset_of, size_of};

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
