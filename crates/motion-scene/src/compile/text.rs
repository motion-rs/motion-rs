//! Compilation of paragraph text: runs, splitting, part reveal and staggered part animation.

use super::Compiler;
use super::element::Built;
use super::style::Style;
use crate::content::TextSpec;
use std::sync::Arc;

#[cfg(feature = "text")]
use super::value::{Lift, Prop};
#[cfg(feature = "text")]
use crate::content::{PartsSpec, TextSplitSpec};
#[cfg(feature = "text")]
use crate::options::TextAlignmentSpec;
#[cfg(feature = "text")]
use motion_core::{Duration, Node};

/// Most styled runs one rich paragraph may contain.
#[cfg(feature = "text")]
const MAX_SPANS: usize = 256;

impl Compiler<'_> {
    /// Compiles a paragraph; `style` is None when element styling already failed.
    #[cfg(feature = "text")]
    pub(crate) fn text(
        &mut self,
        spec: &TextSpec,
        style: Option<Arc<Style>>,
        path: &str,
    ) -> Option<Built> {
        use motion_content::{FontWeight, Text, TextAlignment, TextSpan, TextSplit};
        let rich = !spec.spans.is_empty();
        let mut valid = true;
        if rich == !spec.content.is_empty() {
            let message = if rich {
                "Give either `content` or `spans`, not both"
            } else {
                "Text requires `content` or `spans`"
            };
            self.error(path, "content", message);
            valid = false;
        }
        if spec.spans.len() > MAX_SPANS {
            self.limit(path, "spans", "Rich text is limited to 256 spans");
            valid = false;
        }
        let size = spec
            .size
            .as_ref()
            .map(|size| self.animated(size, &format!("{path}.size")));
        let write = spec
            .write
            .as_ref()
            .map(|write| self.animated(write, &format!("{path}.write")));
        let visible = spec
            .visible_parts
            .as_ref()
            .map(|count| self.animated(count, &format!("{path}.visible_parts")));
        let parts = spec
            .parts
            .as_ref()
            .map(|parts| self.parts(parts, &format!("{path}.parts")));
        let line_height = spec
            .line_height
            .map(|value| self.bounded(value, f64::MIN_POSITIVE, 1e3, path, "line_height"));
        let spacing = spec
            .letter_spacing
            .map(|value| self.bounded(value, -1e6, 1e6, path, "letter_spacing"));
        let width = spec
            .max_width
            .map(|value| self.bounded(value, f64::MIN_POSITIVE, 1e9, path, "max_width"));
        let (size, write, visible) = (size.lift()?, write.lift()?, visible.lift()?);
        let (parts, style) = (parts.lift()?, style?);
        if !valid {
            return None;
        }
        let mut text = if rich {
            Text::rich(spec.spans.iter().map(|span| {
                let mut run = TextSpan::new(span.text.as_str());
                if let Some(family) = &span.family {
                    run = run.family(family.as_str());
                }
                if let Some(size) = span.size {
                    run = run.size(size);
                }
                if let Some(weight) = span.weight {
                    run = run.weight(FontWeight(weight));
                }
                if span.italic {
                    run = run.italic(true);
                }
                if let Some(fill) = &span.fill {
                    run = run.fill(fill.color());
                }
                run
            }))
        } else {
            Text::new(spec.content.as_str())
        };
        if let Some(family) = &spec.family {
            text = text.family(family.as_str());
        }
        if let Some(weight) = spec.weight {
            text = text.weight(FontWeight(weight));
        }
        if let Some(height) = line_height {
            text = text.line_height(height?);
        }
        if let Some(spacing) = spacing {
            text = text.letter_spacing(spacing?);
        }
        if let Some(width) = width {
            text = text.max_width(width?);
        }
        if let Some(alignment) = spec.alignment {
            text = text.alignment(match alignment {
                TextAlignmentSpec::Start => TextAlignment::Start,
                TextAlignmentSpec::End => TextAlignment::End,
                TextAlignmentSpec::Center => TextAlignment::Center,
                TextAlignmentSpec::Justified => TextAlignment::Justified,
            });
        }
        if let Some(split) = spec.split {
            text = text.split(match split {
                TextSplitSpec::Characters => TextSplit::Characters,
                TextSplitSpec::Words => TextSplit::Words,
                TextSplitSpec::Lines => TextSplit::Lines,
            });
        }
        if let Some(fonts) = &self.fonts {
            text = text.fonts(fonts);
        }
        if let Some(parts) = parts {
            text = parts.apply(text);
        }
        // A fixed size is applied once so every frame shares the shaped layout cache.
        let size = match size {
            Some(Prop::Fixed(size)) => {
                text = text.size(size);
                None
            }
            size => size,
        };
        let varying = style.varying()
            || size.is_some()
            || write.as_ref().is_some_and(Prop::varying)
            || visible.as_ref().is_some_and(Prop::varying);
        Some(Built::new(varying, move |time| {
            let mut text = text.clone();
            if let Some(size) = &size {
                text = text.size(size.at(time));
            }
            if let Some(count) = &visible {
                text = text.visible_parts(count.at(time).max(0.0).floor() as usize);
            }
            text = style.text(text, time);
            if let Some(write) = &write {
                text = text.write(write.at(time));
            }
            Node::from(text)
        }))
    }

    #[cfg(feature = "text")]
    fn parts(&mut self, parts: &PartsSpec, path: &str) -> Option<PartProps> {
        let delay = self.seconds(parts.delay, path, "delay");
        let offset = parts
            .offset
            .as_ref()
            .map(|value| self.animated(value, &format!("{path}.offset")));
        let scale = parts
            .scale
            .as_ref()
            .map(|value| self.animated(value, &format!("{path}.scale")));
        let rotate = parts
            .rotate
            .as_ref()
            .map(|value| self.animated(value, &format!("{path}.rotate")));
        let opacity = parts
            .opacity
            .as_ref()
            .map(|value| self.animated(value, &format!("{path}.opacity")));
        Some(PartProps {
            delay: delay?,
            offset: offset.lift()?,
            scale: scale.lift()?,
            rotate: rotate.lift()?,
            opacity: opacity.lift()?,
        })
    }

    #[cfg(not(feature = "text"))]
    pub(crate) fn text(
        &mut self,
        _: &TextSpec,
        _: Option<Arc<Style>>,
        path: &str,
    ) -> Option<Built> {
        self.unsupported(path, "Text requires feature `text`");
        None
    }
}

/// Compiled per-part animation, evaluated by the engine at draw time.
#[cfg(feature = "text")]
struct PartProps {
    delay: Duration,
    offset: Option<Prop<[f64; 2]>>,
    scale: Option<Prop<f64>>,
    rotate: Option<Prop<f64>>,
    opacity: Option<Prop<f64>>,
}

#[cfg(feature = "text")]
impl PartProps {
    fn apply(self, text: motion_content::Text) -> motion_content::Text {
        let delay = self.delay;
        let props = Arc::new(self);
        text.animate_parts(delay, move |part| {
            let time = part.time();
            let mut style = motion_content::PartStyle::new();
            if let Some(offset) = &props.offset {
                let [horizontal, vertical] = offset.at(time.since_origin());
                style = style.offset(horizontal, vertical);
            }
            if let Some(scale) = &props.scale {
                style = style.scale(scale.at(time.since_origin()));
            }
            if let Some(rotate) = &props.rotate {
                style = style.rotate(rotate.at(time.since_origin()).to_radians());
            }
            if let Some(opacity) = &props.opacity {
                style = style.opacity(opacity.at(time.since_origin()));
            }
            style
        })
    }
}
