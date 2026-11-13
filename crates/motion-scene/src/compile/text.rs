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
use motion_core::{Duration, Node, Time};

/// Most styled runs one rich paragraph may contain.
#[cfg(feature = "text")]
const MAX_SPANS: usize = 256;

/// Largest numeric readout precision in decimal places.
#[cfg(feature = "text")]
const MAX_PRECISION: u8 = 16;

/// Largest combined numeric readout affix length, in bytes.
#[cfg(feature = "text")]
const MAX_AFFIX_BYTES: usize = 4096;

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
        let choices = usize::from(rich)
            + usize::from(!spec.content.is_empty())
            + usize::from(spec.number.is_some());
        if choices != 1 {
            self.error(
                path,
                "content",
                "Give exactly one of `content`, `spans` or `number`",
            );
            valid = false;
        }
        let number = spec.number.as_ref().map(|number| {
            if number.precision > MAX_PRECISION
                || number.prefix.len().saturating_add(number.suffix.len()) > MAX_AFFIX_BYTES
            {
                self.limit(
                    &format!("{path}.number"),
                    "format",
                    "Numeric text requires precision <=16 and affixes <=4096 bytes",
                );
                return None;
            }
            let value = self.animated(&number.value, &format!("{path}.number.value"))?;
            Some((
                value,
                number.precision as usize,
                number.prefix.clone(),
                number.suffix.clone(),
            ))
        });
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
        let number = number.lift()?;
        let (line_height, spacing, width) = (line_height.lift()?, spacing.lift()?, width.lift()?);
        if !valid {
            return None;
        }
        // A constant readout is formatted once, like `content`, so it is built a single time.
        let (constant, number) = match number {
            Some((value, precision, prefix, suffix)) if !value.varying() => (
                Some(format!(
                    "{prefix}{:.precision$}{suffix}",
                    value.at(Time::ZERO)
                )),
                None,
            ),
            number => (None, number),
        };
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
                    run = run.fill(*fill);
                }
                run
            }))
        } else if let Some(constant) = constant {
            Text::new(constant)
        } else {
            Text::new(spec.content.as_str())
        };
        let configuration = spec.clone();
        let fonts = self.fonts.clone();
        let parts = parts.map(Arc::new);
        let configure = move |mut text: Text| {
            let spec = &configuration;
            if let Some(family) = &spec.family {
                text = text.family(family.as_str());
            }
            if let Some(weight) = spec.weight {
                text = text.weight(FontWeight(weight));
            }
            if let Some(height) = line_height {
                text = text.line_height(height);
            }
            if let Some(spacing) = spacing {
                text = text.letter_spacing(spacing);
            }
            if let Some(width) = width {
                text = text.max_width(width);
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
            if let Some(fonts) = &fonts {
                text = text.fonts(fonts);
            }
            if let Some(parts) = &parts {
                text = parts.apply(text);
            }
            text
        };
        text = configure(text);
        // A fixed size is applied once so every frame shares the shaped layout cache.
        let fixed_size = match &size {
            Some(Prop::Fixed(size)) => Some(*size),
            _ => None,
        };
        let size = match size {
            Some(Prop::Fixed(size)) => {
                text = text.size(size);
                None
            }
            size => size,
        };
        let varying = style.varying()
            || number.is_some()
            || size.is_some()
            || write.as_ref().is_some_and(Prop::varying)
            || visible.as_ref().is_some_and(Prop::varying);
        Some(Built::new(varying, move |time| {
            let mut text = text.clone();
            if let Some((value, precision, prefix, suffix)) = &number {
                text = configure(Text::new(format!(
                    "{prefix}{:.precision$}{suffix}",
                    value.at(time)
                )));
                if let Some(size) = fixed_size {
                    text = text.size(size);
                }
            }
            if let Some(size) = &size {
                text = text.size(size.at(time));
            }
            if let Some(count) = &visible {
                text = text.visible_parts(count.at(time).max(0.0).floor() as usize);
            }
            text = match style.text(text, time) {
                Ok(text) => text,
                Err(error) => return super::failure::failure(error),
            };
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
    fn apply(self: &Arc<Self>, text: motion_content::Text) -> motion_content::Text {
        let delay = self.delay;
        let props = self.clone();
        text.animate_parts(delay, move |part| {
            let time = part.time();
            let mut style = motion_content::PartStyle::new();
            if let Some(offset) = &props.offset {
                let [horizontal, vertical] = offset.at(time);
                style = style.offset(horizontal, vertical);
            }
            if let Some(scale) = &props.scale {
                style = style.scale(scale.at(time));
            }
            if let Some(rotate) = &props.rotate {
                style = style.rotate(rotate.at(time).to_radians());
            }
            if let Some(opacity) = &props.opacity {
                style = style.opacity(opacity.at(time));
            }
            style
        })
    }
}
