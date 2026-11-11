//! Prepared stereo timeline mixing with query-order-independent reads.

use super::meter::BusMeter;
#[cfg(feature = "audio")]
use super::resample::resample;
use super::{
    AudioClip, AudioSource, MAX_SAMPLES, MixOptions, MixSource, ResampleQuality, ceil_frames,
    frames, invalid,
};
#[cfg(not(feature = "audio"))]
use crate::audio::resample_disabled::resample;
use motion_core::Duration;
use std::sync::Arc;

#[derive(Clone, Debug)]
pub(super) struct PreparedClip {
    pub(super) start: u64,
    pub(super) data: Arc<[f32]>,
    pub(super) bus: Option<String>,
}

/// Immutable prepared stereo mix with bounded offline bus-meter traces.
#[derive(Clone, Debug)]
pub struct MixedAudio {
    pub(super) rate: u32,
    pub(super) quality: ResampleQuality,
    pub(super) frames: u64,
    pub(super) clips: Arc<[PreparedClip]>,
    pub(super) meters: Arc<[BusMeter]>,
    pub(in crate::audio) levels: Arc<std::sync::OnceLock<crate::audio::master::LoudnessReport>>,
}

impl MixedAudio {
    /// Mixes the source in deterministic occurrence order.
    ///
    /// A [`MixSource::Timeline`] resolves its audio files here, applies buses and the master
    /// and honours [`MixOptions::range`] while preserving original fade/envelope clocks.
    /// Clips are prepared at most [`MAX_CLIPS`](crate::audio::MAX_CLIPS) / 128 MiB stereo PCM.
    /// Timeline gaps and EOF are silence. All sources/envelopes must be pure; callback panics
    /// propagate.
    pub fn new<'a>(
        source: impl Into<MixSource<'a>>,
        options: MixOptions,
    ) -> motion_core::Result<Self> {
        let MixOptions {
            sample_rate,
            range,
            resampler,
        } = options;
        match source.into() {
            MixSource::Timeline(timeline) => {
                let resolved = crate::audio::prepare_timeline(timeline)?;
                Self::from_resolved(timeline, &resolved, sample_rate, range, resampler)
            }
            MixSource::Resolved { timeline, audio } => {
                Self::from_resolved(timeline, audio, sample_rate, range, resampler)
            }
            MixSource::Clips { clips, duration } => match range {
                None => Self::new_window(clips, sample_rate, duration, Duration::ZERO, resampler),
                Some(range) => {
                    let range = checked_window(range)?;
                    Self::new_window(
                        clips,
                        sample_rate,
                        range.end - range.start,
                        range.start,
                        resampler,
                    )
                }
            },
        }
    }

    fn from_resolved(
        timeline: &motion_core::Timeline,
        resolved: &motion_core::ResolvedAudio,
        rate: u32,
        range: Option<std::ops::Range<Duration>>,
        quality: ResampleQuality,
    ) -> motion_core::Result<Self> {
        let Some(range) = range else {
            return Self::from_resolved_full(timeline, resolved, rate, quality);
        };
        let range = checked_window(range)?;
        let clips = resolved.clips();
        if timeline.soundtrack().master() != motion_core::Master::default()
            || !timeline.soundtrack().buses().is_empty()
            || clips.iter().any(|clip| {
                !clip.parameters().effects.is_empty() || !clip.parameters().ducking.is_empty()
            })
        {
            return Self::from_resolved_full(timeline, resolved, rate, quality)?.crop(range);
        }
        Self::new_window(clips, rate, range.end - range.start, range.start, quality)
    }

    fn from_resolved_full(
        timeline: &motion_core::Timeline,
        resolved: &motion_core::ResolvedAudio,
        rate: u32,
        quality: ResampleQuality,
    ) -> motion_core::Result<Self> {
        Self::new_window(
            resolved.clips(),
            rate,
            resolved.duration(),
            Duration::ZERO,
            quality,
        )?
        .process_buses(timeline.soundtrack().buses())?
        .apply_master(timeline.soundtrack().master())
    }

    pub(super) fn new_window(
        clips: &[AudioClip],
        rate: u32,
        duration: Duration,
        window: Duration,
        quality: ResampleQuality,
    ) -> motion_core::Result<Self> {
        let (window_time, mix_duration) = (window, duration);
        let resolved = clips
            .iter()
            .map(crate::audio::resolve_clip)
            .collect::<motion_core::Result<Vec<_>>>()?;
        let clips = resolved.as_slice();
        if clips.len() > crate::audio::MAX_CLIPS
            || !(1..=192_000).contains(&rate)
            || duration < Duration::ZERO
            || duration.as_seconds() > 86400.0
        {
            return Err(invalid(
                "mix",
                "Require <=256 clips, 1..=192000 Hz and duration in 0..=24h",
            )
            .into());
        }
        let total = ceil_frames(duration, rate)?;
        let mut prepared = Vec::new();
        let mut retained = 0usize;
        let mut peak_bound = 0.0f64;
        for (index, clip) in clips.iter().enumerate() {
            let duration = clip.resolved_duration().map_err(|mut error| {
                error.path = format!("audio.clips[{index}]");
                error
            })?;
            let clip = clip.parameters();
            let origin = frames(clip.start.since_origin(), rate)?;
            let window = frames(window, rate)?;
            let skipped = window.saturating_sub(origin);
            let start = origin.saturating_sub(window);
            if start >= total {
                continue;
            }
            let count = ceil_frames(
                clip.limit.map_or(duration, |limit| limit.min(duration)),
                rate,
            )?
            .saturating_sub(skipped)
            .min(total - start);
            if count > (MAX_SAMPLES / 2) as u64 || retained + count as usize * 2 > MAX_SAMPLES {
                return Err(invalid("mix_bytes", "Prepared mix exceeds 128 MiB").into());
            }
            let channels = usize::from(clip.source.channels());
            let source_rate = clip.source.sample_rate();
            let input_frames = usize::try_from(
                (u128::from(count) * u128::from(source_rate)).div_ceil(u128::from(rate)),
            )
            .map_err(|_| invalid("source_frames", "Source frame count exceeds address space"))?;
            if input_frames.saturating_mul(channels) > MAX_SAMPLES {
                return Err(invalid("source_bytes", "Resampling input exceeds 128 MiB").into());
            }
            let mut input = vec![0.0f32; input_frames * channels];
            let mut read = 0usize;
            let tail = clip
                .effects
                .iter()
                .fold(Duration::ZERO, |sum, effect| sum + effect.tail());
            let dry_frames = ceil_frames(duration - tail, source_rate)?;
            let offset = frames(clip.offset, source_rate)?
                .checked_add(
                    (u128::from(skipped) * u128::from(source_rate) / u128::from(rate)) as u64,
                )
                .ok_or_else(|| invalid("offset", "Audio source offset overflow"))?;
            while read < input_frames {
                let end = (read + 4096).min(input_frames);
                let end = end.min(dry_frames.saturating_sub(
                    (u128::from(skipped) * u128::from(source_rate) / u128::from(rate)) as u64,
                ) as usize);
                if end <= read {
                    break;
                }
                let target = &mut input[read * channels..end * channels];
                let got = clip.source.try_read(offset + read as u64, target)?;
                if got > end - read
                    || target[..got * channels]
                        .iter()
                        .any(|value| !value.is_finite())
                {
                    return Err(invalid(
                        "source",
                        "AudioSource returned an invalid count or nonfinite PCM",
                    )
                    .into());
                }
                target[got * channels..].fill(0.0);
                if got == 0 {
                    break;
                }
                read += got;
            }
            for effect in clip.effects {
                effect.process(&mut input, channels as u16, source_rate)?;
            }
            let values = match quality {
                ResampleQuality::Sinc => crate::audio::sinc::resample(
                    &input,
                    channels,
                    source_rate,
                    rate,
                    count as usize,
                )?,
                ResampleQuality::Fast => {
                    resample(&input, channels, source_rate, rate, count as usize)?
                }
            };
            let fade_in = frames(clip.fade_in, rate)?;
            let fade_out = frames(clip.fade_out, rate)?;
            let length = ceil_frames(duration, rate)?;
            let mut data = Vec::with_capacity(count as usize * 2);
            for frame in 0..count as usize {
                let local_frame = skipped + frame as u64;
                let local = Duration::from_ticks(
                    (u128::from(local_frame) * motion_core::TICKS_PER_SECOND as u128)
                        .div_ceil(u128::from(rate)) as i64,
                );
                let envelope = clip
                    .envelope
                    .as_ref()
                    .map_or(1.0, |envelope| envelope.sample(local));
                if !envelope.is_finite() || !(0.0..=16.0).contains(&envelope) {
                    return Err(
                        invalid("envelope", "Volume envelope must be finite in 0..=16").into(),
                    );
                }
                let gain = clip.volume
                    * envelope
                    * if fade_in == 0 {
                        1.0
                    } else {
                        (local_frame as f64 / fade_in as f64).min(1.0)
                    }
                    * if fade_out == 0 {
                        1.0
                    } else {
                        ((length.saturating_sub(local_frame)) as f64 / fade_out as f64).min(1.0)
                    };
                let pan = clip
                    .pan_animation
                    .map_or(clip.pan, |animation| animation.sample(local));
                if !pan.is_finite() || !(-1.0..=1.0).contains(&pan) {
                    return Err(invalid("pan_animation", "Pan must be finite in -1..=1").into());
                }
                let (left, right) = if channels == 1 {
                    let angle = (pan + 1.0) * std::f64::consts::FRAC_PI_4;
                    (
                        f64::from(values[frame]) * if pan == 1.0 { 0.0 } else { libm::cos(angle) },
                        f64::from(values[frame]) * if pan == -1.0 { 0.0 } else { libm::sin(angle) },
                    )
                } else {
                    let mut left = f64::from(values[frame * 2]);
                    let mut right = f64::from(values[frame * 2 + 1]);
                    if clip.stereo_width != 1.0 {
                        let middle = (left + right) * 0.5;
                        let side = (left - right) * 0.5 * clip.stereo_width;
                        left = middle + side;
                        right = middle - side;
                    }
                    let balance = match clip.pan_law {
                        motion_core::PanLaw::Linear => (1.0 - pan.max(0.0), 1.0 + pan.min(0.0)),
                        law => {
                            let angle = (pan + 1.0) * std::f64::consts::FRAC_PI_4;
                            let power = if law == motion_core::PanLaw::Minus4_5dB {
                                1.5
                            } else {
                                1.0
                            };
                            (
                                libm::pow(
                                    libm::cos(angle).max(0.0) * std::f64::consts::SQRT_2,
                                    power,
                                ),
                                libm::pow(
                                    libm::sin(angle).max(0.0) * std::f64::consts::SQRT_2,
                                    power,
                                ),
                            )
                        }
                    };
                    (left * balance.0, right * balance.1)
                };
                for value in [left * gain, right * gain] {
                    let value = value as f32;
                    if !value.is_finite() {
                        return Err(invalid("gain", "Audio gain exceeds finite PCM storage").into());
                    }
                    data.push(value);
                }
            }
            for ducking in clip.ducking {
                let motion_core::DuckTrigger::Clip(trigger) = &ducking.trigger else {
                    return Err(invalid("ducking", "Clip ducking requires a clip trigger").into());
                };
                let reduction = Self::new_window(
                    std::slice::from_ref(trigger.as_ref()),
                    rate,
                    mix_duration,
                    window_time,
                    quality,
                )?
                .snapshot()?;
                let reduction =
                    crate::audio::ducking::reduction_db(&reduction, rate, &ducking.duck);
                crate::audio::ducking::apply(&mut data, start as usize, &reduction);
            }
            peak_bound += data
                .iter()
                .map(|value| f64::from(value.abs()))
                .fold(0.0, f64::max);
            if peak_bound > f64::from(f32::MAX) {
                return Err(
                    invalid("mix_peak", "Possible mix sum exceeds finite PCM storage").into(),
                );
            }
            retained += data.len();
            prepared.push(PreparedClip {
                start,
                data: data.into(),
                bus: clip.bus.map(str::to_owned),
            });
        }
        Ok(Self {
            rate,
            quality,
            frames: total,
            clips: prepared.into(),
            meters: Arc::new([]),
            levels: Arc::new(std::sync::OnceLock::new()),
        })
    }

    pub(super) fn duration_time(&self) -> Duration {
        Duration::from_ticks(
            (u128::from(self.frames) * motion_core::TICKS_PER_SECOND as u128)
                .div_ceil(u128::from(self.rate)) as i64,
        )
    }

    /// Fixed output frames, including empty timeline intervals.
    pub fn frames(&self) -> u64 {
        self.frames
    }

    /// Bus levels measured after bus effects/gain and before the master.
    pub fn bus_meters(&self) -> &[BusMeter] {
        &self.meters
    }

    /// Applies a static master gain without changing seek order or summation order.
    pub fn apply_gain(self, gain: f64) -> motion_core::Result<Self> {
        self.apply_master(motion_core::Master {
            gain_db: gain,
            ..Default::default()
        })
    }

    /// Bakes the master once, preserving exact query-order-independent reads.
    pub fn apply_master(mut self, master: motion_core::Master) -> motion_core::Result<Self> {
        master.validate()?;
        if master == motion_core::Master::default() {
            return Ok(self);
        }
        let mut samples = self.snapshot()?;
        crate::audio::master::process(&mut samples, self.rate, master)?;
        self.levels = Arc::new(std::sync::OnceLock::new());
        self.clips = vec![PreparedClip {
            start: 0,
            data: samples.into(),
            bus: None,
        }]
        .into();
        Ok(self)
    }

    pub(crate) fn snapshot(&self) -> motion_core::Result<Vec<f32>> {
        if self.frames > (MAX_SAMPLES / 2) as u64 {
            return Err(invalid("mix_bytes", "Prepared master exceeds 128 MiB").into());
        }
        let mut samples = vec![0.0; self.frames as usize * 2];
        self.read(0, &mut samples);
        Ok(samples)
    }

    /// Crops a prepared output without restarting effects or normalization clocks.
    pub fn crop(mut self, range: std::ops::Range<Duration>) -> motion_core::Result<Self> {
        if range.start < Duration::ZERO || range.end < range.start {
            return Err(invalid("range", "Invalid audio window").into());
        }
        let start = frames(range.start, self.rate)?;
        let count = ceil_frames(range.end - range.start, self.rate)?;
        if count > (MAX_SAMPLES / 2) as u64 {
            return Err(invalid("mix_bytes", "Audio window exceeds 128 MiB").into());
        }
        let mut data = vec![0.0; count as usize * 2];
        self.read(start, &mut data);
        self.frames = count;
        self.levels = Arc::new(std::sync::OnceLock::new());
        self.clips = vec![PreparedClip {
            start: 0,
            data: data.into(),
            bus: None,
        }]
        .into();
        self.meters = self
            .meters
            .iter()
            .cloned()
            .map(|mut meter| {
                meter.start = meter.start.saturating_add(start);
                meter.frames = count;
                meter
            })
            .collect::<Vec<_>>()
            .into();
        Ok(self)
    }

    /// Retained prepared sample bytes; source/decoder scratch is excluded.
    pub fn retained_bytes(&self) -> usize {
        self.clips.iter().map(|clip| clip.data.len() * 4).sum()
    }

    /// Streams an IEEE-float stereo WAV, cropped/padded to frames. This writes to
    /// the supplied writer only; export owns temporary-file transactions.
    pub fn write_wav(&self, output: &mut impl std::io::Write, frames: u64) -> motion_core::Result {
        let bytes = frames
            .checked_mul(8)
            .filter(|bytes| *bytes <= u64::from(u32::MAX) - 36)
            .ok_or_else(|| invalid("wav", "PCM exceeds RIFF32 size"))? as u32;
        output.write_all(b"RIFF")?;
        output.write_all(&(bytes + 36).to_le_bytes())?;
        output.write_all(b"WAVEfmt ")?;
        output.write_all(&16u32.to_le_bytes())?;
        output.write_all(&3u16.to_le_bytes())?;
        output.write_all(&2u16.to_le_bytes())?;
        output.write_all(&self.rate.to_le_bytes())?;
        output.write_all(&(self.rate * 8).to_le_bytes())?;
        output.write_all(&8u16.to_le_bytes())?;
        output.write_all(&32u16.to_le_bytes())?;
        output.write_all(b"data")?;
        output.write_all(&bytes.to_le_bytes())?;
        let mut buffer = vec![0.0; 8192];
        let mut start = 0;
        while start < frames {
            let count = (frames - start).min(4096) as usize;
            self.read(start, &mut buffer[..count * 2]);
            for value in &buffer[..count * 2] {
                output.write_all(&value.to_le_bytes())?;
            }
            start += count as u64;
        }
        Ok(())
    }
}

impl AudioSource for MixedAudio {
    fn sample_rate(&self) -> u32 {
        self.rate
    }

    fn channels(&self) -> u16 {
        2
    }

    fn duration(&self) -> Option<Duration> {
        Some(Duration::from_ticks(
            (u128::from(self.frames) * motion_core::TICKS_PER_SECOND as u128)
                .div_ceil(u128::from(self.rate)) as i64,
        ))
    }

    fn read(&self, start: u64, output: &mut [f32]) -> usize {
        output.fill(0.0);
        let count = self
            .frames
            .saturating_sub(start)
            .min((output.len() / 2) as u64) as usize;
        for frame in 0..count {
            let mut sum = [0.0f64; 2];
            let position = start + frame as u64;
            for clip in self.clips.iter().filter(|clip| {
                position >= clip.start && position - clip.start < (clip.data.len() / 2) as u64
            }) {
                let index = (position - clip.start) as usize * 2;
                sum[0] += f64::from(clip.data[index]);
                sum[1] += f64::from(clip.data[index + 1]);
            }
            output[frame * 2] = sum[0] as f32;
            output[frame * 2 + 1] = sum[1] as f32;
        }
        count
    }
}

fn checked_window(
    range: std::ops::Range<Duration>,
) -> motion_core::Result<std::ops::Range<Duration>> {
    if range.start < Duration::ZERO || range.end < range.start {
        return Err(invalid("range", "Invalid audio window").into());
    }
    Ok(range)
}
