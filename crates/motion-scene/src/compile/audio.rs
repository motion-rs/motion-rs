//! Compilation of audio occurrences, buses, effects, ducking and the master chain.

use super::Compiler;
#[cfg(feature = "audio")]
use super::binding::scalar;
use super::style::point;
use super::value::Prop;
use crate::audio::{
    AudioSpec, BusSpec, DuckSpec, DuckTriggerSpec, EffectSpec, MasterSpec, PanLawSpec,
};
use crate::document::Document;
use motion_core::{
    AudioBus, AudioClip, Duck, DuckTrigger, Duration, Limiter, Loudness, Master, PanLaw, Point,
    Signal,
};
use std::collections::HashMap;
use std::sync::Arc;

#[cfg(feature = "audio")]
use motion_encode::audio::effects::{AnimatedEq, Effect, EqKind};

/// Everything audio a document attaches to its timeline, in declaration order.
pub(crate) struct AudioPlan {
    pub(crate) clips: Vec<AudioClip>,
    pub(crate) buses: Vec<AudioBus>,
    pub(crate) master: Option<Master>,
}

/// An effect ready to attach to a clip or bus.
#[cfg(feature = "audio")]
enum BuiltEffect {
    Plain(Effect),
    Animated(AnimatedEq),
}

impl Compiler<'_> {
    /// Compiles clips first, so bus and clip ducking can name earlier clips by `id`.
    pub(crate) fn audio_plan(&mut self, document: &Document) -> AudioPlan {
        let mut triggers: HashMap<String, AudioClip> = HashMap::new();
        let mut clips = Vec::new();
        for (index, audio) in document.audio.iter().enumerate() {
            let path = format!("audio[{index}]");
            if let Some(clip) = self.audio_clip(audio, &path, &triggers) {
                if let Some(id) = &audio.id
                    && triggers.insert(id.clone(), clip.clone()).is_some()
                {
                    self.error(&path, "id", format!("Duplicate audio id `{id}`"));
                }
                clips.push(clip);
            }
        }
        let mut buses = Vec::new();
        let mut declared: Vec<&str> = Vec::new();
        for (index, bus) in document.buses.iter().enumerate() {
            let path = format!("buses[{index}]");
            if let Some(built) = self.bus(bus, &path, &triggers, &declared) {
                buses.push(built);
            }
            declared.push(&bus.name);
        }
        let master = document
            .master
            .as_ref()
            .and_then(|master| self.master(master, "master"));
        AudioPlan {
            clips,
            buses,
            master,
        }
    }

    fn audio_clip(
        &mut self,
        audio: &AudioSpec,
        path: &str,
        triggers: &HashMap<String, AudioClip>,
    ) -> Option<AudioClip> {
        let source = self.resolve(&audio.source, &format!("{path}.source"));
        let mut clip = source.map(AudioClip::file);
        let mut valid = clip.is_some();
        for (field, value) in [
            ("start", audio.start),
            ("offset", audio.offset),
            ("duration", audio.duration),
            ("fade_in", audio.fade_in),
            ("fade_out", audio.fade_out),
        ] {
            let Some(seconds) = value else { continue };
            let Some(time) = self.seconds(seconds, path, field) else {
                valid = false;
                continue;
            };
            clip = clip.map(|clip| match field {
                "start" => clip.start(time.after_origin()),
                "offset" => clip.offset(time),
                "duration" => clip.duration(time),
                "fade_in" => clip.fade_in(time),
                _ => clip.fade_out(time),
            });
        }
        if let Some(volume) = &audio.volume {
            match self.animated(volume, &format!("{path}.volume")) {
                Some(Prop::Fixed(value)) => {
                    valid &= self.bounded(value, 0.0, 16.0, path, "volume").is_some();
                    clip = clip.map(|clip| clip.volume(value));
                }
                Some(Prop::Varying(envelope)) => {
                    clip = clip.map(|clip| clip.volume_envelope(envelope))
                }
                None => valid = false,
            }
        }
        if let Some(pan) = &audio.pan {
            match self.animated(pan, &format!("{path}.pan")) {
                Some(Prop::Fixed(value)) => {
                    valid &= self.bounded(value, -1.0, 1.0, path, "pan").is_some();
                    clip = clip.map(|clip| clip.pan(value));
                }
                Some(Prop::Varying(animation)) => {
                    clip = clip.map(|clip| clip.pan_animation(animation));
                }
                None => valid = false,
            }
        }
        if let Some(law) = audio.pan_law {
            clip = clip.map(|clip| {
                clip.pan_law(match law {
                    PanLawSpec::Linear => PanLaw::Linear,
                    PanLawSpec::ConstantPower => PanLaw::ConstantPower,
                    PanLawSpec::Minus4Point5Db => PanLaw::Minus4_5dB,
                })
            });
        }
        if let Some(width) = audio.stereo_width {
            valid &= self
                .bounded(width, 0.0, 2.0, path, "stereo_width")
                .is_some();
            clip = clip.map(|clip| clip.stereo_width(width));
        }
        if let Some(spatial) = &audio.spatial {
            let here = format!("{path}.spatial");
            let position = self.animated(&spatial.position, &format!("{here}.position"));
            match (position, clip.take()) {
                (Some(position), Some(next)) => {
                    let animation: Arc<dyn Signal<Point>> = match position {
                        Prop::Fixed(value) => Arc::new(move |_: Duration| point(value)),
                        Prop::Varying(animation) => {
                            Arc::new(move |time: Duration| point(animation.sample(time)))
                        }
                    };
                    match next.spatial(
                        animation,
                        point(spatial.listener),
                        spatial.width,
                        spatial.rolloff,
                    ) {
                        Ok(next) => clip = Some(next),
                        Err(error) => {
                            self.error(&here, "spatial", error.message);
                            valid = false;
                        }
                    }
                }
                _ => valid = false,
            }
        }
        if let Some(bus) = &audio.bus {
            if bus.is_empty() || bus.len() > 128 {
                self.error(path, "bus", "Bus names are 1..=128 bytes");
                valid = false;
            }
            clip = clip.map(|clip| clip.bus(bus.as_str()));
        }
        for (index, effect) in audio.effects.iter().enumerate() {
            let here = format!("{path}.effects[{index}]");
            match self.effect(effect, &here) {
                #[cfg(feature = "audio")]
                Some(BuiltEffect::Plain(effect)) => clip = clip.map(|clip| clip.effect(effect)),
                #[cfg(feature = "audio")]
                Some(BuiltEffect::Animated(effect)) => clip = clip.map(|clip| clip.effect(effect)),
                #[cfg(not(feature = "audio"))]
                Some(never) => match never {},
                None => valid = false,
            }
        }
        for (index, duck) in audio.ducked_by.iter().enumerate() {
            let here = format!("{path}.ducked_by[{index}]");
            let trigger = match &duck.trigger {
                DuckTriggerSpec::Clip(id) => match triggers.get(id) {
                    Some(trigger) => Some(trigger),
                    None => {
                        self.unknown_clip(&here, id, triggers);
                        None
                    }
                },
                DuckTriggerSpec::Bus(_) => {
                    self.error(
                        &here,
                        "trigger",
                        "Clips duck under clips; bus triggers apply to buses",
                    );
                    None
                }
            };
            let duck = self.duck(duck, &here);
            match (trigger, duck) {
                (Some(trigger), Some(duck)) => {
                    clip = clip.map(|clip| clip.ducked_by(trigger, duck))
                }
                _ => valid = false,
            }
        }
        if valid { clip } else { None }
    }

    fn bus(
        &mut self,
        spec: &BusSpec,
        path: &str,
        triggers: &HashMap<String, AudioClip>,
        declared: &[&str],
    ) -> Option<AudioBus> {
        let mut bus = AudioBus::new(spec.name.as_str());
        let mut valid = true;
        if let Some(gain) = spec.gain_db {
            valid &= self.bounded(gain, -96.0, 24.0, path, "gain_db").is_some();
            bus.gain_db = gain;
        }
        for (index, effect) in spec.effects.iter().enumerate() {
            let here = format!("{path}.effects[{index}]");
            match self.effect(effect, &here) {
                #[cfg(feature = "audio")]
                Some(BuiltEffect::Plain(effect)) => bus.effects.push(Arc::new(effect)),
                #[cfg(feature = "audio")]
                Some(BuiltEffect::Animated(effect)) => bus.effects.push(Arc::new(effect)),
                #[cfg(not(feature = "audio"))]
                Some(never) => match never {},
                None => valid = false,
            }
        }
        for (index, duck) in spec.ducked_by.iter().enumerate() {
            let here = format!("{path}.ducked_by[{index}]");
            let trigger = match &duck.trigger {
                DuckTriggerSpec::Clip(id) => triggers
                    .get(id)
                    .map(|clip| DuckTrigger::Clip(Arc::new(clip.clone())))
                    .or_else(|| {
                        self.unknown_clip(&here, id, triggers);
                        None
                    }),
                DuckTriggerSpec::Bus(name) if declared.contains(&name.as_str()) => {
                    Some(DuckTrigger::Bus(name.clone()))
                }
                DuckTriggerSpec::Bus(name) => {
                    let mut message = format!("No earlier bus named `{name}`");
                    if let Some(hint) =
                        crate::diagnostics::did_you_mean(name, declared.iter().copied())
                    {
                        message.push_str(". ");
                        message.push_str(&hint);
                    }
                    self.error(&here, "trigger", message);
                    None
                }
            };
            match (trigger, self.duck(duck, &here)) {
                (Some(trigger), Some(duck)) => bus = bus.ducked_by(trigger, duck),
                _ => valid = false,
            }
        }
        valid.then_some(bus)
    }

    fn unknown_clip(&mut self, path: &str, id: &str, triggers: &HashMap<String, AudioClip>) {
        let mut message = format!("No earlier audio clip with id `{id}`");
        if let Some(hint) =
            crate::diagnostics::did_you_mean(id, triggers.keys().map(String::as_str))
        {
            message.push_str(". ");
            message.push_str(&hint);
        }
        self.error(path, "trigger", message);
    }

    fn duck(&mut self, spec: &DuckSpec, path: &str) -> Option<Duck> {
        let mut duck = Duck::new(
            spec.threshold_db.unwrap_or(-45.0),
            spec.depth_db.unwrap_or(12.0),
        );
        let mut valid = true;
        if let Some(ratio) = spec.ratio {
            duck = duck.ratio(ratio);
        }
        let defaults = Duck::default();
        let attack = self.optional_seconds(spec.attack, defaults.attack, path, "attack");
        let release = self.optional_seconds(spec.release, defaults.release, path, "release");
        let lookahead =
            self.optional_seconds(spec.lookahead, defaults.lookahead, path, "lookahead");
        match (attack, release, lookahead) {
            (Some(attack), Some(release), Some(lookahead)) => {
                duck = duck.timing(attack, release).lookahead(lookahead);
            }
            _ => valid = false,
        }
        valid.then_some(duck)
    }

    fn master(&mut self, spec: &MasterSpec, path: &str) -> Option<Master> {
        let mut master = Master {
            gain_db: spec.gain_db.unwrap_or(0.0),
            ..Master::default()
        };
        let mut valid = true;
        if let Some(limiter) = &spec.limiter {
            let here = format!("{path}.limiter");
            let defaults = Master::safe().limiter?;
            let lookahead =
                self.optional_seconds(limiter.lookahead, defaults.lookahead, &here, "lookahead");
            let release =
                self.optional_seconds(limiter.release, defaults.release, &here, "release");
            match (lookahead, release) {
                (Some(lookahead), Some(release)) => {
                    master.limiter = Some(Limiter {
                        ceiling_db: limiter.ceiling_db,
                        lookahead,
                        release,
                    });
                }
                _ => valid = false,
            }
        }
        if let Some(normalize) = &spec.normalize {
            master.normalize = Some(Loudness {
                target_lufs: normalize.target_lufs,
                true_peak_db: normalize.true_peak_db,
            });
        }
        valid.then_some(master)
    }

    /// Converts an optional seconds value, falling back to an engine default.
    fn optional_seconds(
        &mut self,
        seconds: Option<f64>,
        default: Duration,
        path: &str,
        field: &str,
    ) -> Option<Duration> {
        match seconds {
            Some(seconds) => self.seconds(seconds, path, field),
            None => Some(default),
        }
    }

    #[cfg(feature = "audio")]
    fn effect(&mut self, spec: &EffectSpec, path: &str) -> Option<BuiltEffect> {
        Some(match spec {
            EffectSpec::Eq {
                kind,
                frequency,
                q,
                gain_db,
            } => {
                let kind = eq_kind(*kind);
                let q = q.unwrap_or(0.707);
                let frequency = self.animated(frequency, &format!("{path}.eq.frequency"));
                let gain = self.animated_or(gain_db.as_ref(), 0.0, &format!("{path}.eq.gain_db"));
                match (frequency?, gain?) {
                    (Prop::Fixed(frequency), Prop::Fixed(gain_db)) => {
                        BuiltEffect::Plain(Effect::Eq {
                            kind,
                            frequency,
                            q,
                            gain_db,
                        })
                    }
                    (frequency, gain) => BuiltEffect::Animated(
                        AnimatedEq::new(kind, scalar(frequency), q, 0.0).gain(scalar(gain)),
                    ),
                }
            }
            EffectSpec::Compressor {
                threshold_db,
                ratio,
                attack,
                release,
            } => {
                let here = format!("{path}.compressor");
                let attack = self.optional_seconds(*attack, seconds(0.01), &here, "attack");
                let release = self.optional_seconds(*release, seconds(0.1), &here, "release");
                BuiltEffect::Plain(Effect::Compressor {
                    threshold_db: *threshold_db,
                    ratio: *ratio,
                    attack: attack?,
                    release: release?,
                })
            }
            EffectSpec::Gate {
                threshold_db,
                attack,
                release,
            } => {
                let here = format!("{path}.gate");
                let attack = self.optional_seconds(*attack, seconds(0.005), &here, "attack");
                let release = self.optional_seconds(*release, seconds(0.1), &here, "release");
                BuiltEffect::Plain(Effect::Gate {
                    threshold_db: *threshold_db,
                    attack: attack?,
                    release: release?,
                })
            }
            EffectSpec::DeEsser {
                frequency,
                threshold_db,
                ratio,
            } => BuiltEffect::Plain(Effect::DeEsser {
                frequency: *frequency,
                threshold_db: *threshold_db,
                ratio: *ratio,
            }),
            EffectSpec::Delay {
                delay,
                feedback,
                wet,
                tail,
            } => {
                let here = format!("{path}.delay");
                let time = self.seconds(*delay, &here, "delay");
                let tail = self.optional_seconds(*tail, seconds(1.0), &here, "tail");
                BuiltEffect::Plain(Effect::Delay {
                    delay: time?,
                    feedback: feedback.unwrap_or(0.3),
                    wet: wet.unwrap_or(0.3),
                    tail: tail?,
                })
            }
            EffectSpec::Reverb {
                room,
                damping,
                wet,
                tail,
            } => {
                let here = format!("{path}.reverb");
                let tail = self.optional_seconds(*tail, seconds(2.0), &here, "tail");
                BuiltEffect::Plain(Effect::Reverb {
                    room: *room,
                    damping: damping.unwrap_or(0.5),
                    wet: wet.unwrap_or(0.3),
                    tail: tail?,
                })
            }
        })
    }

    #[cfg(not(feature = "audio"))]
    fn effect(&mut self, _: &EffectSpec, path: &str) -> Option<std::convert::Infallible> {
        self.unsupported(path, "Audio effects require feature `audio`");
        None
    }
}

#[cfg(feature = "audio")]
fn eq_kind(kind: crate::audio::EqKindSpec) -> EqKind {
    use crate::audio::EqKindSpec;
    match kind {
        EqKindSpec::HighPass => EqKind::HighPass,
        EqKindSpec::LowPass => EqKind::LowPass,
        EqKindSpec::Peak => EqKind::Peak,
        EqKindSpec::LowShelf => EqKind::LowShelf,
        EqKindSpec::HighShelf => EqKind::HighShelf,
        EqKindSpec::Notch => EqKind::Notch,
    }
}

#[cfg(feature = "audio")]
fn seconds(value: f64) -> Duration {
    Duration::from_seconds(value)
}
