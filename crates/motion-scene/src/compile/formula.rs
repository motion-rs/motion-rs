//! Compilation of Typst math formulas, with optional handwriting or morphing.

use super::Compiler;
use super::element::Built;
use crate::formula::FormulaSpec;

#[cfg(feature = "formula")]
use super::failure::failure;
#[cfg(feature = "formula")]
use super::value::{Lift, Prop};
#[cfg(feature = "formula")]
use motion_core::Node;
#[cfg(feature = "formula")]
use std::sync::Arc;

impl Compiler<'_> {
    /// Compiles the formula eagerly so Typst errors surface as located diagnostics.
    #[cfg(feature = "formula")]
    pub(crate) fn formula(&mut self, spec: &FormulaSpec, path: &str) -> Option<Built> {
        use motion_formula::{FormulaCompiler, FormulaOptions};
        let mut valid = true;
        if spec.morph_to.is_some() != spec.progress.is_some() {
            self.error(path, "progress", "`morph_to` and `progress` go together");
            valid = false;
        }
        if spec.morph_to.is_some() && spec.write.is_some() {
            self.error(
                path,
                "write",
                "A formula either morphs or is written, not both",
            );
            valid = false;
        }
        let write = spec
            .write
            .as_ref()
            .map(|write| self.animated(write, &format!("{path}.write")));
        let progress = spec
            .progress
            .as_ref()
            .map(|progress| self.animated(progress, &format!("{path}.progress")));
        let (write, progress) = (write.lift()?, progress.lift()?);
        let mut options = FormulaOptions::default();
        if let Some(size) = spec.size {
            options.size = size;
        }
        if let Some(color) = &spec.color {
            options.fill = *color;
        }
        if let Some(family) = &spec.family {
            options.family = family.as_str().into();
        }
        let compiler = match &self.fonts {
            Some(fonts) => FormulaCompiler::with_fonts(fonts),
            None => Ok(FormulaCompiler::new()),
        };
        let compiler = compiler
            .map_err(|error| self.error(path, "fonts", error.to_string()))
            .ok()?;
        let asset = compiler
            .compile(spec.source.as_str(), &options)
            .map_err(|error| self.error(&format!("{path}.source"), "formula", error.to_string()))
            .ok()?;
        let morph = match &spec.morph_to {
            Some(target) => {
                let target = compiler
                    .compile(target.as_str(), &options)
                    .map_err(|error| {
                        self.error(&format!("{path}.morph_to"), "formula", error.to_string())
                    })
                    .ok()?;
                let morph = asset
                    .morph_to(&target)
                    .map_err(|error| self.error(path, "morph_to", error.message))
                    .ok()?;
                Some(Arc::new(morph))
            }
            None => None,
        };
        if !valid {
            return None;
        }
        let varying = write.as_ref().is_some_and(Prop::varying)
            || progress.as_ref().is_some_and(Prop::varying);
        Some(Built::new(varying, move |time| {
            let result: Result<Node, motion_core::ValidationError> =
                match (&morph, &progress, &write) {
                    (Some(morph), Some(progress), _) => morph.sample(progress.at(time)),
                    (None, _, Some(write)) => asset.write(write.at(time)),
                    _ => Ok(asset.node()),
                };
            result.unwrap_or_else(failure)
        }))
    }

    #[cfg(not(feature = "formula"))]
    pub(crate) fn formula(&mut self, _: &FormulaSpec, path: &str) -> Option<Built> {
        self.unsupported(path, "Formulas require feature `formula`");
        None
    }
}
