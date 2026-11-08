//! Host side of the GPU uniform ABI: operation codes, paint kinds, the `GpuUniforms`
//! record shared with WGSL, and the generated shader constants that mirror them.

use crate::prepared::{PixelRect, PreparedFrame};
use motion_core::ValidationError;
use motion_core::drawing::BlendMode;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(u32)]
pub(super) enum GpuOp {
    Clear = 0,
    Fill = 1,
    Composite = 2,
    Mask = 3,
    Blur = 4,
    Offset = 5,
    ColorMatrix = 6,
    Threshold = 7,
    Noise = 8,
    ChannelOffset = 9,
    ColorAdjust = 11,
    Colorize = 12,
    Scale = 13,
    Lut = 14,
    Cdl = 15,
    Optics = 16,
    Particles = 18,
    /// Averages each 2x2 block of the first source into one coarser Bloom pyramid pixel.
    Downsample = 19,
    /// Weights the first source and adds the coarser second source sampled bilinearly.
    Upsample = 20,
}

#[derive(Clone, Copy)]
#[repr(u32)]
pub(super) enum PaintKind {
    Linear = 1,
    Radial = 2,
    Sweep = 3,
    Pattern = 4,
    RadialFocal = 5,
}

#[derive(Clone, Copy, Default, bytemuck::Pod, bytemuck::Zeroable)]
#[repr(C)]
pub(super) struct FrameHeader {
    pub(super) width: u32,
    pub(super) height: u32,
    pub(super) left: u32,
    pub(super) top: u32,
}

#[derive(Clone, Copy, Default, bytemuck::Pod, bytemuck::Zeroable)]
#[repr(C)]
pub(super) struct UniformRect {
    pub(super) left: u32,
    pub(super) top: u32,
    pub(super) right: u32,
    pub(super) bottom: u32,
}

impl From<PixelRect> for UniformRect {
    fn from(bounds: PixelRect) -> Self {
        Self {
            left: bounds.left,
            top: bounds.top,
            right: bounds.right,
            bottom: bounds.bottom,
        }
    }
}

#[derive(Clone, Copy, Default, bytemuck::Pod, bytemuck::Zeroable)]
#[repr(C)]
pub(super) struct OperationHeader {
    pub(super) reserved: [u32; 3],
    pub(super) blend_space: u32,
    pub(super) code: u32,
    pub(super) kind: u32,
    pub(super) detail: u32,
    pub(super) extent: u32,
}

#[derive(Clone, Copy, Default, bytemuck::Pod, bytemuck::Zeroable)]
#[repr(C)]
pub(super) struct SourceFlags {
    pub(super) particle_tiles: u32,
    pub(super) reserved: u32,
    pub(super) first_deferred: u32,
    pub(super) second_deferred: u32,
}

#[derive(Clone, Copy, Default, bytemuck::Pod, bytemuck::Zeroable)]
#[repr(C)]
pub(super) struct GpuUniforms {
    pub(super) frame: FrameHeader,
    pub(super) bounds: UniformRect,
    pub(super) operation: OperationHeader,
    pub(super) values: [f32; 32],
    pub(super) sources: [UniformRect; 3],
    pub(super) flags: SourceFlags,
}

impl GpuUniforms {
    pub(super) fn new(frame: &PreparedFrame, operation: GpuOp, bounds: PixelRect) -> Self {
        Self {
            frame: FrameHeader {
                width: frame.size.width,
                height: frame.size.height,
                left: frame.output_bounds.left,
                top: frame.output_bounds.top,
            },
            bounds: bounds.into(),
            operation: OperationHeader {
                code: operation as u32,
                blend_space: frame.blend_space as u32,
                ..Default::default()
            },
            ..Default::default()
        }
    }

    pub(super) fn gradient(
        &mut self,
        kind: PaintKind,
        units: motion_core::GradientUnits,
        extend: motion_core::GradientExtend,
    ) {
        self.operation.kind = kind as u32;
        self.operation.detail = units as u32;
        self.operation.extent = extend as u32;
    }

    pub(super) fn pattern(&mut self, extend: motion_core::GradientExtend) {
        self.operation.kind = PaintKind::Pattern as u32;
        self.operation.extent = extend as u32;
    }

    pub(super) fn is_pattern(&self) -> bool {
        self.operation.kind == PaintKind::Pattern as u32
    }

    pub(super) fn blend(&mut self, blend: BlendMode) {
        self.operation.kind = mode(blend);
    }

    pub(super) fn mask(&mut self, mask: motion_core::MaskMode) {
        self.operation.detail = match mask {
            motion_core::MaskMode::Alpha => 0,
            motion_core::MaskMode::InvertedAlpha => 1,
            motion_core::MaskMode::Luminance => 2,
            motion_core::MaskMode::InvertedLuminance => 3,
        };
    }

    pub(super) fn blur(&mut self, horizontal: bool, radius: u32) {
        self.operation.detail = u32::from(horizontal);
        self.operation.extent = radius;
    }

    pub(super) fn noise(&mut self, seed: u64) {
        self.operation.detail = seed as u32;
        self.operation.extent = (seed >> 32) as u32;
    }

    pub(super) fn rectangle(&self) -> [f32; 4] {
        [
            self.bounds.left as f32,
            self.bounds.top as f32,
            self.bounds.right as f32,
            self.bounds.bottom as f32,
        ]
    }

    pub(super) fn image_bounds(&self, index: usize) -> PixelRect {
        let bounds = self.sources[index];
        PixelRect {
            left: bounds.left,
            top: bounds.top,
            right: bounds.right,
            bottom: bounds.bottom,
        }
    }

    pub(super) fn images(&mut self, images: [PixelRect; 3]) {
        self.sources = images.map(UniformRect::from);
    }

    pub(super) fn values(&mut self, values: &[f64]) -> Result<(), ValidationError> {
        if values.len() > 32 {
            return Err(ValidationError::invalid(
                "gpu.parameters",
                "values",
                "GPU parameter values exceed the uniform layout",
            ));
        }
        for (index, value) in values.iter().enumerate() {
            let narrowed = *value as f32;
            if !narrowed.is_finite() || (*value != 0.0 && narrowed == 0.0) {
                return Err(ValidationError::unsupported(
                    "gpu.parameters",
                    "precision",
                    "Native GPU parameters require finite representable f32 values",
                ));
            }
            self.values[index] = narrowed;
        }
        Ok(())
    }
}

pub(crate) fn output_shader(source: &str) -> String {
    let lut =
        include_str!("shaders/lut.wgsl").replace("operation_data.values", "output_lut.values");
    [
        source,
        "\n",
        include_str!("shaders/noise.wgsl"),
        lut.as_str(),
        include_str!("shaders/tonemap.wgsl"),
    ]
    .concat()
}

pub(super) fn mode(mode: BlendMode) -> u32 {
    match mode {
        BlendMode::SourceOver => 0,
        BlendMode::Plus => 1,
        BlendMode::Multiply => 2,
        BlendMode::Screen => 3,
        BlendMode::Darken => 4,
        BlendMode::Lighten => 5,
        BlendMode::Difference => 6,
        BlendMode::Exclusion => 7,
        BlendMode::Overlay => 8,
        BlendMode::HardLight => 9,
        BlendMode::SoftLight => 10,
        BlendMode::ColorDodge => 11,
        BlendMode::ColorBurn => 12,
        BlendMode::Hue => 13,
        BlendMode::Saturation => 14,
        BlendMode::Color => 15,
        BlendMode::Luminosity => 16,
        BlendMode::LinearPlus => 17,
    }
}

pub(super) fn shader_constants() -> String {
    use std::fmt::Write;
    let mut source = String::new();
    for (name, value) in [
        ("OP_CLEAR", GpuOp::Clear as u32),
        ("OP_FILL", GpuOp::Fill as u32),
        ("OP_COMPOSITE", GpuOp::Composite as u32),
        ("OP_MASK", GpuOp::Mask as u32),
        ("OP_BLUR", GpuOp::Blur as u32),
        ("OP_OFFSET", GpuOp::Offset as u32),
        ("OP_COLOR_MATRIX", GpuOp::ColorMatrix as u32),
        ("OP_THRESHOLD", GpuOp::Threshold as u32),
        ("OP_NOISE", GpuOp::Noise as u32),
        ("OP_CHANNEL_OFFSET", GpuOp::ChannelOffset as u32),
        ("OP_COLOR_ADJUST", GpuOp::ColorAdjust as u32),
        ("OP_COLORIZE", GpuOp::Colorize as u32),
        ("OP_SCALE", GpuOp::Scale as u32),
        ("OP_LUT", GpuOp::Lut as u32),
        ("OP_CDL", GpuOp::Cdl as u32),
        ("OP_OPTICS", GpuOp::Optics as u32),
        ("OP_PARTICLES", GpuOp::Particles as u32),
        ("OP_DOWNSAMPLE", GpuOp::Downsample as u32),
        ("OP_UPSAMPLE", GpuOp::Upsample as u32),
        ("BLEND_SOURCE_OVER", mode(BlendMode::SourceOver)),
        ("BLEND_PLUS", mode(BlendMode::Plus)),
        ("BLEND_MULTIPLY", mode(BlendMode::Multiply)),
        ("BLEND_SCREEN", mode(BlendMode::Screen)),
        ("BLEND_DARKEN", mode(BlendMode::Darken)),
        ("BLEND_LIGHTEN", mode(BlendMode::Lighten)),
        ("BLEND_DIFFERENCE", mode(BlendMode::Difference)),
        ("BLEND_EXCLUSION", mode(BlendMode::Exclusion)),
        ("BLEND_OVERLAY", mode(BlendMode::Overlay)),
        ("BLEND_HARD_LIGHT", mode(BlendMode::HardLight)),
        ("BLEND_SOFT_LIGHT", mode(BlendMode::SoftLight)),
        ("BLEND_COLOR_DODGE", mode(BlendMode::ColorDodge)),
        ("BLEND_COLOR_BURN", mode(BlendMode::ColorBurn)),
        ("BLEND_HUE", mode(BlendMode::Hue)),
        ("BLEND_SATURATION", mode(BlendMode::Saturation)),
        ("BLEND_COLOR", mode(BlendMode::Color)),
        ("BLEND_LUMINOSITY", mode(BlendMode::Luminosity)),
        ("BLEND_LINEAR_PLUS", mode(BlendMode::LinearPlus)),
    ] {
        writeln!(source, "const {name}: u32 = {value}u;")
            .expect("formatting a shader constant into a String cannot fail");
    }
    source
}
