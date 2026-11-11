//! Headless audio readout model for meters and the pre-export loudness report.

use motion_core::Time;
use motion_encode::MixedAudio;

/// Bus level at one instant, in dBFS (silence is -96).
#[derive(Clone, Debug, PartialEq)]
pub struct BusReadout {
    /// Declared bus name.
    pub name: String,
    /// 50 ms RMS level.
    pub rms_db: f64,
    /// 50 ms sample-peak level.
    pub peak_db: f64,
}

/// Loudness, peak and per-bus meter data for a prepared mix.
#[derive(Clone, Debug, PartialEq)]
pub struct AudioReadout {
    /// Gated integrated loudness; `None` for silence.
    pub integrated_lufs: Option<f64>,
    /// Largest stored sample, in dBFS.
    pub sample_peak_db: f64,
    /// Reconstructed (4x oversampled) peak, in dBTP.
    pub true_peak_db: f64,
    /// Stereo correlation in -1..=1; `None` when a channel is silent.
    pub correlation: Option<f64>,
    /// Bus meters sampled at the requested time.
    pub buses: Vec<BusReadout>,
}

fn decibels(linear: f64) -> f64 {
    (20.0 * linear.max(1e-12).log10()).max(-96.0)
}

impl AudioReadout {
    /// Measures `mix` and samples its bus meters at `time`.
    pub fn from_mix(mix: &MixedAudio, time: Time) -> motion_core::Result<Self> {
        let report = mix.loudness_report()?;
        Ok(Self {
            integrated_lufs: report.integrated_lufs,
            sample_peak_db: decibels(report.sample_peak),
            true_peak_db: decibels(report.true_peak),
            correlation: report.correlation,
            buses: mix
                .bus_meters()
                .iter()
                .map(|meter| {
                    let [rms_db, peak_db] = meter.level(time.since_origin());
                    BusReadout {
                        name: meter.name.clone(),
                        rms_db,
                        peak_db,
                    }
                })
                .collect(),
        })
    }

    /// Whether the reconstructed peak exceeds `ceiling_db`, so export would clip or need limiting.
    pub fn exceeds(&self, ceiling_db: f64) -> bool {
        self.true_peak_db > ceiling_db
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use motion_core::{AudioBus, AudioClip, Size, TimeLiteral, Timeline};
    use motion_encode::{MixOptions, PcmBuffer};

    #[test]
    fn readout_reports_levels_and_bus_meters() -> motion_core::Result {
        let samples = (0..48_000)
            .flat_map(|frame| {
                let value = 0.5 * libm::sin(frame as f64 * 0.1) as f32;
                [value, value]
            })
            .collect();
        let timeline = Timeline::new(Size::new(8.0, 8.0), 30)
            .audio(AudioClip::source(PcmBuffer::new(48_000, 2, samples)?).bus("music"))
            .audio_bus(AudioBus::new("music"))
            .build()?;
        let mix = MixedAudio::new(&timeline, MixOptions::new(48_000))?;
        let readout = AudioReadout::from_mix(&mix, Time::ZERO)?;
        assert!(readout.integrated_lufs.is_some());
        assert!((readout.sample_peak_db + 6.02).abs() < 0.1);
        assert!(readout.exceeds(-7.0) && !readout.exceeds(-1.0));
        assert_eq!(readout.buses.len(), 1);
        assert_eq!(readout.buses[0].name, "music");
        assert!(readout.buses[0].peak_db > -7.0);
        assert!(AudioReadout::from_mix(&mix, 5.seconds().after_origin()).is_ok());
        Ok(())
    }
}
