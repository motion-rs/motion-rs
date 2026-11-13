//! Compilation of path and SVG morphs driven by an animated progress.

use super::Compiler;
use super::element::Built;
#[cfg(feature = "svg")]
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
        use motion_content::Morph;
        let from = self.svg_asset(&spec.from, &format!("{path}.from"));
        let to = self.svg_asset(&spec.to, &format!("{path}.to"));
        let progress = self.animated(&spec.progress, &format!("{path}.progress"));
        let (from, to, progress) = (from?, to?, progress?);
        let morph = Morph::new(&from.tree, &to.tree)
            .map_err(|error| self.error(path, "morph", error.message))
            .ok()?;
        let varying = progress.varying();
        Some(Built::new(varying, move |time| {
            morph.sample(progress.at(time)).unwrap_or_else(failure)
        }))
    }

    #[cfg(feature = "svg")]
    pub(crate) fn svg_asset(
        &mut self,
        source: &crate::content::SvgSourceSpec,
        path: &str,
    ) -> Option<Arc<motion_content::SvgAsset>> {
        use crate::content::SvgSourceSpec;
        use std::io::Read;
        let bytes = match source {
            SvgSourceSpec::File(file) => {
                let file = self.resolve(file, path)?;
                let file = std::fs::File::open(file)
                    .map_err(|error| self.error(path, "source", error.to_string()))
                    .ok()?;
                let mut bytes = Vec::new();
                file.take(MAX_SVG_BYTES + 1)
                    .read_to_end(&mut bytes)
                    .map_err(|error| self.error(path, "source", error.to_string()))
                    .ok()?;
                if bytes.len() as u64 > MAX_SVG_BYTES {
                    self.limit(path, "source", "SVG sources are limited to 16 MiB");
                    return None;
                }
                bytes
            }
            SvgSourceSpec::Inline { data } => {
                if data.len() as u64 > MAX_SVG_BYTES {
                    self.limit(path, "data", "SVG sources are limited to 16 MiB");
                    return None;
                }
                return motion_content::SvgAsset::from_bytes(data.as_bytes())
                    .map(Arc::new)
                    .map_err(|error| self.error(path, "data", error.to_string()))
                    .ok();
            }
        };
        motion_content::SvgAsset::from_bytes(&bytes)
            .map(Arc::new)
            .map_err(|error| self.error(path, "source", error.to_string()))
            .ok()
    }

    #[cfg(not(feature = "svg"))]
    pub(crate) fn svg_morph(&mut self, _: &SvgMorphSpec, path: &str) -> Option<Built> {
        self.unsupported(path, "SVG morphs require feature `svg`");
        None
    }
}
