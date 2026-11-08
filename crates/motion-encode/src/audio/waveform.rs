//! Bounded multi-resolution peak envelopes for long timeline views.

use super::pcm::invalid;
use motion_core::{AudioSource, Duration};

/// Peak envelope pyramid; every parent covers two adjacent children.
#[derive(Clone, Debug)]
pub struct WaveformPyramid {
    levels: Vec<Vec<f32>>,
    frames: u64,
    base: u64,
}

impl WaveformPyramid {
    /// Streams a finite interval in bounded blocks; at most one million base bins.
    pub fn from_source(source: &dyn AudioSource, duration: Duration) -> motion_core::Result<Self> {
        let channels = usize::from(source.channels());
        let rate = source.sample_rate();
        if !(1..=2).contains(&channels)
            || !(1..=192000).contains(&rate)
            || duration.as_seconds() > 86400.0
        {
            return Err(invalid("waveform", "Invalid waveform format or duration").into());
        }
        let frames = motion_core::audio::ceil_frames(duration, rate)?;
        let base = frames.div_ceil(1_000_000).max(64);
        let bins = frames.div_ceil(base) as usize;
        let mut peaks = vec![0.0_f32; bins];
        let mut block = vec![0.0; 4096 * channels];
        let mut start = 0;
        while start < frames {
            let count = (frames - start).min(4096) as usize;
            let target = &mut block[..count * channels];
            target.fill(0.0);
            let got = source.try_read(start, target)?;
            if got > count
                || target[..got * channels]
                    .iter()
                    .any(|sample| !sample.is_finite())
            {
                return Err(invalid("waveform_source", "Invalid source count or samples").into());
            }
            for frame in 0..got {
                let bin = ((start + frame as u64) / base) as usize;
                let peak = target[frame * channels..frame * channels + channels]
                    .iter()
                    .map(|sample| sample.abs())
                    .fold(0.0, f32::max);
                peaks[bin] = peaks[bin].max(peak);
            }
            start += count as u64;
        }
        let mut levels = vec![peaks];
        while levels.last().is_some_and(|level| level.len() > 1) {
            let next = levels
                .last()
                .unwrap()
                .chunks(2)
                .map(|pair| pair.iter().copied().fold(0.0, f32::max))
                .collect();
            levels.push(next);
        }
        Ok(Self {
            levels,
            frames,
            base,
        })
    }

    /// Resamples peak bins to a fixed display width without discarding short transients.
    pub fn peaks(&self, pixels: usize) -> motion_core::Result<Vec<f32>> {
        if pixels == 0 || pixels > 16384 {
            return Err(invalid("waveform_pixels", "Require 1..=16384 display bins").into());
        }
        let stride = self.frames.div_ceil(pixels as u64).max(1);
        let mut level = 0;
        while level + 1 < self.levels.len() && self.base * (1_u64 << (level + 1)) <= stride {
            level += 1;
        }
        let bins = &self.levels[level];
        Ok((0..pixels)
            .map(|index| {
                let first = index * bins.len() / pixels;
                let end = ((index + 1) * bins.len())
                    .div_ceil(pixels)
                    .max(first + 1)
                    .min(bins.len());
                bins.get(first..end)
                    .unwrap_or(&[])
                    .iter()
                    .copied()
                    .fold(0.0, f32::max)
            })
            .collect())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use motion_core::TimeLiteral;

    #[test]
    fn long_waveforms_keep_transients_and_bounded_levels() -> motion_core::Result {
        let mut samples = vec![0.0; 400000];
        samples[12345] = 0.8;
        let pcm = crate::PcmBuffer::new(1000, 1, samples)?;
        let pyramid = WaveformPyramid::from_source(&pcm, 400.seconds())?;
        for width in [1, 64, 1024] {
            let peaks = pyramid.peaks(width)?;
            assert_eq!(peaks.len(), width);
            assert!(peaks.contains(&0.8));
        }
        assert!(pyramid.peaks(0).is_err());
        Ok(())
    }
}
