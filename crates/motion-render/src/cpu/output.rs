//! Explicit SDR output conversion of immutable linear-premultiplied frames.

use super::LinearFrame;
use motion_core::{PixelSize, RasterLimits, ValidationError, budget::Budget};

#[cfg(test)]
mod tests;

const BAYER_4X4: [[u8; 4]; 4] = [[0, 8, 2, 10], [12, 4, 14, 6], [3, 11, 1, 9], [15, 7, 13, 5]];

/// Float bit patterns per lookup bucket, as a power of two. Neighboring byte thresholds are at
/// least a few buckets apart, so a bucket nearly always holds at most one of them.
const BUCKET_SHIFT: u32 = 15;

/// Quantizes linear alpha without dither; `present.wgsl` uses the same rounding on the GPU.
fn alpha_to_byte(alpha: f32) -> u8 {
    (f64::from(alpha) * 255.0).round() as u8
}

fn dither_offset(rank: u8) -> f32 {
    (f32::from(rank) + 0.5) / 16.0 - 0.5
}

fn srgb_thresholds() -> &'static [[u32; 255]; 16] {
    static TABLE: std::sync::OnceLock<[[u32; 255]; 16]> = std::sync::OnceLock::new();
    TABLE.get_or_init(|| {
        std::array::from_fn(|rank| {
            let offset = dither_offset(rank as u8);
            std::array::from_fn(|index| {
                let mut low = 0;
                let mut high = 1.0_f32.to_bits();
                while low < high {
                    let middle = low + (high - low) / 2;
                    let byte = (motion_core::linear_to_srgb(f32::from_bits(middle)) * 255.0
                        + offset)
                        .round() as u8;
                    if byte >= (index + 1) as u8 {
                        high = middle;
                    } else {
                        low = middle + 1;
                    }
                }
                low
            })
        })
    })
}

/// The dithered sRGB byte thresholds with an index over their float bit patterns.
///
/// Finding a byte is "count the thresholds not above the value". A binary search does that in
/// eight dependent loads, and dithered noise makes each comparison unpredictable. The index
/// jumps to the bucket the value falls in; its entry is the number of thresholds that lie
/// below the whole bucket, so only the few inside it remain to be checked.
struct SrgbLevels {
    /// Ascending per rank, followed by `u32::MAX` so a probe at the end of the list is valid.
    thresholds: [[u32; 256]; 16],
    /// Smallest threshold of any rank; smaller values stay at byte zero.
    floor: u32,
    /// `buckets[rank][k]` counts thresholds below bit pattern `floor + (k << BUCKET_SHIFT)`.
    buckets: [Vec<u8>; 16],
}

fn srgb_levels() -> &'static SrgbLevels {
    static LEVELS: std::sync::OnceLock<SrgbLevels> = std::sync::OnceLock::new();
    LEVELS.get_or_init(|| {
        let exact = srgb_thresholds();
        let thresholds = std::array::from_fn(|rank| {
            std::array::from_fn(|index| exact[rank].get(index).copied().unwrap_or(u32::MAX))
        });
        let floor = exact.iter().map(|ranks| ranks[0]).min().unwrap_or(0);
        let top = 1.0_f32.to_bits();
        let buckets = std::array::from_fn(|rank| {
            let mut count = 0;
            (0..=(top.saturating_sub(floor) >> BUCKET_SHIFT))
                .map(|bucket| {
                    let start = floor + (bucket << BUCKET_SHIFT);
                    while count < exact[rank].len() && exact[rank][count] < start {
                        count += 1;
                    }
                    count as u8
                })
                .collect()
        });
        SrgbLevels {
            thresholds,
            floor,
            buckets,
        }
    })
}

impl SrgbLevels {
    /// Dithered sRGB byte for a straight linear channel, equal to counting `thresholds[rank]`
    /// entries not above the value (zero at or below 0.0, 255 from 1.0 up and for NaN).
    #[inline(always)]
    fn byte(&self, rank: usize, value: f32) -> u8 {
        if value <= 0.0 {
            return 0;
        }
        if value >= 1.0 || value.is_nan() {
            return 255;
        }
        let bits = value.to_bits();
        if bits < self.floor {
            return 0;
        }
        let thresholds = &self.thresholds[rank];
        let mut count =
            usize::from(self.buckets[rank][((bits - self.floor) >> BUCKET_SHIFT) as usize]);
        // One step covers the usual single threshold inside the bucket without a branch; the
        // loop only runs for denser buckets and stops at the `u32::MAX` sentinel.
        count += usize::from(thresholds[count] <= bits);
        while thresholds[count] <= bits {
            count += 1;
        }
        count as u8
    }
}

/// The binary-search definition that `SrgbLevels::byte` must reproduce.
#[cfg(test)]
fn threshold_byte(value: f32, thresholds: &[u32; 255]) -> u8 {
    if value <= 0.0 {
        0
    } else if value >= 1.0 {
        255
    } else {
        thresholds.partition_point(|threshold| *threshold <= value.to_bits()) as u8
    }
}

/// Logical output conversion totals, independent of rendering work.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct OutputStats {
    /// Peak source frame, output frame and pixel payload bytes; not process RSS.
    /// Excludes the shared immutable 16,320-byte sRGB threshold table.
    pub peak_bytes: u64,
    /// Pixels converted, including transparent pixels.
    pub work: u64,
}

/// Read-only, tightly packed row-major sRGB RGBA8 output with linear alpha.
///
/// RGB is SDR-clipped and ordered-dithered. Premultiplication, when requested,
/// is in encoded sRGB, not linear light. This buffer does not encode a file.
#[derive(Clone, Debug, PartialEq)]
pub struct Rgba8Frame {
    size: PixelSize,
    pixels: Vec<[u8; 4]>,
    premultiplied: bool,
    stats: OutputStats,
}

pub(crate) struct RgbaEncoder {
    premultiplied: bool,
    previous: [u32; 4],
    cached: [[u8; 4]; 16],
    valid: u16,
}

pub(crate) fn display_pixel(
    source: [f32; 4],
    premultiplied: bool,
    display: crate::display::DisplaySettings<'_>,
    column: u32,
    row: u32,
    cache: &mut crate::display::EncodedCache,
) -> [u8; 4] {
    let alpha = alpha_to_byte(source[3]);
    if alpha == 0 {
        return [0; 4];
    }
    let rank = BAYER_4X4[((row + display.origin[1]) % 4) as usize]
        [((column + display.origin[0]) % 4) as usize];
    let rgb = display
        .encoded_cached(source, column, row, false, cache)
        .map(|value| {
            let encoded = if premultiplied {
                value * source[3]
            } else {
                value
            };
            let byte = (encoded * 255.0 + dither_offset(rank)).round() as u8;
            if premultiplied { byte.min(alpha) } else { byte }
        });
    [rgb[0], rgb[1], rgb[2], alpha]
}

impl RgbaEncoder {
    pub(crate) fn new(premultiplied: bool) -> Self {
        Self {
            premultiplied,
            previous: [u32::MAX; 4],
            cached: [[0; 4]; 16],
            valid: 0,
        }
    }

    pub(crate) fn encode(
        &mut self,
        source: &[[f32; 4]],
        width: usize,
        mut write: impl FnMut([u8; 4]),
    ) {
        for (row, pixels) in source.chunks(width).enumerate() {
            for (column, pixel) in pixels.iter().enumerate() {
                write(self.pixel(*pixel, column, row));
            }
        }
    }

    /// Appends the encoded bytes of `source`. The space is filled in place; pushing four bytes
    /// per pixel instead pays a capacity check each time.
    pub(crate) fn encode_bytes(&mut self, source: &[[f32; 4]], width: usize, bytes: &mut Vec<u8>) {
        let start = bytes.len();
        bytes.resize(start + source.len() * 4, 0);
        let lines = bytes[start..].chunks_mut(width * 4);
        for (row, (pixels, line)) in source.chunks(width).zip(lines).enumerate() {
            let targets = line.as_chunks_mut::<4>().0;
            for (column, (pixel, target)) in pixels.iter().zip(targets).enumerate() {
                *target = self.pixel(*pixel, column, row);
            }
        }
    }

    #[inline(always)]
    pub(crate) fn pixel(&mut self, source: [f32; 4], column: usize, row: usize) -> [u8; 4] {
        let rank = BAYER_4X4[row % 4][column % 4] as usize;
        let key = [
            source[0].to_bits(),
            source[1].to_bits(),
            source[2].to_bits(),
            source[3].to_bits(),
        ];
        if key == self.previous && self.valid & (1 << rank) != 0 {
            return self.cached[rank];
        }
        self.uncached_pixel(source, key, rank)
    }

    fn uncached_pixel(&mut self, source: [f32; 4], key: [u32; 4], rank: usize) -> [u8; 4] {
        let alpha = alpha_to_byte(source[3]);
        if alpha == 0 {
            return [0; 4];
        }
        if key != self.previous {
            self.previous = key;
            self.valid = 0;
        }
        let opaque = source[3] == 1.0;
        if opaque {
            // Alpha one needs neither a division nor premultiplication, and the byte table
            // applies in both alpha modes.
            let levels = srgb_levels();
            let byte = |channel: f32| levels.byte(rank, f64::from(channel).clamp(0.0, 1.0) as f32);
            let pixel = [byte(source[0]), byte(source[1]), byte(source[2]), alpha];
            self.cached[rank] = pixel;
            self.valid |= 1 << rank;
            return pixel;
        }
        let levels = if !self.premultiplied {
            Some(srgb_levels())
        } else {
            None
        };
        let offset = dither_offset(rank as u8);
        let rgb = [source[0], source[1], source[2]].map(|channel| {
            let straight = if opaque {
                f64::from(channel).clamp(0.0, 1.0)
            } else {
                (f64::from(channel) / f64::from(source[3])).clamp(0.0, 1.0)
            };
            if let Some(levels) = levels {
                return levels.byte(rank, straight as f32);
            }
            let encoded = motion_core::linear_to_srgb(straight as f32);
            let encoded = if self.premultiplied {
                encoded * source[3]
            } else {
                encoded
            };
            let byte = (encoded * 255.0 + offset).round() as u8;
            if self.premultiplied {
                byte.min(alpha)
            } else {
                byte
            }
        });
        let pixel = [rgb[0], rgb[1], rgb[2], alpha];
        self.cached[rank] = pixel;
        self.valid |= 1 << rank;
        pixel
    }
}

impl Rgba8Frame {
    /// Output dimensions.
    pub fn size(&self) -> PixelSize {
        self.size
    }

    /// Row-major encoded RGBA pixels.
    pub fn pixels(&self) -> &[[u8; 4]] {
        &self.pixels
    }

    /// Tightly packed RGBA bytes without row padding or another allocation.
    pub fn bytes(&self) -> &[u8] {
        self.pixels.as_flattened()
    }

    /// One pixel, or None outside the half-open output bounds.
    pub fn pixel(&self, x: u32, y: u32) -> Option<[u8; 4]> {
        if x >= self.size.width || y >= self.size.height {
            return None;
        }
        Some(self.pixels[y as usize * self.size.width as usize + x as usize])
    }

    /// Whether encoded RGB is premultiplied by alpha.
    pub fn premultiplied(&self) -> bool {
        self.premultiplied
    }

    /// Conversion work and peak logical source/output storage.
    pub fn stats(&self) -> OutputStats {
        self.stats
    }

    /// Consumes opaque RGBA8 into the shared tightly packed RGB Snapshot.
    ///
    /// Rejects any nonopaque pixel rather than discarding alpha or compositing
    /// in encoded space. Encoded premultiplication is immaterial at alpha one.
    /// Limits admit both source and destination storage and one work unit per
    /// pixel before allocation. No file is written and no encoder is invoked.
    pub fn into_snapshot(self, limits: RasterLimits) -> Result<crate::Snapshot, ValidationError> {
        self.into_snapshot_with_stats(limits).map(|result| result.0)
    }

    pub(crate) fn into_snapshot_with_stats(
        self,
        limits: RasterLimits,
    ) -> Result<(crate::Snapshot, OutputStats), ValidationError> {
        Self::snapshot_from_bytes(
            self.size,
            self.bytes(),
            size_of::<Rgba8Frame>() as u64 + self.pixels.len() as u64 * 4,
            limits,
        )
    }

    pub(crate) fn snapshot_from_bytes(
        size: PixelSize,
        source: &[u8],
        source_bytes: u64,
        limits: RasterLimits,
    ) -> Result<(crate::Snapshot, OutputStats), ValidationError> {
        let mut budget = Budget::new(size, limits)?;
        if source.len() as u64 != u64::from(size.width) * u64::from(size.height) * 4 {
            return Err(ValidationError::invalid(
                "snapshot.pixels",
                "size",
                "Snapshot source must contain one RGBA8 pixel per output pixel",
            ));
        }
        let count = source.len() as u64 / 4;
        budget.bytes(source_bytes, "snapshot.source")?;
        budget.bytes(
            size_of::<crate::Snapshot>() as u64 + count * size_of::<u32>() as u64,
            "snapshot.output",
        )?;
        budget.work(count, "snapshot.pixels")?;
        let mut pixels = Vec::new();
        pixels.try_reserve_exact(count as usize).map_err(|_| {
            ValidationError::limit(
                "snapshot.pixels",
                "max_bytes",
                "Could not reserve snapshot storage",
            )
        })?;
        let rgba = source.as_chunks::<4>().0;
        if rgba.iter().any(|pixel| pixel[3] != 255) {
            return Err(ValidationError::invalid(
                "snapshot.pixels",
                "alpha",
                "Snapshot requires opaque RGBA8; composite a background in linear light first",
            ));
        }
        pixels.extend(rgba.iter().map(|pixel| u32::from_be_bytes(*pixel) >> 8));
        Ok((
            crate::Snapshot::from_pixels(size.width as usize, size.height as usize, pixels),
            OutputStats {
                peak_bytes: budget.peak_bytes(),
                work: budget.work_done(),
            },
        ))
    }
}

impl LinearFrame {
    /// Converts to SDR sRGB RGBA8, with straight or encoded-premultiplied RGB.
    ///
    /// Unpremultiplication uses f64 before clipping straight linear RGB to 0..=1
    /// and applying the sRGB transfer function. There is no HDR tone mapping.
    /// RGB uses the fixed 4x4 Bayer matrix, anchored at output (0, 0), with
    /// offsets `(rank + 0.5) / 16 - 0.5` in byte units; all channels share the
    /// offset. Alpha is rounded to nearest without dithering. Alpha rounding
    /// to zero clears all RGB; premultiplied output channels never exceed alpha.
    ///
    /// Independent limits include the source/output frame values and both pixel
    /// payloads, excluding clones, PreparedFrame, excess capacity and allocator
    /// overhead. One work unit per pixel is admitted before allocation. Failure
    /// leaves the linear source unchanged, without returning a partial output.
    ///
    /// ```
    /// use motion_render::prelude::*;
    /// use motion_core::prelude::*;
    /// use motion_core::{
    ///     DisplayList, RasterLimits, ResourcePool, ValidatedDisplayList, MAX_PATH_VERBS,
    ///     MAX_STROKE_VERBS, MAX_SVG_PATH_BYTES, sample_offsets,
    /// };
    /// use motion_render::{Composition, RenderPrepare, RenderOptions, Renderer, render_still};
    /// let node =
    ///     Node::from(Rect::new(0.0, 0.0, 2.0, 1.0).fill(Color::from_hex(0xff0000))).opacity(0.5);
    /// let prepared = node
    ///     .compile(
    ///         Size::new(2.0, 1.0),
    ///         PixelSize::new(2, 1),
    ///         ValidationLimits::default(),
    ///     )?
    ///     .prepare(ValidationLimits::default(), RasterLimits::default())?;
    /// let linear = prepared.render_linear(RasterLimits::default())?;
    /// let rgba = linear.to_srgb_rgba8(false, RasterLimits::default())?;
    /// assert_eq!(rgba.pixel(0, 0), Some([255, 0, 0, 128]));
    /// assert_eq!(rgba.bytes(), &[255, 0, 0, 128, 255, 0, 0, 128]);
    /// # Ok::<(), motion_core::ValidationError>(())
    /// ```
    pub fn to_srgb_rgba8(
        &self,
        premultiplied: bool,
        limits: RasterLimits,
    ) -> Result<Rgba8Frame, ValidationError> {
        let mut budget = Budget::new(self.size, limits)?;
        let count = self.pixels.len() as u64;
        budget.bytes(size_of::<LinearFrame>() as u64, "rgba8.source")?;
        budget.bytes(
            count
                .checked_mul(size_of::<[f32; 4]>() as u64)
                .ok_or_else(|| {
                    ValidationError::limit(
                        "rgba8.source",
                        "max_bytes",
                        "Linear pixel storage size overflow",
                    )
                })?,
            "rgba8.source",
        )?;
        budget.bytes(size_of::<Rgba8Frame>() as u64, "rgba8.frame")?;
        budget.bytes(
            count
                .checked_mul(size_of::<[u8; 4]>() as u64)
                .ok_or_else(|| {
                    ValidationError::limit(
                        "rgba8.pixels",
                        "max_bytes",
                        "RGBA8 pixel storage size overflow",
                    )
                })?,
            "rgba8.pixels",
        )?;
        budget.work(count, "rgba8.pixels")?;
        let mut pixels = Vec::new();
        pixels.try_reserve_exact(self.pixels.len()).map_err(|_| {
            ValidationError::limit(
                "rgba8.pixels",
                "max_bytes",
                "Could not reserve RGBA8 storage",
            )
        })?;
        RgbaEncoder::new(premultiplied).encode(&self.pixels, self.size.width as usize, |pixel| {
            pixels.push(pixel);
        });
        Ok(Rgba8Frame {
            size: self.size,
            pixels,
            premultiplied,
            stats: OutputStats {
                peak_bytes: budget.peak_bytes(),
                work: budget.work_done(),
            },
        })
    }
}
