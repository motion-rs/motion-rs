//! Compilation of path and SVG morphs driven by an animated progress.

use super::Compiler;
use super::element::Built;
use super::failure::failure;
use super::style::Style;
use crate::formula::{MorphSpec, SvgMorphSpec};
use motion_core::MorphPath;
use std::sync::Arc;

/// Contour samples used when a morph does not choose its own.
const DEFAULT_SAMPLES: u32 = 32;

/// Largest accepted SVG file in bytes.
#[cfg(feature = "svg")]
const MAX_SVG_BYTES: u64 = 16 * 1024 * 1024;

impl Compiler<'_> {
    /// Pairs two paths once, then interpolates per frame under the element style.
    pub(crate) fn morph(
        &mut self,
        spec: &MorphSpec,
        style: Option<Arc<Style>>,
        path: &str,
    ) -> Option<Built> {
        let from = self.svg_path(&spec.from, &format!("{path}.from"));
        let to = self.svg_path(&spec.to, &format!("{path}.to"));
        let progress = self.animated(&spec.progress, &format!("{path}.progress"));
        let samples = self.bounded(
            f64::from(spec.samples.unwrap_or(DEFAULT_SAMPLES)),
            4.0,
            256.0,
            path,
            "samples",
        );
        let (from, to, progress, samples, style) = (from?, to?, progress?, samples?, style?);
        let (source, target) = MorphPath::pair_with_samples(from, to, samples as usize)
            .map_err(|error| self.error(path, "morph", error.message))
            .ok()?;
        let varying = progress.varying() || style.varying();
        Some(Built::new(varying, move |time| {
            style.shape(source.try_interpolate(&target, progress.at(time)), time)
        }))
    }

    /// Prepares the morph between two SVG files at compile time.
    #[cfg(feature = "svg")]
    pub(crate) fn svg_morph(&mut self, spec: &SvgMorphSpec, path: &str) -> Option<Built> {
        use motion_content::{Morph, Svg, SvgAsset};
        let from = self.resolve(&spec.from, &format!("{path}.from"));
        let to = self.resolve(&spec.to, &format!("{path}.to"));
        let progress = self.animated(&spec.progress, &format!("{path}.progress"));
        let (from, to, progress) = (from?, to?, progress?);
        let mut trees = Vec::new();
        for (field, file) in [("from", from), ("to", to)] {
            let here = format!("{path}.{field}");
            let length = std::fs::metadata(&file)
                .map_err(|error| self.error(&here, "source", error.to_string()))
                .ok()?
                .len();
            if length > MAX_SVG_BYTES {
                self.limit(&here, "source", "SVG files are limited to 16 MiB");
                return None;
            }
            let bytes = std::fs::read(&file)
                .map_err(|error| self.error(&here, "source", error.to_string()))
                .ok()?;
            let asset = SvgAsset::from_bytes(&bytes)
                .map_err(|error| self.error(&here, "source", error.to_string()))
                .ok()?;
            let tree = Svg::from_asset(Arc::new(asset))
                .vector_tree()
                .map_err(|error| self.error(&here, "source", error.to_string()))
                .ok()?;
            trees.push(tree);
        }
        let morph = Morph::new(&trees[0], &trees[1])
            .map_err(|error| self.error(path, "morph", error.message))
            .ok()?;
        let varying = progress.varying();
        Some(Built::new(varying, move |time| {
            morph.sample(progress.at(time)).unwrap_or_else(failure)
        }))
    }

    #[cfg(not(feature = "svg"))]
    pub(crate) fn svg_morph(&mut self, _: &SvgMorphSpec, path: &str) -> Option<Built> {
        self.unsupported(path, "SVG morphs require feature `svg`");
        None
    }
}
