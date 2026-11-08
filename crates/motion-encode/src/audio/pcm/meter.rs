//! Bus submixing with offline RMS and sample-peak meter traces.

use super::mix::{MixedAudio, PreparedClip};
use super::{MAX_SAMPLES, invalid};
use motion_core::Time;
use std::sync::Arc;

/// Immutable prepared stereo mix. Reads perform fixed-order f64 summation and are
/// independent of seek order; source reads/resampler history never run at query time.
#[derive(Clone, Debug)]
pub struct BusMeter {
    /// Declared bus identity.
    pub name: String,
    pub(super) rate: u32,
    pub(super) hop: u64,
    pub(super) start: u64,
    pub(super) frames: u64,
    pub(super) levels: Arc<[[f64; 2]]>,
}

impl BusMeter {
    /// Offline 50 ms RMS and sample-peak levels in dBFS; silence/EOF return -96 dBFS.
    pub fn level(&self, time: Time) -> [f64; 2] {
        if time < Time::ZERO {
            return [-96.0; 2];
        }
        let frame = (time.as_seconds() * f64::from(self.rate)).floor() as u64;
        if frame >= self.frames {
            return [-96.0; 2];
        }
        let index = (self.start.saturating_add(frame) / self.hop) as usize;
        self.levels.get(index).copied().unwrap_or([-96.0; 2])
    }
}

impl MixedAudio {
    pub(super) fn process_buses(
        mut self,
        buses: &[motion_core::AudioBus],
    ) -> motion_core::Result<Self> {
        if self.clips.iter().any(|clip| {
            clip.bus
                .as_ref()
                .is_some_and(|name| !buses.iter().any(|bus| bus.name == *name))
        }) {
            return Err(invalid("bus", "Clip references an undeclared audio bus").into());
        }
        if buses.is_empty() {
            return Ok(self);
        }
        let mut processed: Vec<_> = self
            .clips
            .iter()
            .filter(|clip| clip.bus.is_none())
            .cloned()
            .collect();
        let mut retained: usize = processed.iter().map(|clip| clip.data.len()).sum();
        let mut meters = Vec::new();
        let mut meter_bytes = 0_usize;
        let mut rendered: std::collections::HashMap<String, Arc<[f32]>> = Default::default();
        for (index, bus) in buses.iter().enumerate() {
            if !self
                .clips
                .iter()
                .any(|clip| clip.bus.as_deref() == Some(&bus.name))
            {
                continue;
            }
            if self.frames > (MAX_SAMPLES / 2) as u64
                || retained.saturating_add(self.frames as usize * 2) > MAX_SAMPLES
            {
                return Err(invalid("bus_bytes", "Submixes exceed 128 MiB").into());
            }
            let mix = Self {
                rate: self.rate,
                quality: self.quality,
                frames: self.frames,
                clips: self
                    .clips
                    .iter()
                    .filter(|clip| clip.bus.as_deref() == Some(&bus.name))
                    .cloned()
                    .collect::<Vec<_>>()
                    .into(),
                meters: Arc::new([]),
                levels: Arc::new(std::sync::OnceLock::new()),
            };
            let mut samples = mix.snapshot()?;
            for effect in &bus.effects {
                effect.validate(self.rate)?;
                effect.process(&mut samples, 2, self.rate)?;
            }
            crate::audio::master::process(
                &mut samples,
                self.rate,
                motion_core::Master {
                    gain_db: bus.gain_db,
                    ..Default::default()
                },
            )?;
            for ducking in &bus.ducking {
                ducking.duck.validate()?;
                let reduction = match &ducking.trigger {
                    motion_core::DuckTrigger::Bus(name) => {
                        let declared = buses.iter().position(|other| other.name == *name);
                        if !declared.is_some_and(|position| position < index) {
                            return Err(invalid(
                                "ducking",
                                "Bus trigger must be declared before the ducked bus",
                            )
                            .into());
                        }
                        let Some(trigger) = rendered.get(name) else {
                            continue;
                        };
                        crate::audio::ducking::reduction_db(trigger, self.rate, &ducking.duck)
                    }
                    motion_core::DuckTrigger::Clip(trigger) => {
                        let trigger = Self::new_window(
                            std::slice::from_ref(trigger.as_ref()),
                            self.rate,
                            self.duration_time(),
                            Time::ZERO,
                            self.quality,
                        )?
                        .snapshot()?;
                        crate::audio::ducking::reduction_db(&trigger, self.rate, &ducking.duck)
                    }
                };
                crate::audio::ducking::apply(&mut samples, 0, &reduction);
            }
            let hop = (self.rate / 20).max(1) as usize;
            meter_bytes = meter_bytes.saturating_add(samples.len().div_ceil(hop * 2) * 16);
            if meter_bytes > 16 * 1024 * 1024 {
                return Err(invalid("bus_meter_bytes", "Bus meter traces exceed 16 MiB").into());
            }
            let levels = samples
                .chunks(hop * 2)
                .map(|chunk| {
                    let peak = chunk
                        .iter()
                        .map(|sample| f64::from(sample.abs()))
                        .fold(0.0, f64::max);
                    let rms = libm::sqrt(
                        chunk
                            .iter()
                            .map(|sample| f64::from(*sample).powi(2))
                            .sum::<f64>()
                            / chunk.len() as f64,
                    );
                    [rms, peak].map(|value| (20.0 * libm::log10(value.max(1e-12))).max(-96.0))
                })
                .collect::<Vec<_>>();
            meters.push(BusMeter {
                name: bus.name.clone(),
                rate: self.rate,
                hop: hop as u64,
                start: 0,
                frames: self.frames,
                levels: levels.into(),
            });
            retained += samples.len();
            let samples: Arc<[f32]> = samples.into();
            rendered.insert(bus.name.clone(), samples.clone());
            processed.push(PreparedClip {
                start: 0,
                data: samples,
                bus: None,
            });
        }
        self.clips = processed.into();
        self.meters = meters.into();
        self.levels = Arc::new(std::sync::OnceLock::new());
        Ok(self)
    }
}
