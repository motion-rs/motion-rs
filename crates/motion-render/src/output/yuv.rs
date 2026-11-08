//! Exact 4:2:0 Y'CbCr block encoding with an approximate sRGB transfer guarded by a precise fallback.

const TRANSFER_SHIFT: u32 = 15;

const TRANSFER_FIRST: u32 = (0.0031308_f32.to_bits() + (1 << TRANSFER_SHIFT) - 1) >> TRANSFER_SHIFT;

const TRANSFER_LAST: u32 = 1.0_f32.to_bits() >> TRANSFER_SHIFT;

const TRANSFER_LENGTH: usize = (TRANSFER_LAST - TRANSFER_FIRST + 1) as usize;

const TRANSFER_ERROR: f64 = 1.0 / 1_048_576.0;

fn approximate_srgb(value: f32) -> f64 {
    let bits = value.to_bits();
    if !(TRANSFER_FIRST << TRANSFER_SHIFT..1.0_f32.to_bits()).contains(&bits) {
        return f64::from(motion_core::linear_to_srgb(value));
    }

    static TABLE: std::sync::OnceLock<[f32; TRANSFER_LENGTH]> = std::sync::OnceLock::new();
    let table = TABLE.get_or_init(|| {
        std::array::from_fn(|index| {
            motion_core::linear_to_srgb(f32::from_bits(
                (TRANSFER_FIRST + index as u32) << TRANSFER_SHIFT,
            ))
        })
    });
    let index = ((bits >> TRANSFER_SHIFT) - TRANSFER_FIRST) as usize;
    let fraction = f64::from(bits & ((1 << TRANSFER_SHIFT) - 1)) / f64::from(1 << TRANSFER_SHIFT);
    let start = f64::from(table[index]);
    start + (f64::from(table[index + 1]) - start) * fraction
}

fn ambiguous(sample: f64, error: f64) -> bool {
    (sample - sample.round()).abs() + error >= 0.5
}

pub(super) struct Yuv420BlockEncoder {
    scale: f64,
    quantization_error_bounds: [f64; 3],
    previous_rgb_bits: [u32; 3],
    cached_ycbcr: [f64; 3],
    cached_is_exact: bool,
    uniform_block_cache: Option<([u32; 3], [u16; 6])>,
    #[cfg(test)]
    exact_pixels: usize,
    #[cfg(test)]
    luma_fallbacks: usize,
    #[cfg(test)]
    chroma_fallbacks: usize,
}

impl Yuv420BlockEncoder {
    pub(super) fn new(scale: f64) -> Self {
        Self {
            scale,
            quantization_error_bounds: [219.0, 224.0 * 2.0 / 1.8556, 224.0 * 2.0 / 1.5748]
                .map(|weight| weight * scale * TRANSFER_ERROR + 1e-12),
            previous_rgb_bits: [u32::MAX; 3],
            cached_ycbcr: [0.0; 3],
            cached_is_exact: false,
            uniform_block_cache: None,
            #[cfg(test)]
            exact_pixels: 0,
            #[cfg(test)]
            luma_fallbacks: 0,
            #[cfg(test)]
            chroma_fallbacks: 0,
        }
    }

    pub(super) fn encode_display_block(
        &mut self,
        pixels: [[f32; 4]; 4],
        display: crate::display::DisplaySettings<'_>,
        column: u32,
        row: u32,
    ) -> [u16; 6] {
        if display.display.is_none() {
            return self.encode_block(pixels);
        }
        let mut samples = [0; 6];
        let mut chroma = [0.0; 2];
        let bayer = [[0, 8, 2, 10], [12, 4, 14, 6], [3, 11, 1, 9], [15, 7, 13, 5]];
        for (index, pixel) in pixels.into_iter().enumerate() {
            let x = column + index as u32 % 2;
            let y = row + index as u32 / 2;
            let rgb = display.encoded(pixel, x, y, true).map(f64::from);
            let luma = 0.2126 * rgb[0] + 0.7152 * rgb[1] + 0.0722 * rgb[2];
            let rank = bayer[((y + display.origin[1]) % 4) as usize]
                [((x + display.origin[0]) % 4) as usize];
            let dither = (f64::from(rank) + 0.5) / 16.0 - 0.5;
            samples[index] = ((16.0 + 219.0 * luma) * self.scale + dither).round() as u16;
            chroma[0] += (rgb[2] - luma) / 1.8556 * 0.25;
            chroma[1] += (rgb[0] - luma) / 1.5748 * 0.25;
        }
        samples[4] = ((128.0 + 224.0 * chroma[0]) * self.scale).round() as u16;
        samples[5] = ((128.0 + 224.0 * chroma[1]) * self.scale).round() as u16;
        samples
    }

    pub(super) fn encode_block(&mut self, pixels: [[f32; 4]; 4]) -> [u16; 6] {
        if pixels[1..].iter().all(|pixel| pixel[..3] == pixels[0][..3]) {
            let key = [pixels[0][0], pixels[0][1], pixels[0][2]].map(f32::to_bits);
            if let Some((previous, samples)) = self.uniform_block_cache
                && key == previous
            {
                return samples;
            }
            let color = self.encode_pixel(pixels[0], true);
            let mut samples = [((16.0 + 219.0 * color[0]) * self.scale).round() as u16; 6];
            let mut chroma = [0.0; 2];
            for _ in 0..4 {
                chroma[0] += color[1] * 0.25;
                chroma[1] += color[2] * 0.25;
            }
            samples[4] = ((128.0 + 224.0 * chroma[0]) * self.scale).round() as u16;
            samples[5] = ((128.0 + 224.0 * chroma[1]) * self.scale).round() as u16;
            self.uniform_block_cache = Some((key, samples));
            return samples;
        }
        let mut samples = [0; 6];
        let mut chroma = [0.0; 2];
        for (index, pixel) in pixels.iter().enumerate() {
            let mut color = self.encode_pixel(*pixel, false);
            let mut sample = (16.0 + 219.0 * color[0]) * self.scale;
            if ambiguous(sample, self.quantization_error_bounds[0]) {
                #[cfg(test)]
                {
                    self.luma_fallbacks += 1;
                }
                color = self.encode_pixel(*pixel, true);
                sample = (16.0 + 219.0 * color[0]) * self.scale;
            }
            samples[index] = sample.round() as u16;
            chroma[0] += color[1] * 0.25;
            chroma[1] += color[2] * 0.25;
        }
        let mut packed = chroma.map(|value| (128.0 + 224.0 * value) * self.scale);
        if ambiguous(packed[0], self.quantization_error_bounds[1])
            || ambiguous(packed[1], self.quantization_error_bounds[2])
        {
            #[cfg(test)]
            {
                self.chroma_fallbacks += 1;
            }
            chroma = [0.0; 2];
            for pixel in pixels {
                let color = self.encode_pixel(pixel, true);
                chroma[0] += color[1] * 0.25;
                chroma[1] += color[2] * 0.25;
            }
            packed = chroma.map(|value| (128.0 + 224.0 * value) * self.scale);
        }
        samples[4] = packed[0].round() as u16;
        samples[5] = packed[1].round() as u16;
        samples
    }

    fn encode_pixel(&mut self, pixel: [f32; 4], exact: bool) -> [f64; 3] {
        #[cfg(test)]
        if exact {
            self.exact_pixels += 1;
        }
        let key = [pixel[0], pixel[1], pixel[2]].map(f32::to_bits);
        if key == self.previous_rgb_bits && (!exact || self.cached_is_exact) {
            return self.cached_ycbcr;
        }
        let rgb = [pixel[0], pixel[1], pixel[2]].map(|value| {
            let value = value.clamp(0.0, 1.0);
            if exact {
                f64::from(motion_core::linear_to_srgb(value))
            } else {
                approximate_srgb(value)
            }
        });
        let luma = 0.2126 * rgb[0] + 0.7152 * rgb[1] + 0.0722 * rgb[2];
        self.cached_ycbcr = [luma, (rgb[2] - luma) / 1.8556, (rgb[0] - luma) / 1.5748];
        self.previous_rgb_bits = key;
        self.cached_is_exact = exact;
        self.cached_ycbcr
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn scalar_samples(pixels: [[f32; 4]; 4], scale: f64) -> [f64; 6] {
        let mut samples = [0.0; 6];
        let mut chroma = [0.0; 2];
        for (index, pixel) in pixels.into_iter().enumerate() {
            let rgb = [pixel[0], pixel[1], pixel[2]]
                .map(|value| f64::from(motion_core::linear_to_srgb(value.clamp(0.0, 1.0))));
            let luma = 0.2126 * rgb[0] + 0.7152 * rgb[1] + 0.0722 * rgb[2];
            samples[index] = (16.0 + 219.0 * luma) * scale;
            chroma[0] += ((rgb[2] - luma) / 1.8556) * 0.25;
            chroma[1] += ((rgb[0] - luma) / 1.5748) * 0.25;
        }
        samples[4] = (128.0 + 224.0 * chroma[0]) * scale;
        samples[5] = (128.0 + 224.0 * chroma[1]) * scale;
        samples
    }

    fn check_block(encoder: &mut Yuv420BlockEncoder, pixels: [[f32; 4]; 4]) {
        assert_eq!(
            encoder.encode_block(pixels),
            scalar_samples(pixels, encoder.scale).map(|sample| sample.round() as u16),
            "scale={}, pixels={pixels:?}",
            encoder.scale
        );
    }

    #[test]
    fn yuv_random_blocks_are_exact_and_over_99_percent_avoid_precise_fallback() {
        for scale in [1.0, 4.0] {
            let mut state = 0x1234_5678_u32;
            let mut encoder = Yuv420BlockEncoder::new(scale);
            let blocks = 65_536;
            for _ in 0..blocks {
                let pixels = std::array::from_fn(|_| {
                    let mut pixel = [0.0, 0.0, 0.0, 1.0];
                    for channel in &mut pixel[..3] {
                        state = state.wrapping_mul(1664525).wrapping_add(1013904223);
                        *channel = (state >> 8) as f32 / 16_777_215.0;
                    }
                    pixel
                });
                check_block(&mut encoder, pixels);
            }
            println!(
                "YUV scale={scale}: {} precise requests / {} pixels, {} luma and {} chroma-block fallbacks",
                encoder.exact_pixels,
                blocks * 4,
                encoder.luma_fallbacks,
                encoder.chroma_fallbacks
            );
            assert!(encoder.exact_pixels * 100 < blocks * 4);
        }
    }

    #[test]
    fn yuv_quantization_boundaries_and_mixed_chroma_are_exact() {
        for scale in [1.0, 4.0] {
            let mut encoder = Yuv420BlockEncoder::new(scale);
            for base in [0.0, 1.0] {
                for (sample, channel, mixed) in [
                    (0, None, false),
                    (3, Some(1), true),
                    (4, Some(2), false),
                    (5, Some(0), false),
                    (4, Some(2), true),
                    (5, Some(0), true),
                ] {
                    let pixels = |bits| {
                        let mut pixels = [[base, base, base, 1.0]; 4];
                        if mixed {
                            pixels = [
                                [0.25, 0.5, 0.75, 1.0],
                                [0.8, 0.15, 0.35, 1.0],
                                [0.0031308, 0.04, 0.18, 1.0],
                                [base, base, base, 1.0],
                            ];
                        }
                        for (index, pixel) in pixels.iter_mut().enumerate() {
                            if mixed && index != 3 {
                                continue;
                            }
                            if let Some(channel) = channel {
                                pixel[channel] = f32::from_bits(bits);
                            } else {
                                pixel[..3].fill(f32::from_bits(bits));
                            }
                        }
                        pixels
                    };
                    let start = scalar_samples(pixels(0), scale)[sample];
                    let end = scalar_samples(pixels(1.0_f32.to_bits()), scale)[sample];
                    for integer in start.floor() as u16..end.ceil() as u16 {
                        let threshold = f64::from(integer) + 0.5;
                        if threshold <= start || threshold >= end {
                            continue;
                        }
                        let mut low = 0;
                        let mut high = 1.0_f32.to_bits();
                        while low < high {
                            let middle = low + (high - low) / 2;
                            if scalar_samples(pixels(middle), scale)[sample] >= threshold {
                                high = middle;
                            } else {
                                low = middle + 1;
                            }
                        }
                        for offset in -8..=8 {
                            if let Some(bits) = low.checked_add_signed(offset) {
                                check_block(&mut encoder, pixels(bits));
                            }
                        }
                    }
                }
            }
            assert!(encoder.luma_fallbacks > 0);
            assert!(encoder.chroma_fallbacks > 0);
            for value in [
                -f32::MAX,
                -1.0,
                -0.0,
                0.0,
                f32::MIN_POSITIVE,
                f32::from_bits(1),
                0.0031308_f32.next_down(),
                0.0031308,
                0.0031308_f32.next_up(),
                1.0_f32.next_down(),
                1.0,
                4.0,
                f32::MAX,
            ] {
                check_block(
                    &mut encoder,
                    [
                        [value, 0.5, 0.75, 1.0],
                        [0.25, value, 0.75, 1.0],
                        [0.25, 0.5, value, 1.0],
                        [value, value, value, 1.0],
                    ],
                );
            }
        }
    }

    #[test]
    fn approximate_transfer_stays_inside_the_quantization_guard() {
        let mut largest = 0.0_f64;
        for index in 0..TRANSFER_LENGTH - 1 {
            let start = (TRANSFER_FIRST + index as u32) << TRANSFER_SHIFT;
            for offset in [0, 1, 4096, 8192, 16384, 24576, 32766, 32767] {
                let value = f32::from_bits(start + offset);
                let error =
                    (approximate_srgb(value) - f64::from(motion_core::linear_to_srgb(value))).abs();
                largest = largest.max(error);
                assert!(error <= TRANSFER_ERROR, "value={value}, error={error}");
            }
        }
        for value in [0.0, -0.0, 0.0031308, 1.0] {
            assert_eq!(
                approximate_srgb(value),
                f64::from(motion_core::linear_to_srgb(value))
            );
        }
        println!(
            "Transfer probe: {TRANSFER_LENGTH} entries, sampled maximum error {largest}, guard {TRANSFER_ERROR}"
        );
    }

    #[test]
    #[ignore = "Exhausts every interpolated f32 input; run with --release"]
    fn approximate_transfer_exhausts_its_f32_domain() {
        let start = TRANSFER_FIRST << TRANSFER_SHIFT;
        let end = 1.0_f32.to_bits();
        let mut largest = 0.0_f64;
        for bits in start..end {
            let value = f32::from_bits(bits);
            let error =
                (approximate_srgb(value) - f64::from(motion_core::linear_to_srgb(value))).abs();
            largest = largest.max(error);
            assert!(
                error <= TRANSFER_ERROR,
                "bits={bits:#x}, value={value}, error={error}"
            );
        }
        println!(
            "Exhausted {} interpolated f32 values: maximum error {largest}, guard {TRANSFER_ERROR}",
            end - start
        );
    }
}
