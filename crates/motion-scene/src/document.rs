//! Versioned film documents: tracks, scenes, transitions, audio and looks.

use crate::audio::{AudioSpec, BusSpec, MasterSpec};
use crate::element::Element;
use crate::{FORMAT_VERSION, MAX_DOCUMENT_BYTES, definitions, diagnostics, params};
use motion_core::Color;
use motion_core::{Direction, IrisShape};
use motion_core::{FrameRate, ValidationError};
use schemars::{JsonSchema, Schema, SchemaGenerator, json_schema};
use serde::de::Error as _;
use serde::{Deserialize, Deserializer, Serialize, Serializer};
use std::borrow::Cow;
use std::collections::BTreeMap;
use std::path::Path;

/// A complete declarative film: canvas, frame rate, tracks, audio and look.
///
/// Times are seconds, angles are degrees and colors are CSS-style strings.
/// [`Document::from_json`] and [`Document::to_json`] round-trip the data model;
/// [`Document::compile`](crate::Document::compile) produces an engine timeline.
///
/// ```
/// use motion_scene::{Document, Element, SceneSpec, TrackSpec};
/// use motion_core::Color;
/// let document = Document::new([320.0, 180.0], 30).track(TrackSpec::new().scene(
///     SceneSpec::new(2.0, Element::circle([160.0, 90.0], 40.0).fill(Color::WHITE)),
/// ));
/// let json = document.to_json()?;
/// assert_eq!(Document::from_json(&json)?, document);
/// # Ok::<(), motion_core::Error>(())
/// ```
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct Document {
    /// Optional JSON Schema reference for editors; ignored by the engine.
    #[serde(rename = "$schema", default, skip_serializing_if = "Option::is_none")]
    pub schema: Option<String>,
    /// Format version; must equal [`FORMAT_VERSION`].
    #[schemars(range(min = 1, max = 1))]
    pub version: u32,
    /// Optional human-readable title.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    /// Logical design size `[width, height]`.
    pub size: [f64; 2],
    /// Frame rate: an integer, or a `"numerator/denominator"` string.
    pub fps: FrameRateSpec,
    /// Opaque background color behind every track.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub background: Option<Color>,
    /// Explicit film duration in seconds; the latest track or audio end by default.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[schemars(range(max = 1e6), extend("exclusiveMinimum" = 0))]
    pub duration: Option<f64>,
    /// Film look applied to every scene and the display transform.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub look: Option<LookSpec>,
    /// Named parameters: `{"$param": "name"}` values are replaced on load, and
    /// expressions read numeric ones as `$name`. Overrides replace values by key.
    /// `{"$calc": "expression"}` values resolve after overrides, before substitution.
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub params: BTreeMap<String, serde_json::Value>,
    /// Font files bound to every text element, relative to the document.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub fonts: Vec<String>,
    /// Tracks composited bottom to top.
    #[serde(default)]
    pub tracks: Vec<TrackSpec>,
    /// Named submixes, declared before the clips that route to them.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub buses: Vec<BusSpec>,
    /// Final gain, limiter and loudness normalization.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub master: Option<MasterSpec>,
    /// Audio occurrences mixed into export.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub audio: Vec<AudioSpec>,
}

impl Document {
    /// Creates an empty current-version document.
    pub fn new(size: [f64; 2], fps: impl Into<FrameRateSpec>) -> Self {
        Self {
            schema: None,
            version: FORMAT_VERSION,
            name: None,
            size,
            fps: fps.into(),
            background: None,
            duration: None,
            look: None,
            params: BTreeMap::new(),
            fonts: Vec::new(),
            tracks: Vec::new(),
            buses: Vec::new(),
            master: None,
            audio: Vec::new(),
        }
    }

    /// Parses a document, reporting the JSON path of any failure.
    ///
    /// Inputs over [`MAX_DOCUMENT_BYTES`] and unsupported versions are rejected.
    /// `{"$param": "name"}` references are replaced by their declared values.
    /// `{"$calc": "expression"}` evaluates finite numeric expressions once on load;
    /// parameters may depend on one another, but time and element bindings are invalid.
    /// `{"use": "name"}` references expand root `defs` after parameters resolve;
    /// sibling fields replace whole definition fields. Definitions are removed.
    pub fn from_json(text: &str) -> motion_core::Result<Self> {
        Self::from_json_with(text, None)
    }

    /// Parses a document after overriding declared parameters.
    ///
    /// `overrides` is a JSON object whose keys must be declared in `params`; each
    /// replaces the whole default value. The returned document holds the effective
    /// parameter values after `$calc` evaluation, so writing it back records the
    /// resolved film. References outside `params` are substituted; reusable `defs`
    /// are expanded and removed before type checking.
    pub fn from_json_with(
        text: &str,
        overrides: Option<&serde_json::Value>,
    ) -> motion_core::Result<Self> {
        if text.len() > MAX_DOCUMENT_BYTES {
            return Err(
                ValidationError::limit("document", "bytes", "Document exceeds 16 MiB").into(),
            );
        }
        let mut value: serde_json::Value = serde_json::from_str(text)
            .map_err(|error| ValidationError::invalid("document", "json", error.to_string()))?;
        params::resolve(&mut value, overrides, MAX_DOCUMENT_BYTES)
            .map_err(|error| diagnostics::annotate(error, text))?;
        definitions::resolve(&mut value, MAX_DOCUMENT_BYTES)
            .map_err(|error| diagnostics::annotate(error, text))?;
        let document: Self = serde_path_to_error::deserialize(value).map_err(|error| {
            let path = error.path().to_string();
            let inner = error.into_inner();
            let path = if path == "." { "document".into() } else { path };
            diagnostics::annotate(
                ValidationError::invalid(path, "json", inner.to_string()),
                text,
            )
        })?;
        document.check_version()?;
        Ok(document)
    }

    /// Reads and parses a UTF-8 document file.
    pub fn read(path: impl AsRef<Path>) -> motion_core::Result<Self> {
        Self::read_with(path, None)
    }

    /// Reads a document file, applying parameter overrides.
    pub fn read_with(
        path: impl AsRef<Path>,
        overrides: Option<&serde_json::Value>,
    ) -> motion_core::Result<Self> {
        let path = path.as_ref();
        let length = std::fs::metadata(path)?.len();
        if length > MAX_DOCUMENT_BYTES as u64 {
            return Err(
                ValidationError::limit("document", "bytes", "Document exceeds 16 MiB").into(),
            );
        }
        Self::from_json_with(&std::fs::read_to_string(path)?, overrides)
    }

    /// Serializes as pretty-printed JSON, omitting default values.
    pub fn to_json(&self) -> motion_core::Result<String> {
        serde_json::to_string_pretty(self)
            .map_err(|error| ValidationError::invalid("document", "json", error.to_string()).into())
    }

    /// Writes pretty-printed JSON with a trailing newline.
    pub fn write(&self, path: impl AsRef<Path>) -> motion_core::Result<()> {
        let mut text = self.to_json()?;
        text.push('\n');
        std::fs::write(path, text)?;
        Ok(())
    }

    /// Sets the human-readable title.
    pub fn name(mut self, name: impl Into<String>) -> Self {
        self.name = Some(name.into());
        self
    }

    /// Sets the background color.
    pub fn background(mut self, color: impl Into<Color>) -> Self {
        self.background = Some(color.into());
        self
    }

    /// Sets an explicit duration in seconds.
    pub fn duration(mut self, seconds: f64) -> Self {
        self.duration = Some(seconds);
        self
    }

    /// Selects the film look.
    pub fn look(mut self, look: LookSpec) -> Self {
        self.look = Some(look);
        self
    }

    /// Adds a font file.
    pub fn font(mut self, path: impl Into<String>) -> Self {
        self.fonts.push(path.into());
        self
    }

    /// Appends a track above the existing ones.
    pub fn track(mut self, track: TrackSpec) -> Self {
        self.tracks.push(track);
        self
    }

    /// Adds an audio occurrence.
    pub fn audio(mut self, audio: AudioSpec) -> Self {
        self.audio.push(audio);
        self
    }

    /// Declares a named submix.
    pub fn bus(mut self, bus: BusSpec) -> Self {
        self.buses.push(bus);
        self
    }

    /// Sets the master chain.
    pub fn master(mut self, master: MasterSpec) -> Self {
        self.master = Some(master);
        self
    }

    pub(crate) fn check_version(&self) -> Result<(), ValidationError> {
        if self.version == FORMAT_VERSION {
            Ok(())
        } else {
            Err(ValidationError::unsupported(
                "version",
                "version",
                format!(
                    "Document version {} is not supported; expected {FORMAT_VERSION}",
                    self.version
                ),
            ))
        }
    }
}

/// A rational frame rate, written as `30` or `"30000/1001"`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct FrameRateSpec {
    /// Nonzero frames.
    pub numerator: u32,
    /// Nonzero seconds.
    pub denominator: u32,
}

impl FrameRateSpec {
    /// Returns the engine rate, or None for a zero component.
    pub fn frame_rate(self) -> Option<FrameRate> {
        FrameRate::try_new(self.numerator, self.denominator).ok()
    }
}

impl From<u32> for FrameRateSpec {
    fn from(frames_per_second: u32) -> Self {
        Self {
            numerator: frames_per_second,
            denominator: 1,
        }
    }
}

impl From<FrameRate> for FrameRateSpec {
    fn from(rate: FrameRate) -> Self {
        Self {
            numerator: rate.numerator(),
            denominator: rate.denominator(),
        }
    }
}

impl Serialize for FrameRateSpec {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        if self.denominator == 1 {
            serializer.serialize_u32(self.numerator)
        } else {
            serializer.collect_str(&format_args!("{}/{}", self.numerator, self.denominator))
        }
    }
}

impl<'de> Deserialize<'de> for FrameRateSpec {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        match serde_json::Value::deserialize(deserializer)? {
            serde_json::Value::Number(number) => number
                .as_u64()
                .and_then(|value| u32::try_from(value).ok())
                .map(Self::from)
                .ok_or_else(|| D::Error::custom("Frame rate must be a positive integer")),
            serde_json::Value::String(text) => text
                .split_once('/')
                .and_then(|(numerator, denominator)| {
                    Some(Self {
                        numerator: numerator.trim().parse().ok()?,
                        denominator: denominator.trim().parse().ok()?,
                    })
                })
                .ok_or_else(|| D::Error::custom("Frame rate string must be NUMERATOR/DENOMINATOR")),
            _ => Err(D::Error::custom(
                "Frame rate must be an integer or a NUMERATOR/DENOMINATOR string",
            )),
        }
    }
}

impl JsonSchema for FrameRateSpec {
    fn schema_name() -> Cow<'static, str> {
        "FrameRate".into()
    }

    fn json_schema(_: &mut SchemaGenerator) -> Schema {
        json_schema!({
            "description": "Integer frames per second, or a NUMERATOR/DENOMINATOR string.",
            "anyOf": [
                {"type": "integer", "minimum": 1, "maximum": u32::MAX},
                {"type": "string", "pattern": "^\\s*[0-9]+\\s*/\\s*[0-9]+\\s*$"}
            ]
        })
    }
}

/// One sequential lane of scenes, gaps and transitions.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct TrackSpec {
    /// Optional diagnostic name.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    /// Items in cursor order.
    #[serde(default)]
    pub items: Vec<TrackItem>,
}

impl TrackSpec {
    /// Creates an empty track.
    pub fn new() -> Self {
        Self::default()
    }

    /// Appends a scene at the cursor, or at its explicit start.
    pub fn scene(mut self, scene: SceneSpec) -> Self {
        self.items.push(TrackItem::Scene(Box::new(scene)));
        self
    }

    /// Overlaps the next scene with the previous one.
    pub fn transition(mut self, effect: TransitionEffect, duration: f64) -> Self {
        self.items
            .push(TrackItem::Transition(TransitionSpec { effect, duration }));
        self
    }

    /// Advances the cursor by `seconds`.
    pub fn gap(mut self, seconds: f64) -> Self {
        self.items.push(TrackItem::Gap(seconds));
        self
    }
}

/// A track entry, written as `{"scene": ...}`, `{"transition": ...}` or `{"gap": SECONDS}`.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case", deny_unknown_fields)]
pub enum TrackItem {
    /// A timed element tree.
    Scene(Box<SceneSpec>),
    /// Overlap between the previous and next scene.
    Transition(TransitionSpec),
    /// Cursor advance in seconds.
    Gap(f64),
}

/// A timed element tree evaluated in scene-local time.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct SceneSpec {
    /// Optional diagnostic name.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    /// Absolute start in seconds; the track cursor by default.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[schemars(range(min = 0.0, max = 1e6))]
    pub start: Option<f64>,
    /// Positive duration in seconds.
    #[schemars(range(max = 1e6), extend("exclusiveMinimum" = 0))]
    pub duration: f64,
    /// Root element.
    pub content: Element,
}

impl SceneSpec {
    /// Creates a scene placed at the track cursor.
    pub fn new(duration: f64, content: Element) -> Self {
        Self {
            name: None,
            start: None,
            duration,
            content,
        }
    }

    /// Sets the diagnostic name.
    pub fn name(mut self, name: impl Into<String>) -> Self {
        self.name = Some(name.into());
        self
    }

    /// Places the scene at an absolute start time.
    pub fn start(mut self, seconds: f64) -> Self {
        self.start = Some(seconds);
        self
    }
}

/// A transition into the next scene.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct TransitionSpec {
    /// Composition effect.
    pub effect: TransitionEffect,
    /// Positive overlap in seconds.
    pub duration: f64,
}

/// A transition effect, written as `"cross_fade"` or `{"slide": {"direction": "left"}}`.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case", deny_unknown_fields)]
pub enum TransitionEffect {
    /// Linear premultiplied dissolve.
    CrossFade,
    /// Incoming view slides over the outgoing view.
    Slide {
        /// Travel direction.
        direction: Direction,
    },
    /// Both views move together.
    Push {
        /// Travel direction.
        direction: Direction,
    },
    /// Advancing projected edge.
    Wipe {
        /// Edge normal in degrees from +X.
        #[serde(default)]
        angle: f64,
        /// Nonnegative soft edge width in design units.
        #[serde(default)]
        feather: f64,
    },
    /// Centered growing reveal.
    Iris {
        /// Reveal geometry.
        #[serde(default)]
        shape: IrisShape,
    },
    /// Radial zoom blur dissolve.
    ZoomBlur,
}

/// A built-in film look, written as `"cinematic"` or `{"film": {"seed": 42}}`.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case", deny_unknown_fields)]
pub enum LookSpec {
    /// Neutral tone mapping only.
    Clean,
    /// Graded highlights, bloom, lens and 180-degree shutter.
    Cinematic,
    /// Cinematic plus seeded grain, halation and gate weave.
    Film {
        /// Explicit grain and weave seed.
        seed: u64,
    },
}
