//! Derived audio analysis: signal envelopes and normalization, spectrum and chroma bands,
//! pitch, onset detection, marker files and beat grids built on top of the offline traces.

use super::*;
use motion_core::sync::MutexExt;

impl AudioSignal {
    /// Borrows offline values in hop order, without sampling or source reads.
    pub fn values(&self) -> &[f64] {
        &self.0.values
    }

    fn mapped(&self, transform: impl Fn(f64) -> f64) -> Self {
        Self(Arc::new(AnalysisTrace {
            values: self
                .0
                .values
                .iter()
                .map(|value| transform(*value))
                .collect(),
            sample_rate: self.0.sample_rate,
            hop_frames: self.0.hop_frames,
            duration: self.0.duration,
        }))
    }

    /// Smooths the complete trace offline, preserving seek-independent output.
    pub fn attack_release(&self, attack: Duration, release: Duration) -> motion_core::Result<Self> {
        if attack.as_seconds() > 30.0 || release.as_seconds() > 30.0 {
            return Err(invalid(
                "signal_smoothing",
                "Require smoothing times in 0..=30 seconds",
            )
            .into());
        }
        let seconds = self.0.hop_frames as f64 / f64::from(self.0.sample_rate);
        let coefficient = |duration: Duration| {
            if duration == Duration::ZERO {
                0.0
            } else {
                libm::exp(-seconds / duration.as_seconds())
            }
        };
        let attack = coefficient(attack);
        let release = coefficient(release);
        let mut previous = 0.0;
        let values = self
            .0
            .values
            .iter()
            .map(|value| {
                let coefficient = if *value > previous { attack } else { release };
                previous = coefficient * previous + (1.0 - coefficient) * value;
                previous
            })
            .collect();
        Ok(Self(Arc::new(AnalysisTrace {
            values,
            sample_rate: self.0.sample_rate,
            hop_frames: self.0.hop_frames,
            duration: self.0.duration,
        })))
    }

    /// Maps nonnegative amplitude logarithmically into `[0, 1]`, with a -60 dB floor.
    pub fn log_scale(&self) -> Self {
        self.mapped(|value| ((20.0 * libm::log10(value.max(0.001)) + 60.0) / 60.0).clamp(0.0, 1.0))
    }

    /// Scales by a robust amplitude percentile, clamping overshoots to one.
    pub fn normalize(&self, percentile: f64) -> motion_core::Result<Self> {
        if !percentile.is_finite() || !(0.0..=1.0).contains(&percentile) {
            return Err(invalid("signal_percentile", "Require percentile in 0..=1").into());
        }
        let mut ordered = self.0.values.clone();
        ordered.sort_by(f64::total_cmp);
        let pivot = ordered
            .get(((ordered.len().saturating_sub(1)) as f64 * percentile).round() as usize)
            .copied()
            .unwrap_or(0.0);
        Ok(self.mapped(|value| {
            if pivot > 0.0 {
                (value / pivot).clamp(0.0, 1.0)
            } else {
                0.0
            }
        }))
    }

    /// Produces a binary event envelope at a finite amplitude threshold.
    pub fn threshold(&self, level: f64) -> motion_core::Result<Self> {
        if !level.is_finite() || level < 0.0 {
            return Err(invalid("signal_threshold", "Require a finite nonnegative level").into());
        }
        Ok(self.mapped(|value| if value >= level { 1.0 } else { 0.0 }))
    }
}

/// One-pass logarithmic or chromatic band analysis.
#[derive(Clone, Debug)]
pub struct AudioSpectrum {
    bands: Vec<AudioSignal>,
}

impl AudioSpectrum {
    /// Band animations, ordered from low to high frequency or C through B.
    pub fn bands(&self) -> &[AudioSignal] {
        &self.bands
    }
}

/// Stable, ascending offline onset timestamps.
#[derive(Clone, Debug, Default)]
pub struct AudioMarkers {
    times: Vec<Duration>,
}

impl AudioMarkers {
    /// Timestamp-ordered events suitable for particle bursts and Studio cues.
    pub fn times(&self) -> &[Duration] {
        &self.times
    }

    /// Creates ordered markers; duplicate, negative and beyond-24h times are rejected.
    pub fn new(times: Vec<Duration>) -> motion_core::Result<Self> {
        if times.len() > 1_000_000
            || times.iter().any(|time| time.as_seconds() > 86400.0)
            || times.windows(2).any(|pair| pair[0] >= pair[1])
        {
            return Err(invalid(
                "markers",
                "Require at most one million strictly ordered times in 0..=24h",
            )
            .into());
        }
        Ok(Self { times })
    }

    /// Writes a versioned tick-based JSON sidecar without float timestamp loss.
    pub fn write_json(&self, output: &mut impl std::io::Write) -> motion_core::Result {
        let value = serde_json::json!({"version":1,"ticks_per_second":motion_core::TICKS_PER_SECOND,"times":self.times.iter().map(|time|time.ticks()).collect::<Vec<_>>()});
        serde_json::to_writer(output, &value).map_err(std::io::Error::other)?;
        Ok(())
    }

    /// Parses a bounded sidecar and validates its clock, version and marker order.
    pub fn read_json(input: &mut impl std::io::Read) -> motion_core::Result<Self> {
        use std::io::Read;
        let mut bytes = Vec::new();
        input.take(16 * 1024 * 1024 + 1).read_to_end(&mut bytes)?;
        if bytes.len() > 16 * 1024 * 1024 {
            return Err(invalid("markers_json", "Marker sidecar exceeds 16 MiB").into());
        }
        let value: serde_json::Value =
            serde_json::from_slice(&bytes).map_err(std::io::Error::other)?;
        if value.get("version").and_then(serde_json::Value::as_u64) != Some(1)
            || value
                .get("ticks_per_second")
                .and_then(serde_json::Value::as_i64)
                != Some(motion_core::TICKS_PER_SECOND)
        {
            return Err(invalid("markers_json", "Unknown marker sidecar version or clock").into());
        }
        let times = value
            .get("times")
            .and_then(serde_json::Value::as_array)
            .ok_or_else(|| invalid("markers_json", "Missing marker times"))?
            .iter()
            .map(|value| {
                value
                    .as_u64()
                    .map(Duration::from_ticks)
                    .ok_or_else(|| invalid("markers_json", "Require integer ticks"))
            })
            .collect::<Result<Vec<_>, _>>()?;
        Self::new(times)
    }
}

/// A constant-tempo grid with confidence and a manually overridable origin.
#[derive(Clone, Copy, Debug)]
pub struct BeatGrid {
    /// Beats per minute.
    pub bpm: f64,
    /// First beat position.
    pub offset: Duration,
    /// Fraction of onset intervals agreeing with the estimated period.
    pub confidence: f64,
}

impl BeatGrid {
    /// Creates an explicit grid; automatic estimates may be replaced by this.
    pub fn new(bpm: f64, offset: Duration) -> motion_core::Result<Self> {
        if !bpm.is_finite() || !(20.0..=400.0).contains(&bpm) {
            return Err(invalid("beat_grid", "Invalid BPM").into());
        }
        Ok(Self {
            bpm,
            offset,
            confidence: 1.0,
        })
    }

    /// Returns bounded beat markers through a positive duration.
    pub fn markers(&self, duration: Duration) -> motion_core::Result<AudioMarkers> {
        Self::new(self.bpm, self.offset)?;
        if duration.as_seconds() > 86400.0 {
            return Err(invalid("beat_duration", "Require duration in 0..=24h").into());
        }
        let count =
            (duration.saturating_sub(self.offset).as_seconds() * self.bpm / 60.0).ceil() as usize;
        if count > 1_000_000 {
            return Err(invalid("beat_count", "Too many beat markers").into());
        }
        Ok(AudioMarkers {
            times: (0..count)
                .map(|index| {
                    Duration::from_ticks(
                        self.offset.ticks()
                            + (index as f64 * 60.0 / self.bpm
                                * motion_core::TICKS_PER_SECOND as f64)
                                .round() as u64,
                    )
                })
                .filter(|time| *time < duration)
                .collect(),
        })
    }
}

impl AudioAnalysis {
    fn cached(
        &self,
        key: Key,
        bands: usize,
        build: impl FnOnce() -> motion_core::Result<AudioSpectrum>,
    ) -> motion_core::Result<AudioSpectrum> {
        let window = ((self.pcm.sample_rate() / 25) as usize)
            .next_power_of_two()
            .clamp(64, 8192);
        let bytes = (self.pcm.frames() as usize)
            .div_ceil(window / 4)
            .saturating_mul(bands)
            .saturating_mul(8);
        if bytes > 64 * 1024 * 1024 {
            return Err(invalid("analysis_bytes", "Analysis group exceeds 64 MiB").into());
        }
        let mut cache = self.pcm.analysis.lock_unpoisoned();
        if let Some(traces) = cache.get(&key) {
            return Ok(AudioSpectrum {
                bands: traces.iter().cloned().map(AudioSignal).collect(),
            });
        }
        let result = build()?;
        let retained: usize = cache
            .values()
            .flatten()
            .map(|trace| trace.values.len() * 8)
            .sum();
        if cache.len() >= 16 || retained.saturating_add(bytes) > 64 * 1024 * 1024 {
            cache.clear();
        }
        cache.insert(
            key,
            result.bands.iter().map(|signal| signal.0.clone()).collect(),
        );
        Ok(result)
    }

    fn stft(
        &self,
        mut consume: impl FnMut(usize, &[f64], u32, usize),
    ) -> motion_core::Result<(usize, usize)> {
        let rate = self.pcm.sample_rate();
        let channels = usize::from(self.pcm.channels());
        let window = ((rate / 25) as usize).next_power_of_two().clamp(64, 8192);
        let hop = window / 4;
        let count = (self.pcm.frames() as usize).div_ceil(hop);
        if count > 1_000_000 || count.saturating_mul(window) > 128_000_000 {
            return Err(invalid("analysis_work", "STFT exceeds work allowance").into());
        }
        let weights: Vec<_> = (0..window)
            .map(|index| {
                0.5 - 0.5 * libm::cos(std::f64::consts::TAU * index as f64 / window as f64)
            })
            .collect();
        let power: f64 = weights.iter().map(|value| value * value).sum();
        let fft = FftPlanner::<f64>::new().plan_fft_forward(window);
        let mut buffer = vec![Complex::new(0.0, 0.0); window];
        let mut energy = vec![0.0; window / 2 + 1];
        for start in (0..self.pcm.frames() as usize).step_by(hop) {
            energy.fill(0.0);
            for channel in 0..channels {
                for (index, value) in buffer.iter_mut().enumerate() {
                    *value = Complex::new(
                        f64::from(
                            self.pcm
                                .samples()
                                .get((start + index) * channels + channel)
                                .copied()
                                .unwrap_or(0.0),
                        ) * weights[index],
                        0.0,
                    );
                }
                fft.process(&mut buffer);
                for (index, value) in energy.iter_mut().enumerate() {
                    *value += buffer[index].norm_sqr()
                        * (if index == 0 || index == window / 2 {
                            1.0
                        } else {
                            2.0
                        })
                        / (window as f64 * power * channels as f64);
                }
            }
            consume(start, &energy, rate, window);
        }
        Ok((window, hop))
    }

    fn signal(&self, values: Vec<f64>, hop: usize) -> AudioSignal {
        AudioSignal(Arc::new(AnalysisTrace {
            values,
            sample_rate: self.pcm.sample_rate(),
            hop_frames: hop,
            duration: self.pcm.duration().unwrap_or(Duration::ZERO),
        }))
    }

    /// Computes 1..=64 logarithmic RMS bands in one Hann-window FFT pass.
    pub fn spectrum(
        &self,
        bands: usize,
        low: f64,
        high: f64,
    ) -> motion_core::Result<AudioSpectrum> {
        if !(1..=64).contains(&bands)
            || !low.is_finite()
            || !high.is_finite()
            || low <= 0.0
            || high <= low
            || high > f64::from(self.pcm.sample_rate()) * 0.5
        {
            return Err(invalid("spectrum", "Require 1..=64 bands and 0<low<high<=Nyquist").into());
        }
        let window = ((self.pcm.sample_rate() / 25) as usize)
            .next_power_of_two()
            .clamp(64, 8192);
        if (self.pcm.frames() as usize)
            .div_ceil(window / 4)
            .saturating_mul(bands)
            .saturating_mul(8)
            > 64 * 1024 * 1024
        {
            return Err(invalid("spectrum_bytes", "Spectrum exceeds 64 MiB").into());
        }
        self.cached(
            Key::Spectrum(bands, low.to_bits(), high.to_bits(), window, window / 4),
            bands,
            || {
                let edges: Vec<_> = (0..=bands)
                    .map(|index| low * libm::pow(high / low, index as f64 / bands as f64))
                    .collect();
                let mut values = vec![Vec::new(); bands];
                let (_, hop) = self.stft(|_, energy, rate, window| {
                    let mut sums = vec![0.0; bands];
                    for (index, power) in energy.iter().enumerate() {
                        let frequency = index as f64 * f64::from(rate) / window as f64;
                        if frequency >= low && frequency < high {
                            let band = edges
                                .partition_point(|edge| *edge <= frequency)
                                .saturating_sub(1)
                                .min(bands - 1);
                            sums[band] += power;
                        }
                    }
                    for (trace, sum) in values.iter_mut().zip(sums) {
                        trace.push(libm::sqrt(sum));
                    }
                })?;
                Ok(AudioSpectrum {
                    bands: values
                        .into_iter()
                        .map(|values| self.signal(values, hop))
                        .collect(),
                })
            },
        )
    }

    /// RMS chromatic energy ordered C,C#,D,...,B. Not a polyphonic transcription.
    pub fn chroma(&self) -> motion_core::Result<AudioSpectrum> {
        self.cached(Key::Chroma, 12, || {
            let mut values = vec![Vec::new(); 12];
            let (_, hop) = self.stft(|_, energy, rate, window| {
                let mut sums = [0.0; 12];
                for (index, power) in energy.iter().enumerate().skip(1) {
                    let frequency = index as f64 * f64::from(rate) / window as f64;
                    if frequency >= 20.0 {
                        let note = (69.0 + 12.0 * libm::log2(frequency / 440.0)).round() as i64;
                        sums[note.rem_euclid(12) as usize] += power;
                    }
                }
                for (trace, sum) in values.iter_mut().zip(sums) {
                    trace.push(libm::sqrt(sum));
                }
            })?;
            Ok(AudioSpectrum {
                bands: values
                    .into_iter()
                    .map(|values| self.signal(values, hop))
                    .collect(),
            })
        })
    }

    /// Dominant spectral pitch in Hz, with parabolic peak refinement; silence is zero.
    pub fn pitch(&self) -> motion_core::Result<AudioSignal> {
        Ok(self
            .cached(Key::Pitch, 1, || {
                let mut values = Vec::new();
                let (_, hop) = self.stft(|_, energy, rate, window| {
                    let peak = energy
                        .iter()
                        .enumerate()
                        .skip(1)
                        .max_by(|left, right| left.1.total_cmp(right.1));
                    let frequency = peak.map_or(0.0, |(index, power)| {
                        if *power < 1e-12 {
                            return 0.0;
                        }
                        let first = energy[index - 1];
                        let third = energy.get(index + 1).copied().unwrap_or(*power);
                        let denominator = first - 2.0 * power + third;
                        let correction = if denominator.abs() > 1e-20 {
                            (0.5 * (first - third) / denominator).clamp(-0.5, 0.5)
                        } else {
                            0.0
                        };
                        (index as f64 + correction) * f64::from(rate) / window as f64
                    });
                    values.push(frequency);
                })?;
                Ok(AudioSpectrum {
                    bands: vec![self.signal(values, hop)],
                })
            })?
            .bands[0]
            .clone())
    }

    /// Positive spectral flux, offline and independent of subsequent seek order.
    pub fn onset_strength(&self) -> motion_core::Result<AudioSignal> {
        Ok(self
            .cached(Key::Flux, 1, || {
                let mut previous = Vec::new();
                let mut values = Vec::new();
                let (_, hop) = self.stft(|_, energy, _, _| {
                    if previous.is_empty() {
                        previous = vec![0.0; energy.len()];
                    }
                    let mut flux = 0.0;
                    for (old, power) in previous.iter_mut().zip(energy) {
                        let current = libm::sqrt(*power);
                        flux += (current - *old).max(0.0);
                        *old = current;
                    }
                    values.push(flux);
                })?;
                Ok(AudioSpectrum {
                    bands: vec![self.signal(values, hop)],
                })
            })?
            .bands[0]
            .clone())
    }

    /// Adaptive spectral-flux peaks with a 100 ms minimum separation.
    pub fn onsets(&self) -> motion_core::Result<AudioMarkers> {
        let flux = self.onset_strength()?;
        let values = flux.values();
        let hop = flux.0.hop_frames;
        let neighborhood = (f64::from(self.pcm.sample_rate()) * 0.15 / hop as f64).ceil() as usize;
        let maximum = values.iter().copied().fold(0.0, f64::max);
        let mut times = Vec::new();
        for (index, value) in values.iter().enumerate() {
            let first = index.saturating_sub(neighborhood);
            let end = (index + neighborhood + 1).min(values.len());
            let mean = values[first..end].iter().sum::<f64>() / (end - first) as f64;
            if *value > mean * 1.5 + maximum * 0.01
                && *value >= values.get(index.wrapping_sub(1)).copied().unwrap_or(0.0)
                && *value > values.get(index + 1).copied().unwrap_or(0.0)
            {
                let time = Duration::from_ticks(
                    (((index * hop + hop * 2) as u128 * motion_core::TICKS_PER_SECOND as u128)
                        / u128::from(self.pcm.sample_rate())) as u64,
                );
                if time < flux.0.duration
                    && times
                        .last()
                        .is_none_or(|previous: &Duration| seconds_between(time, *previous) >= 0.1)
                {
                    times.push(time);
                }
            }
        }
        Ok(AudioMarkers { times })
    }

    /// Autocorrelation tempo estimate with onset-based phase alignment; irregular music may need correction.
    pub fn beat_grid(&self) -> motion_core::Result<BeatGrid> {
        let onsets = self.onsets()?;
        if onsets.times.len() < 3 {
            return Err(invalid("beat_grid", "At least three reliable onsets are required").into());
        }
        let signal = self.onset_strength()?;
        let step = signal.values().len().div_ceil(50_000).max(1);
        let values: Vec<_> = signal
            .values()
            .chunks(step)
            .map(|chunk| chunk.iter().sum::<f64>())
            .collect();
        let seconds = signal.0.hop_frames as f64 * step as f64 / f64::from(signal.0.sample_rate);
        let first = (0.15 / seconds).ceil().max(1.0) as usize;
        let last = (3.0 / seconds).floor() as usize;
        let mut scores = vec![0.0; last + 1];
        for lag in first..=last.min(values.len().saturating_sub(1)) {
            let mut product = 0.0;
            let mut left = 0.0;
            let mut right = 0.0;
            for (first, second) in values[..values.len() - lag].iter().zip(&values[lag..]) {
                product += first * second;
                left += first * first;
                right += second * second;
            }
            scores[lag] = product / libm::sqrt((left * right).max(1e-30));
        }
        let mut intervals: Vec<_> = onsets
            .times
            .windows(2)
            .map(|pair| (pair[1] - pair[0]).as_seconds())
            .collect();
        intervals.sort_by(f64::total_cmp);
        let prior = intervals[intervals.len() / 2];
        for (index, score) in scores.iter_mut().enumerate() {
            *score /= 1.0 + (index as f64 * seconds / prior - 1.0).abs();
        }
        let maximum = scores.iter().copied().fold(0.0, f64::max);
        if maximum <= 0.0 {
            return Err(invalid("beat_grid", "No periodic onset energy").into());
        }
        let lag = (first..=last)
            .find(|index| {
                scores[*index] >= maximum * 0.97
                    && scores[*index] >= scores.get(index - 1).copied().unwrap_or(0.0)
                    && scores[*index] >= scores.get(index + 1).copied().unwrap_or(0.0)
            })
            .unwrap_or(first);
        let estimate = lag as f64 * seconds;
        let mut periods: Vec<_> = onsets
            .times
            .windows(2)
            .map(|pair| (pair[1] - pair[0]).as_seconds())
            .map(|interval| interval / (interval / estimate).round().max(1.0))
            .filter(|period| (*period - estimate).abs() < estimate * 0.1)
            .collect();
        if periods.is_empty() {
            return Err(invalid("beat_grid", "No stable onset intervals").into());
        }
        periods.sort_by(f64::total_cmp);
        let period = periods[periods.len() / 2];
        let offset = onsets
            .times
            .iter()
            .copied()
            .take(64)
            .max_by(|left, right| {
                let aligned = |origin: Duration| {
                    onsets
                        .times
                        .iter()
                        .filter(|time| {
                            let position = seconds_between(**time, origin) / period;
                            (position - position.round()).abs() < 0.1
                        })
                        .count()
                };
                aligned(*left)
                    .cmp(&aligned(*right))
                    .then_with(|| right.cmp(left))
            })
            .unwrap_or(Duration::ZERO);
        let aligned = onsets
            .times
            .iter()
            .filter(|time| {
                let position = seconds_between(**time, offset) / period;
                (position - position.round()).abs() < 0.1
            })
            .count();
        Ok(BeatGrid {
            bpm: 60.0 / period,
            offset: Duration::from_seconds(offset.as_seconds().rem_euclid(period)),
            confidence: aligned as f64 / onsets.times.len() as f64,
        })
    }
}

/// Signed seconds from `earlier` to `later`, exact in integer ticks before conversion.
fn seconds_between(later: Duration, earlier: Duration) -> f64 {
    (i128::from(later.ticks()) - i128::from(earlier.ticks())) as f64
        / motion_core::TICKS_PER_SECOND as f64
}

#[cfg(test)]
mod tests {
    use super::*;
    use motion_core::TimeLiteral;

    #[test]
    fn spectrum_pitch_and_signal_mapping_are_stateless() -> motion_core::Result {
        let signal = (0..16000)
            .map(|index| {
                (libm::sin(std::f64::consts::TAU * 1000.0 * index as f64 / 8000.0) * 0.5) as f32
            })
            .collect();
        let analysis = AudioAnalysis::from_pcm(Arc::new(PcmBuffer::new(8000, 1, signal)?));
        assert!((analysis.pitch()?.sample(500.milliseconds().after_origin()) - 1000.0).abs() < 1.0);
        assert_eq!(analysis.spectrum(32, 20.0, 4000.0)?.bands().len(), 32);
        assert_eq!(analysis.chroma()?.bands().len(), 12);
        let signal = analysis
            .loudness()?
            .normalize(0.95)?
            .attack_release(10.milliseconds(), 100.milliseconds())?
            .log_scale()
            .threshold(0.5)?;
        assert_eq!(signal.sample(500.milliseconds().after_origin()), 1.0);
        assert_eq!(signal.sample(2.seconds().after_origin()), 0.0);
        assert!(analysis.spectrum(65, 20.0, 4000.0).is_err());
        Ok(())
    }

    #[test]
    fn known_drum_pulses_produce_ordered_markers_and_tempo() -> motion_core::Result {
        let signal = (0..64000)
            .map(|index| {
                let local = index % 4000;
                (libm::sin(std::f64::consts::TAU * 1000.0 * local as f64 / 8000.0)
                    * libm::exp(-(local as f64) / 40.0)) as f32
            })
            .collect();
        let analysis = AudioAnalysis::from_pcm(Arc::new(PcmBuffer::new(8000, 1, signal)?));
        let markers = analysis.onsets()?;
        assert!(markers.times().len() >= 14);
        assert!(markers.times().windows(2).all(|pair| pair[0] < pair[1]));
        let grid = analysis.beat_grid()?;
        assert!((grid.bpm - 120.0).abs() < 2.0);
        assert!(grid.confidence > 0.8);
        assert_eq!(
            BeatGrid::new(120.0, Duration::ZERO)?
                .markers(2.seconds())?
                .times()
                .len(),
            4
        );
        Ok(())
    }

    #[test]
    fn marker_json_and_cached_parameter_groups_are_exact() -> motion_core::Result {
        let markers = AudioMarkers::new(vec![Duration::ZERO, Duration::from_ticks(12345)])?;
        let mut json = Vec::new();
        markers.write_json(&mut json)?;
        assert_eq!(
            AudioMarkers::read_json(&mut json.as_slice())?.times(),
            markers.times()
        );
        assert!(AudioMarkers::new(vec![1.seconds(), Duration::ZERO]).is_err());
        assert!(AudioMarkers::read_json(&mut b"{\"version\":2}".as_slice()).is_err());
        let analysis = AudioAnalysis::from_pcm(Arc::new(PcmBuffer::new(8000, 1, vec![0.1; 8000])?));
        let first = analysis.spectrum(32, 20.0, 4000.0)?;
        let second = analysis.clone().spectrum(32, 20.0, 4000.0)?;
        assert!(Arc::ptr_eq(&first.bands()[0].0, &second.bands()[0].0));
        let changed = analysis.spectrum(32, 30.0, 4000.0)?;
        assert!(!Arc::ptr_eq(&first.bands()[0].0, &changed.bands()[0].0));
        assert!(Arc::ptr_eq(&analysis.pitch()?.0, &analysis.pitch()?.0));
        Ok(())
    }
}
