//! Immutable source-time transforms and clip convenience operations.

use super::pcm::{MAX_SAMPLES, PcmBuffer, invalid};
use motion_core::{AudioClip, AudioSource, Duration};

fn duration(frames: u64, rate: u32) -> Duration {
    Duration::from_ticks(
        (u128::from(frames) * motion_core::TICKS_PER_SECOND as u128).div_ceil(u128::from(rate))
            as u64,
    )
}

fn freeze(
    source: &dyn AudioSource,
    offset: Duration,
    length: Duration,
) -> motion_core::Result<PcmBuffer> {
    let channels = usize::from(source.channels());
    let rate = source.sample_rate();
    if !(1..=2).contains(&channels) || !(1..=192000).contains(&rate) {
        return Err(invalid("source_format", "Require bounded mono/stereo source").into());
    }
    let count = motion_core::audio::ceil_frames(length, rate);
    if count == 0 || count > (MAX_SAMPLES / channels) as u64 {
        return Err(invalid("source_bytes", "Transform snapshot exceeds 128 MiB").into());
    }
    let offset = motion_core::audio::frames(offset, rate);
    let mut values = vec![0.0; count as usize * channels];
    let mut start = 0;
    while start < count as usize {
        let end = (start + 4096).min(count as usize);
        let target = &mut values[start * channels..end * channels];
        let got = source.try_read(offset + start as u64, target)?;
        if got > end - start
            || target[..got * channels]
                .iter()
                .any(|value| !value.is_finite())
        {
            return Err(invalid("source", "Invalid transformed source read").into());
        }
        target[got * channels..].fill(0.0);
        if got == 0 {
            break;
        }
        start += got;
    }
    Ok(PcmBuffer::new(rate, channels as u16, values)?)
}

/// Frozen reverse-time source. Original file cursors are never retained.
#[derive(Clone, Debug)]
pub struct ReverseSource(PcmBuffer);

impl ReverseSource {
    /// Freezes and reverses a finite source by complete interleaved frames.
    pub fn new(source: &dyn AudioSource) -> motion_core::Result<Self> {
        let length = source
            .duration()
            .ok_or_else(|| invalid("reverse", "Reverse requires a finite source"))?;
        let pcm = freeze(source, Duration::ZERO, length)?;
        let channels = usize::from(pcm.channels());
        let values = pcm
            .samples()
            .chunks_exact(channels)
            .rev()
            .flat_map(|frame| frame.iter().copied())
            .collect();
        Ok(Self(PcmBuffer::new(
            pcm.sample_rate(),
            pcm.channels(),
            values,
        )?))
    }
}

/// Sinc-resampled time change: pitch and playback speed remain coupled.
#[derive(Clone, Debug)]
pub struct SpeedSource(PcmBuffer);

impl SpeedSource {
    /// Resamples a finite source at a speed in 0.25..=4, retaining its output rate.
    pub fn new(source: &dyn AudioSource, speed: f64) -> motion_core::Result<Self> {
        if !speed.is_finite() || !(0.25..=4.0).contains(&speed) {
            return Err(invalid("speed", "Require finite speed in 0.25..=4").into());
        }
        let pcm = freeze(
            source,
            Duration::ZERO,
            source
                .duration()
                .ok_or_else(|| invalid("speed", "Require finite source"))?,
        )?;
        let channels = usize::from(pcm.channels());
        let count = (pcm.frames() as f64 / speed).ceil() as usize;
        if count.saturating_mul(channels) > MAX_SAMPLES {
            return Err(invalid("speed_bytes", "Resampled source exceeds 128 MiB").into());
        }
        let mut values = vec![0.0; count * channels];
        let cutoff = (1.0 / speed).min(1.0);
        for frame in 0..count {
            let position = frame as f64 * speed;
            let center = position.floor() as i64;
            let mut sum = [0.0; 2];
            let mut total = 0.0;
            for index in -16..=16 {
                let offset = (center + index) as f64 - position;
                let angle = std::f64::consts::PI * offset * cutoff;
                let weight = (if angle.abs() < 1e-12 {
                    1.0
                } else {
                    libm::sin(angle) / angle
                }) * (0.5 + 0.5 * libm::cos(std::f64::consts::PI * offset / 17.0))
                    * cutoff;
                total += weight;
                if center + index >= 0 {
                    for (channel, value) in sum[..channels].iter_mut().enumerate() {
                        *value += f64::from(
                            pcm.samples()
                                .get((center + index) as usize * channels + channel)
                                .copied()
                                .unwrap_or(0.0),
                        ) * weight;
                    }
                }
            }
            for channel in 0..channels {
                values[frame * channels + channel] = (sum[channel] / total) as f32;
            }
        }
        Ok(Self(PcmBuffer::new(
            pcm.sample_rate(),
            pcm.channels(),
            values,
        )?))
    }
}

/// Finite loop with an equal-power overlapping seam and deterministic period.
#[derive(Clone, Debug)]
pub struct LoopSource {
    pcm: PcmBuffer,
    frames: u64,
    crossfade: usize,
}

impl LoopSource {
    /// Freezes a source interval; seam length must be less than half the interval.
    pub fn new(
        source: &dyn AudioSource,
        region: std::ops::Range<Duration>,
        crossfade: Duration,
        length: Duration,
    ) -> motion_core::Result<Self> {
        if region.start < Duration::ZERO
            || region.end <= region.start
            || crossfade < Duration::ZERO
            || crossfade.as_seconds() * 2.0 >= (region.end - region.start).as_seconds()
            || length <= Duration::ZERO
            || length.as_seconds() > 86400.0
        {
            return Err(invalid("loop", "Invalid source region, seam or output length").into());
        }
        let pcm = freeze(source, region.start, region.end - region.start)?;
        let frames = motion_core::audio::ceil_frames(length, pcm.sample_rate());
        let crossfade = motion_core::audio::frames(crossfade, pcm.sample_rate()) as usize;
        Ok(Self {
            pcm,
            frames,
            crossfade,
        })
    }
}

impl AudioSource for ReverseSource {
    fn sample_rate(&self) -> u32 {
        self.0.sample_rate()
    }

    fn channels(&self) -> u16 {
        self.0.channels()
    }

    fn duration(&self) -> Option<Duration> {
        self.0.duration()
    }

    fn read(&self, start: u64, output: &mut [f32]) -> usize {
        self.0.read(start, output)
    }
}

impl AudioSource for SpeedSource {
    fn sample_rate(&self) -> u32 {
        self.0.sample_rate()
    }

    fn channels(&self) -> u16 {
        self.0.channels()
    }

    fn duration(&self) -> Option<Duration> {
        self.0.duration()
    }

    fn read(&self, start: u64, output: &mut [f32]) -> usize {
        self.0.read(start, output)
    }
}

impl AudioSource for LoopSource {
    fn sample_rate(&self) -> u32 {
        self.pcm.sample_rate()
    }

    fn channels(&self) -> u16 {
        self.pcm.channels()
    }

    fn duration(&self) -> Option<Duration> {
        Some(duration(self.frames, self.sample_rate()))
    }

    fn read(&self, start: u64, output: &mut [f32]) -> usize {
        output.fill(0.0);
        let channels = usize::from(self.channels());
        let count = self
            .frames
            .saturating_sub(start)
            .min((output.len() / channels) as u64) as usize;
        let length = self.pcm.frames() as usize;
        let period = length - self.crossfade;
        for frame in 0..count {
            let local = ((start + frame as u64) % period as u64) as usize;
            for channel in 0..channels {
                let first = f64::from(self.pcm.samples()[local * channels + channel]);
                output[frame * channels + channel] = if local < self.crossfade {
                    let phase =
                        (local as f64 + 0.5) / self.crossfade as f64 * std::f64::consts::FRAC_PI_2;
                    let previous =
                        f64::from(self.pcm.samples()[(period + local) * channels + channel]);
                    (first * libm::sin(phase) + previous * libm::cos(phase)) as f32
                } else {
                    first as f32
                };
            }
        }
        count
    }
}

/// Eager source transformations for authored clips; file errors propagate here.
pub trait AudioClipExt: Sized {
    /// Changes speed and pitch together, scaling offset and fade timing.
    fn speed(self, factor: f64) -> motion_core::Result<Self>;

    /// Reverses the trimmed source interval before Clip effects.
    fn reverse(self) -> motion_core::Result<Self>;

    /// Changes pitch independently of duration using Sinc resampling and WSOLA.
    fn pitch_shift(self, semitones: f64) -> motion_core::Result<Self>;

    /// Repeats the trimmed interval through a finite output length.
    fn loop_until(self, length: Duration, crossfade: Duration) -> motion_core::Result<Self>;

    /// Places the next clip with a bounded linear-amplitude overlap.
    fn crossfade(self, next: AudioClip, length: Duration) -> motion_core::Result<[AudioClip; 2]>;
}

fn clip_pcm(clip: &AudioClip) -> motion_core::Result<PcmBuffer> {
    let clip = super::resolve_clip(clip)?;
    let parameters = clip.parameters();
    let tail = parameters
        .effects
        .iter()
        .fold(Duration::ZERO, |sum, effect| sum + effect.tail());
    freeze(
        parameters.source,
        parameters.offset,
        clip.resolved_duration()? - tail,
    )
}

impl AudioClipExt for AudioClip {
    fn speed(self, factor: f64) -> motion_core::Result<Self> {
        let source = SpeedSource::new(&clip_pcm(&self)?, factor)?;
        let length = source.duration().unwrap_or(Duration::ZERO);
        let parameters = self.parameters();
        let fade_in =
            Duration::from_ticks((parameters.fade_in.ticks() as f64 / factor).round() as u64);
        let fade_out =
            Duration::from_ticks((parameters.fade_out.ticks() as f64 / factor).round() as u64);
        Ok(self
            .with_resolved_source(std::sync::Arc::new(source))
            .offset(Duration::ZERO)
            .duration(length)
            .fade_in(fade_in)
            .fade_out(fade_out))
    }

    fn reverse(self) -> motion_core::Result<Self> {
        let source = ReverseSource::new(&clip_pcm(&self)?)?;
        let length = source.duration().unwrap_or(Duration::ZERO);
        Ok(self
            .with_resolved_source(std::sync::Arc::new(source))
            .offset(Duration::ZERO)
            .duration(length))
    }

    fn pitch_shift(self, semitones: f64) -> motion_core::Result<Self> {
        let pcm = clip_pcm(&self)?;
        let shifted = pcm.pitch_shift(semitones)?;
        let length = shifted.duration().unwrap_or(Duration::ZERO);
        Ok(self
            .with_resolved_source(std::sync::Arc::new(shifted))
            .offset(Duration::ZERO)
            .duration(length))
    }

    fn loop_until(self, length: Duration, crossfade: Duration) -> motion_core::Result<Self> {
        let pcm = clip_pcm(&self)?;
        let source = LoopSource::new(
            &pcm,
            Duration::ZERO..pcm.duration().unwrap_or(Duration::ZERO),
            crossfade,
            length,
        )?;
        Ok(self
            .with_resolved_source(std::sync::Arc::new(source))
            .offset(Duration::ZERO)
            .duration(length))
    }

    fn crossfade(self, next: AudioClip, length: Duration) -> motion_core::Result<[AudioClip; 2]> {
        if length <= Duration::ZERO
            || length > self.resolved_duration()?
            || length > next.resolved_duration()?
        {
            return Err(invalid("crossfade", "Overlap must fit both clips").into());
        }
        let start = self.end()? - length;
        Ok([self.fade_out(length), next.start(start).fade_in(length)])
    }
}

impl PcmBuffer {
    /// Bounded WSOLA time stretching; pitch is preserved approximately, rate is unchanged.
    pub fn time_stretch(&self, factor: f64) -> motion_core::Result<Self> {
        if !factor.is_finite() || !(0.25..=4.0).contains(&factor) {
            return Err(invalid("time_stretch", "Require finite factor in 0.25..=4").into());
        }
        if factor == 1.0 {
            return Ok(self.clone());
        }
        let output = (self.frames() as f64 * factor).round().max(1.0) as usize;
        stretch(self, output)
    }

    /// Changes pitch by -12..=12 semitones, preserving the original frame count.
    pub fn pitch_shift(&self, semitones: f64) -> motion_core::Result<Self> {
        if !semitones.is_finite() || !(-12.0..=12.0).contains(&semitones) {
            return Err(invalid("pitch_shift", "Require finite -12..=12 semitones").into());
        }
        if semitones == 0.0 {
            return Ok(self.clone());
        }
        let speed = libm::pow(2.0, semitones / 12.0);
        let resampled = SpeedSource::new(self, speed)?;
        stretch(&resampled.0, self.frames() as usize)
    }
}

fn stretch(pcm: &PcmBuffer, count: usize) -> motion_core::Result<PcmBuffer> {
    let channels = usize::from(pcm.channels());
    if count.saturating_mul(channels) > MAX_SAMPLES {
        return Err(invalid("stretch_bytes", "Duration stretch exceeds 128 MiB").into());
    }
    let window = ((pcm.sample_rate() / 50) as usize)
        .next_power_of_two()
        .clamp(128, 2048);
    let hop = window / 4;
    let candidates = hop.min(64);
    let steps = count.div_ceil(hop);
    if steps
        .saturating_mul(window)
        .saturating_mul(candidates * 2 + 1)
        > 128_000_000
    {
        return Err(invalid("stretch_work", "WSOLA exceeds correlation work allowance").into());
    }
    let mut output = vec![0.0_f64; count * channels];
    let mut weights = vec![0.0_f64; count];
    let envelope: Vec<_> = (0..window)
        .map(|index| {
            0.5 - 0.5 * libm::cos(std::f64::consts::TAU * (index as f64 + 0.5) / window as f64)
        })
        .collect();
    for destination in (0..count).step_by(hop) {
        let nominal = (destination as f64 * pcm.frames() as f64 / count as f64).round() as usize;
        let mut selected = nominal;
        let mut best = f64::NEG_INFINITY;
        if destination > 0 {
            for candidate in nominal.saturating_sub(candidates)
                ..=(nominal + candidates).min(pcm.frames() as usize - 1)
            {
                let mut product = 0.0;
                let mut source_energy = 0.0;
                let mut output_energy = 0.0;
                for index in 0..hop {
                    let target = destination + index;
                    if target >= count || weights[target] == 0.0 {
                        break;
                    }
                    for channel in 0..channels {
                        let first = output[target * channels + channel] / weights[target];
                        let second = f64::from(
                            pcm.samples()
                                .get((candidate + index) * channels + channel)
                                .copied()
                                .unwrap_or(0.0),
                        );
                        product += first * second;
                        source_energy += second * second;
                        output_energy += first * first;
                    }
                }
                let score = product / libm::sqrt((source_energy * output_energy).max(1e-30))
                    - (candidate.abs_diff(nominal) as f64) * 1e-8;
                if score > best {
                    best = score;
                    selected = candidate;
                }
            }
        }
        for (index, weight) in envelope.iter().enumerate() {
            let target = destination + index;
            if target >= count {
                break;
            }
            weights[target] += weight;
            for channel in 0..channels {
                output[target * channels + channel] += f64::from(
                    pcm.samples()
                        .get((selected + index) * channels + channel)
                        .copied()
                        .unwrap_or(0.0),
                ) * weight;
            }
        }
    }
    let samples = output
        .into_iter()
        .enumerate()
        .map(|(index, value)| (value / weights[index / channels].max(1e-30)) as f32)
        .collect();
    Ok(PcmBuffer::new(pcm.sample_rate(), pcm.channels(), samples)?)
}

#[cfg(test)]
mod tests {
    use super::*;
    use motion_core::TimeLiteral;

    #[test]
    fn source_transforms_preserve_stereo_and_seek_contracts() -> motion_core::Result {
        let pcm = PcmBuffer::new(1000, 2, vec![0.1, 0.2, 0.3, 0.4, 0.5, 0.6, 0.7, 0.8])?;
        let reverse = ReverseSource::new(&pcm)?;
        let mut reversed = [0.0; 8];
        reverse.read(0, &mut reversed);
        assert_eq!(reversed, [0.7, 0.8, 0.5, 0.6, 0.3, 0.4, 0.1, 0.2]);
        let looped = LoopSource::new(
            &pcm,
            Duration::ZERO..4.milliseconds(),
            Duration::ZERO,
            20.milliseconds(),
        )?;
        let mut whole = [0.0; 40];
        looped.read(0, &mut whole);
        let mut window = [0.0; 8];
        looped.read(7, &mut window);
        assert_eq!(window, whole[14..22]);
        let speed = SpeedSource::new(&pcm, 2.0)?;
        assert_eq!(speed.duration(), Some(2.milliseconds()));
        assert!(SpeedSource::new(&pcm, f64::NAN).is_err());
        let clips =
            AudioClip::source(pcm.clone()).crossfade(AudioClip::source(pcm), 1.milliseconds())?;
        assert_eq!(clips[1].parameters().start, 3.milliseconds().after_origin());
        Ok(())
    }

    #[test]
    #[cfg(feature = "audio")]
    fn independent_pitch_and_tempo_keep_frame_and_replay_contracts() -> motion_core::Result {
        let signal = (0..8000)
            .map(|index| {
                (libm::sin(std::f64::consts::TAU * 400.0 * index as f64 / 8000.0) * 0.5) as f32
            })
            .collect();
        let pcm = PcmBuffer::new(8000, 1, signal)?;
        assert_eq!(pcm.pitch_shift(0.0)?.samples(), pcm.samples());
        let shifted = pcm.pitch_shift(12.0)?;
        assert_eq!(shifted.frames(), pcm.frames());
        let analysis =
            super::super::analysis::AudioAnalysis::from_pcm(std::sync::Arc::new(shifted.clone()));
        assert!(
            (motion_core::Signal::sample(&analysis.pitch()?, 500.milliseconds().after_origin())
                - 800.0)
                .abs()
                < 25.0
        );
        assert_eq!(pcm.time_stretch(2.0)?.frames(), 16000);
        let mut whole = vec![0.0; 8000];
        shifted.read(0, &mut whole);
        let mut window = [0.0; 37];
        shifted.read(731, &mut window);
        assert_eq!(window, whole[731..768]);
        Ok(())
    }
}
