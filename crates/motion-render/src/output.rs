//! Validated memory targets and shared CPU/GPU output conversion.

use crate::RasterLimits;
use crate::linear::LinearFrame;
use motion_core::{PixelSize, ValidationError};

mod bands;
mod png;
mod rgb;
#[cfg(test)]
mod tests;
mod yuv;

/// Bytes of one linear-sRGB premultiplied f32 RGBA pixel.
const LINEAR_PIXEL_BYTES: u64 = 16;

/// Bytes of the widest packed output pixel (single-float RGBA).
const MAX_OUTPUT_PIXEL_BYTES: u64 = 16;

/// Planes of the most planar output format (Y, U and V).
const MAX_PLANES: u64 = 3;

/// Rows converted for constant output before they are replicated over the frame.
const CONSTANT_BAND_ROWS: u32 = 16;

pub(crate) type OutputPool = crate::cpu::pool::BufferPool<u8>;

/// Explicit storage/color/alpha contract of an owned output buffer.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PixelFormat {
    /// Little-endian half-float linear-sRGB premultiplied RGBA.
    LinearRgba16Float,
    /// Little-endian f32 linear-sRGB premultiplied RGBA, exactly as composed and unquantized.
    /// RGB may be extended or negative; no display transform applies.
    LinearRgba32Float,
    /// Encoded sRGB with linear alpha; premultiplication is encoded-space.
    SrgbRgba8 {
        /// Whether RGB is multiplied by alpha.
        premultiplied: bool,
    },
    /// Planar BT.709 limited-range Y/U/V; 8-bit bytes or 10-bit LE u16 samples.
    Yuv420 {
        /// Supported depths are 8 and 10; dimensions must be even.
        bit_depth: u8,
    },
}

/// One tightly packed plane; strides are bytes, not samples.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PixelPlane {
    width: u32,
    height: u32,
    stride: usize,
    bytes: Vec<u8>,
}

impl PixelPlane {
    /// Plane width in samples (RGBA planes have one packed pixel per sample).
    pub fn width(&self) -> u32 {
        self.width
    }

    /// Plane row count.
    pub fn height(&self) -> u32 {
        self.height
    }

    /// Byte distance between adjacent rows.
    pub fn stride(&self) -> usize {
        self.stride
    }

    /// Tightly packed immutable bytes.
    pub fn bytes(&self) -> &[u8] {
        &self.bytes
    }
}

/// Owned validated output, with unforgeable plane dimensions and storage sizes.
#[derive(Clone, Debug)]
pub struct FrameBuffer {
    size: PixelSize,
    format: PixelFormat,
    planes: Vec<PixelPlane>,
    pool: Option<OutputPool>,
}

impl FrameBuffer {
    pub(crate) fn from_constant(
        size: PixelSize,
        pixel: [f32; 4],
        format: PixelFormat,
        limits: RasterLimits,
    ) -> Result<(Self, u64), ValidationError> {
        let depth = match format {
            PixelFormat::SrgbRgba8 { .. } => 4,
            PixelFormat::LinearRgba16Float => 8,
            PixelFormat::LinearRgba32Float => 16,
            PixelFormat::Yuv420 { bit_depth } => {
                if !size.width.is_multiple_of(2)
                    || !size.height.is_multiple_of(2)
                    || ![8, 10].contains(&bit_depth)
                {
                    return Err(output_invalid(
                        "yuv420",
                        "YUV420 requires even dimensions and 8/10-bit samples",
                    ));
                }
                if bit_depth == 8 { 1 } else { 2 }
            }
        };
        let count = u64::from(size.width) * u64::from(size.height);
        let payload = if matches!(format, PixelFormat::Yuv420 { .. }) {
            count * 3 / 2 * depth
        } else {
            count * depth
        };
        let height = size.height.min(CONSTANT_BAND_ROWS);
        let band_pixels = u64::from(size.width) * u64::from(height);
        let scratch = band_pixels * (LINEAR_PIXEL_BYTES + MAX_OUTPUT_PIXEL_BYTES)
            + std::mem::size_of::<LinearFrame>() as u64
            + std::mem::size_of::<Self>() as u64
            + MAX_PLANES * std::mem::size_of::<PixelPlane>() as u64;
        let output = payload
            + std::mem::size_of::<Self>() as u64
            + MAX_PLANES * std::mem::size_of::<PixelPlane>() as u64;
        validate_conversion_budget(size, scratch, output, limits)?;
        if Self::conversion_work(size, format) > limits.max_work {
            return Err(ValidationError::limit(
                "output",
                "max_work",
                "Output conversion passes exceed the work allowance",
            ));
        }
        let linear = LinearFrame::constant(PixelSize::new(size.width, height), pixel);
        let mut band = Self::from_linear(&linear, format, limits)?;
        let mut planes = Vec::new();
        for (index, plane) in std::mem::take(&mut band.planes).into_iter().enumerate() {
            let rows = if index == 0 {
                size.height
            } else {
                size.height / 2
            };
            let bytes = rows as usize * plane.stride;
            let mut data = Vec::new();
            data.try_reserve_exact(bytes)
                .map_err(|_| output_invalid("max_bytes", "Could not reserve constant output"))?;
            while data.len() < bytes {
                let count = (bytes - data.len()).min(plane.bytes.len());
                data.extend_from_slice(&plane.bytes[..count]);
            }
            planes.push(PixelPlane {
                width: plane.width,
                height: rows,
                stride: plane.stride,
                bytes: data,
            });
        }
        Ok((
            Self {
                size,
                format,
                planes,
                pool: None,
            },
            scratch + output,
        ))
    }

    pub(crate) fn vacant() -> Self {
        Self {
            size: PixelSize::new(0, 0),
            format: PixelFormat::SrgbRgba8 {
                premultiplied: false,
            },
            planes: Vec::new(),
            pool: None,
        }
    }

    /// Freezes tightly packed Y/U/V planes with checked 8-bit 4:2:0 dimensions.
    pub fn from_yuv420(
        size: PixelSize,
        y: Vec<u8>,
        u: Vec<u8>,
        v: Vec<u8>,
        limits: RasterLimits,
    ) -> Result<Self, ValidationError> {
        Self::from_yuv420_storage(size, [y, u, v], 8, limits)
    }

    pub(crate) fn from_yuv420_storage(
        size: PixelSize,
        planes: [Vec<u8>; 3],
        bit_depth: u8,
        limits: RasterLimits,
    ) -> Result<Self, ValidationError> {
        if ![8, 10].contains(&bit_depth) {
            return Err(output_invalid(
                "bit_depth",
                "YUV420 requires 8/10-bit samples",
            ));
        }
        let [y, u, v] = planes;
        let sample_bytes = if bit_depth == 8 { 1 } else { 2 };
        let count = u64::from(size.width) * u64::from(size.height);
        let luma_bytes = count * sample_bytes;
        validate_conversion_budget(
            size,
            0,
            luma_bytes * 3 / 2
                + std::mem::size_of::<Self>() as u64
                + MAX_PLANES * std::mem::size_of::<PixelPlane>() as u64,
            limits,
        )?;
        if !size.width.is_multiple_of(2)
            || !size.height.is_multiple_of(2)
            || y.len() as u64 != luma_bytes
            || u.len() as u64 != luma_bytes / 4
            || v.len() != u.len()
        {
            return Err(output_invalid(
                "planes",
                "YUV420 planes do not match even output dimensions",
            ));
        }
        Ok(Self {
            size,
            format: PixelFormat::Yuv420 { bit_depth },
            planes: vec![
                PixelPlane {
                    width: size.width,
                    height: size.height,
                    stride: size.width as usize * sample_bytes as usize,
                    bytes: y,
                },
                PixelPlane {
                    width: size.width / 2,
                    height: size.height / 2,
                    stride: size.width as usize / 2 * sample_bytes as usize,
                    bytes: u,
                },
                PixelPlane {
                    width: size.width / 2,
                    height: size.height / 2,
                    stride: size.width as usize / 2 * sample_bytes as usize,
                    bytes: v,
                },
            ],
            pool: None,
        })
    }

    /// Allocates a transparent encoded RGBA8 memory target.
    pub fn new(size: PixelSize, limits: RasterLimits) -> Result<Self, ValidationError> {
        let bytes = u64::from(size.width) * u64::from(size.height) * 4;
        validate_conversion_budget(
            size,
            0,
            bytes + std::mem::size_of::<Self>() as u64 + std::mem::size_of::<PixelPlane>() as u64,
            limits,
        )?;
        Ok(Self {
            size,
            format: PixelFormat::SrgbRgba8 {
                premultiplied: false,
            },
            planes: vec![PixelPlane {
                width: size.width,
                height: size.height,
                stride: size.width as usize * 4,
                bytes: vec![0; bytes as usize],
            }],
            pool: None,
        })
    }

    /// Validates and freezes already encoded straight RGBA pixels.
    pub fn from_rgba8(
        size: PixelSize,
        bytes: Vec<u8>,
        limits: RasterLimits,
    ) -> Result<Self, ValidationError> {
        let mut frame = Self::from_rgba8_storage(size, bytes, false, limits)?;
        for pixel in frame.planes[0].bytes.as_chunks_mut::<4>().0 {
            if pixel[3] == 0 {
                pixel.fill(0);
            }
        }
        Ok(frame)
    }

    pub(crate) fn from_rgba8_storage(
        size: PixelSize,
        bytes: Vec<u8>,
        premultiplied: bool,
        limits: RasterLimits,
    ) -> Result<Self, ValidationError> {
        validate_conversion_budget(
            size,
            0,
            bytes.len() as u64
                + std::mem::size_of::<Self>() as u64
                + std::mem::size_of::<PixelPlane>() as u64,
            limits,
        )?;
        if bytes.len() != size.width as usize * size.height as usize * 4 {
            return Err(output_invalid(
                "bytes",
                "RGBA payload does not match output dimensions",
            ));
        }
        Ok(Self {
            size,
            format: PixelFormat::SrgbRgba8 { premultiplied },
            planes: vec![PixelPlane {
                width: size.width,
                height: size.height,
                stride: size.width as usize * 4,
                bytes,
            }],
            pool: None,
        })
    }

    /// Converts linear output into straight sRGB RGBA8; transparent pixels keep their alpha.
    #[cfg(any(feature = "gpu", test))]
    pub(crate) fn from_linear_rgba8(
        frame: &LinearFrame,
        limits: RasterLimits,
    ) -> Result<Self, ValidationError> {
        Self::from_linear(
            frame,
            PixelFormat::SrgbRgba8 {
                premultiplied: false,
            },
            limits,
        )
    }

    /// Wraps pixels whose storage a GPU plan already admitted as linear f32 output.
    #[cfg(any(feature = "gpu", test))]
    pub(crate) fn linear_unbounded(frame: &LinearFrame) -> Result<Self, ValidationError> {
        let unbounded = RasterLimits {
            max_pixels: u64::MAX,
            max_bytes: u64::MAX,
            max_work: u64::MAX,
        };
        Self::from_linear(frame, PixelFormat::LinearRgba32Float, unbounded)
    }

    /// Converts the same immutable linear pixels regardless of rendering backend.
    pub(crate) fn from_linear(
        frame: &LinearFrame,
        format: PixelFormat,
        limits: RasterLimits,
    ) -> Result<Self, ValidationError> {
        Self::from_linear_pooled(frame, format, limits, &OutputPool::default())
    }

    fn from_linear_pooled(
        frame: &LinearFrame,
        format: PixelFormat,
        limits: RasterLimits,
        pool: &OutputPool,
    ) -> Result<Self, ValidationError> {
        Self::from_linear_transformed(
            frame,
            format,
            limits,
            pool,
            crate::display::DisplaySettings::default(),
        )
    }

    /// Converts scene-linear pixels using explicit grading and frame-addressed grain.
    /// LinearRgba16Float remains scene-linear, without a display transform.
    pub(crate) fn from_linear_display(
        frame: &LinearFrame,
        format: PixelFormat,
        limits: RasterLimits,
        display: &crate::DisplayTransform,
        frame_index: u64,
    ) -> Result<Self, ValidationError> {
        display.validate()?;
        Self::from_linear_transformed(
            frame,
            format,
            limits,
            &OutputPool::default(),
            crate::display::DisplaySettings {
                display: (!display.is_identity()).then_some(display),
                origin: [0; 2],
                frame: frame_index,
            },
        )
    }

    fn from_linear_transformed(
        frame: &LinearFrame,
        format: PixelFormat,
        limits: RasterLimits,
        pool: &OutputPool,
        display: crate::display::DisplaySettings<'_>,
    ) -> Result<Self, ValidationError> {
        let size = frame.size();
        let count = u64::from(size.width) * u64::from(size.height);
        let bytes = match format {
            PixelFormat::SrgbRgba8 { .. } => count * 4,
            PixelFormat::LinearRgba16Float => count * 8,
            PixelFormat::LinearRgba32Float => count * 16,
            PixelFormat::Yuv420 { bit_depth } => {
                if !size.width.is_multiple_of(2)
                    || !size.height.is_multiple_of(2)
                    || ![8, 10].contains(&bit_depth)
                {
                    return Err(output_invalid(
                        "yuv420",
                        "YUV420 requires even dimensions and 8/10-bit samples",
                    ));
                }
                count * 3 / 2 * if bit_depth == 8 { 1 } else { 2 }
            }
        };
        validate_conversion_budget(
            size,
            count * LINEAR_PIXEL_BYTES + std::mem::size_of::<LinearFrame>() as u64,
            bytes
                + std::mem::size_of::<Self>() as u64
                + MAX_PLANES * std::mem::size_of::<PixelPlane>() as u64,
            limits,
        )?;
        if Self::display_conversion_work(size, format, display.display) > limits.max_work {
            return Err(ValidationError::limit(
                "output",
                "max_work",
                "Output conversion passes exceed the work allowance",
            ));
        }
        let planes = match format {
            PixelFormat::SrgbRgba8 { premultiplied } => {
                let mut bytes = storage(bytes as usize, pool)?;
                if display.display.is_none() {
                    crate::cpu::output::RgbaEncoder::new(premultiplied).encode_bytes(
                        frame.pixels(),
                        size.width as usize,
                        &mut bytes,
                    );
                } else {
                    let mut cache = crate::display::EncodedCache::default();
                    for (row, pixels) in frame.pixels().chunks(size.width as usize).enumerate() {
                        for (column, pixel) in pixels.iter().enumerate() {
                            bytes.extend(crate::cpu::output::display_pixel(
                                *pixel,
                                premultiplied,
                                display,
                                column as u32,
                                row as u32,
                                &mut cache,
                            ));
                        }
                    }
                }
                vec![PixelPlane {
                    width: size.width,
                    height: size.height,
                    stride: size.width as usize * 4,
                    bytes,
                }]
            }
            PixelFormat::LinearRgba16Float => {
                let mut bytes = storage(bytes as usize, pool)?;
                for pixel in frame.pixels() {
                    let mut half = pixel.map(half::f16::from_f32);
                    if half.iter().any(|value| !value.is_finite()) {
                        return Err(output_invalid(
                            "half",
                            "Working pixel exceeds finite RGBA16F storage",
                        ));
                    }
                    if half[3] == half::f16::ZERO {
                        half.fill(half::f16::ZERO);
                    }
                    for value in half {
                        bytes.extend(value.to_bits().to_le_bytes());
                    }
                }
                vec![PixelPlane {
                    width: size.width,
                    height: size.height,
                    stride: size.width as usize * 8,
                    bytes,
                }]
            }
            PixelFormat::LinearRgba32Float => {
                let mut bytes = storage(bytes as usize, pool)?;
                for value in frame.pixels().iter().flatten() {
                    bytes.extend(value.to_le_bytes());
                }
                vec![PixelPlane {
                    width: size.width,
                    height: size.height,
                    stride: size.width as usize * 16,
                    bytes,
                }]
            }
            PixelFormat::Yuv420 { bit_depth } => {
                if frame.pixels().iter().any(|pixel| pixel[3] != 1.0) {
                    return Err(output_invalid(
                        "alpha",
                        "YUV420 cannot discard alpha; composite an explicit opaque background first",
                    ));
                }
                let depth = if bit_depth == 8 { 1 } else { 2 };
                let scale = if bit_depth == 8 { 1.0 } else { 4.0 };
                let mut luma = storage(count as usize * depth, pool)?;
                luma.resize(count as usize * depth, 0);
                let mut cb = storage(count as usize / 4 * depth, pool)?;
                let mut cr = storage(count as usize / 4 * depth, pool)?;
                let mut encoder = yuv::Yuv420BlockEncoder::new(scale);
                let push = |data: &mut Vec<u8>, sample: u16| {
                    if depth == 1 {
                        data.push(sample as u8);
                    } else {
                        data.extend(sample.to_le_bytes());
                    }
                };
                for row in (0..size.height).step_by(2) {
                    for column in (0..size.width).step_by(2) {
                        let first = row as usize * size.width as usize + column as usize;
                        let indices = [
                            first,
                            first + 1,
                            first + size.width as usize,
                            first + size.width as usize + 1,
                        ];
                        let samples = encoder.encode_display_block(
                            indices.map(|index| frame.pixels()[index]),
                            display,
                            column,
                            row,
                        );
                        for (index, sample) in indices.into_iter().zip(samples) {
                            let offset = index * depth;
                            if depth == 1 {
                                luma[offset] = sample as u8;
                            } else {
                                luma[offset..offset + 2].copy_from_slice(&sample.to_le_bytes());
                            }
                        }
                        push(&mut cb, samples[4]);
                        push(&mut cr, samples[5]);
                    }
                }
                vec![
                    PixelPlane {
                        width: size.width,
                        height: size.height,
                        stride: size.width as usize * depth,
                        bytes: luma,
                    },
                    PixelPlane {
                        width: size.width / 2,
                        height: size.height / 2,
                        stride: size.width as usize / 2 * depth,
                        bytes: cb,
                    },
                    PixelPlane {
                        width: size.width / 2,
                        height: size.height / 2,
                        stride: size.width as usize / 2 * depth,
                        bytes: cr,
                    },
                ]
            }
        };
        Ok(Self {
            size,
            format,
            planes,
            pool: Some(pool.clone()),
        })
    }

    /// Output dimensions.
    pub fn size(&self) -> PixelSize {
        self.size
    }

    pub(crate) fn conversion_work(size: PixelSize, format: PixelFormat) -> u64 {
        u64::from(size.width)
            * u64::from(size.height)
            * if matches!(format, PixelFormat::Yuv420 { .. }) {
                3
            } else {
                1
            }
    }

    pub(crate) fn display_conversion_work(
        size: PixelSize,
        format: PixelFormat,
        display: Option<&crate::DisplayTransform>,
    ) -> u64 {
        let extra = if matches!(
            format,
            PixelFormat::LinearRgba16Float | PixelFormat::LinearRgba32Float
        ) {
            0
        } else {
            display.map_or(0, crate::DisplayTransform::work_per_pixel)
        };
        Self::conversion_work(size, format)
            .saturating_add(u64::from(size.width) * u64::from(size.height) * extra)
    }

    pub(crate) fn retained_bytes(&self) -> u64 {
        self.byte_len() as u64
            + std::mem::size_of::<Self>() as u64
            + self.planes.len() as u64 * std::mem::size_of::<PixelPlane>() as u64
    }

    /// Storage/color/alpha format.
    pub fn format(&self) -> PixelFormat {
        self.format
    }

    /// One packed plane or three Y/U/V planes.
    pub fn planes(&self) -> &[PixelPlane] {
        &self.planes
    }

    /// Bytes of the first plane: the whole image for packed formats.
    pub fn bytes(&self) -> &[u8] {
        &self.planes[0].bytes
    }

    /// Logical payload bytes, excluding allocation overhead.
    pub fn byte_len(&self) -> usize {
        self.planes.iter().map(|plane| plane.bytes.len()).sum()
    }

    /// Atomically replaces a memory target on successful conversion.
    #[cfg(any(feature = "gpu", test))]
    pub(crate) fn write_linear(
        frame: &LinearFrame,
        target: RenderTarget<'_>,
        format: PixelFormat,
        limits: RasterLimits,
    ) -> Result<(), ValidationError> {
        match target {
            #[cfg(feature = "gpu")]
            RenderTarget::Texture(_) => Err(output_invalid(
                "target",
                "External textures require GpuContext::render; no implicit CPU upload",
            )),
            RenderTarget::Memory(target) => {
                let mut remaining = limits;
                remaining.max_bytes = remaining
                    .max_bytes
                    .checked_sub(target.retained_bytes())
                    .ok_or_else(|| {
                        output_invalid("max_bytes", "Existing target exceeds conversion allowance")
                    })?;
                let replacement = Self::from_linear(frame, format, remaining)?;
                *target = replacement;
                Ok(())
            }
        }
    }
}

impl PartialEq for FrameBuffer {
    fn eq(&self, other: &Self) -> bool {
        self.size == other.size && self.format == other.format && self.planes == other.planes
    }
}

impl Eq for FrameBuffer {}

impl Drop for FrameBuffer {
    fn drop(&mut self) {
        if let Some(pool) = &self.pool {
            for plane in &mut self.planes {
                pool.put(std::mem::take(&mut plane.bytes));
            }
        }
    }
}

/// A rendering destination. External GPU textures remain the owning backend gate.
pub enum RenderTarget<'a> {
    /// Replace only after a complete successful conversion.
    Memory(&'a mut FrameBuffer),
    /// External same-device color texture; use GpuContext::render for this target.
    #[cfg(feature = "gpu")]
    Texture(&'a wgpu::Texture),
}

fn storage(count: usize, pool: &OutputPool) -> Result<Vec<u8>, ValidationError> {
    let mut bytes = pool.take(count).unwrap_or_default();
    bytes.clear();
    bytes
        .try_reserve_exact(count)
        .map_err(|_| output_invalid("max_bytes", "Could not reserve output storage"))?;
    Ok(bytes)
}

fn output_invalid(field: &str, message: &str) -> ValidationError {
    ValidationError::invalid("output", field, message)
}

fn validate_conversion_budget(
    size: PixelSize,
    source: u64,
    output: u64,
    limits: RasterLimits,
) -> Result<(), ValidationError> {
    limits.validate_size(size)?;
    if source
        .checked_add(output)
        .is_none_or(|bytes| bytes > limits.max_bytes)
    {
        return Err(ValidationError::limit(
            "output",
            "max_bytes",
            "Source and output storage exceed the conversion allowance",
        ));
    }
    if u64::from(size.width) * u64::from(size.height) > limits.max_work {
        return Err(ValidationError::limit(
            "output",
            "max_work",
            "Output conversion exceeds the work allowance",
        ));
    }
    Ok(())
}
