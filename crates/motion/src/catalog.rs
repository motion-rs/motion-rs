//! Compiled scene registration and portable visual-check metadata.

use std::collections::BTreeSet;
use std::sync::Arc;

use crate::render::Composition;
use crate::{Node, PixelSize, Rect, Scene, SceneContext, ValidationError};

/// A named, integer-pixel region used for automated nonblank checks.
#[derive(Clone, Debug)]
pub struct VisualRegion {
    /// Human-readable label included in reports and failures.
    pub name: String,
    /// Output-pixel bounds; all edges must be integers inside the composition.
    pub bounds: Rect,
    /// Required pixels different from the output's top-left reference pixel.
    pub min_nonblank_pixels: usize,
}

impl VisualRegion {
    /// Stores region metadata without painting. Defaults to 101 nonblank pixels.
    pub fn new(name: impl Into<String>, bounds: Rect) -> Self {
        Self {
            name: name.into(),
            bounds,
            min_nonblank_pixels: 101,
        }
    }
}

/// Metadata for automated keyframe checks; not a human visual acceptance result.
/// Constructing or attaching this policy does not render frames or execute checks.
#[derive(Clone, Debug)]
pub struct VisualCheckPolicy {
    /// Between 2 and 1024 distinct indices inside the composition, in sampling order.
    /// Sorting is not required; the supplied first/last entries are compared for motion.
    pub frames: Vec<u32>,
    /// At most 64 named output-pixel regions whose content must be nonblank.
    pub regions: Vec<VisualRegion>,
    /// Allowed maximum per-channel CPU/GPU RGB difference.
    pub max_rgb_difference: u8,
    /// Minimum changed pixels between first and last sampled frames; zero permits static art.
    pub min_changed_pixels: usize,
}

impl VisualCheckPolicy {
    /// Stores bounded-check metadata; validation occurs in [`SceneDescriptor::preflight`].
    /// Defaults to RGB tolerance 3 and at least 101 first/last changed pixels.
    pub fn new(frames: impl Into<Vec<u32>>, regions: impl Into<Vec<VisualRegion>>) -> Self {
        Self {
            frames: frames.into(),
            regions: regions.into(),
            max_rgb_difference: 3,
            min_changed_pixels: 101,
        }
    }

    fn validate(&self, size: PixelSize, frames: u64, id: &str) -> Result<(), ValidationError> {
        let invalid = |field: &str, message: &str| {
            ValidationError::invalid(format!("catalog[{id}]"), field, message)
        };
        if !(2..=1024).contains(&self.frames.len()) {
            return Err(invalid("visual.frames", "Require 2..=1024 sampled frames"));
        }
        let unique: BTreeSet<_> = self.frames.iter().copied().collect();
        if unique.len() != self.frames.len()
            || self.frames.iter().any(|index| u64::from(*index) >= frames)
        {
            return Err(invalid(
                "visual.frames",
                "Require 2..=1024 distinct indices inside the composition",
            ));
        }
        if self.regions.len() > 64 {
            return Err(invalid("visual", "At most 64 regions are allowed"));
        }
        for (index, region) in self.regions.iter().enumerate() {
            let bounds = region.bounds;
            if region.name.is_empty()
                || region.name.len() > 256
                || [bounds.left, bounds.top, bounds.right, bounds.bottom]
                    .iter()
                    .any(|value| !value.is_finite() || value.fract() != 0.0)
                || bounds.left < 0.0
                || bounds.top < 0.0
                || bounds.width() <= 0.0
                || bounds.height() <= 0.0
                || bounds.right > f64::from(size.width)
                || bounds.bottom > f64::from(size.height)
                || region.min_nonblank_pixels > bounds.width() as usize * bounds.height() as usize
            {
                return Err(invalid(
                    &format!("visual.regions[{index}]"),
                    "Require a named integer-pixel region inside the output with an attainable nonblank threshold",
                ));
            }
        }
        if self.min_changed_pixels > size.width as usize * size.height as usize {
            return Err(invalid(
                "visual.min_changed_pixels",
                "Changed-pixel threshold exceeds the output pixel count",
            ));
        }
        Ok(())
    }
}

/// One source of truth for a compiled scene, Studio registration and keyframe tests.
///
/// IDs are stable ASCII letters/digits, `-`, `_` or `.`, independent of display
/// names. Builders are shared and may run concurrently, just as in Studio; they
/// must not depend on prior invocations. Arbitrary callbacks are not serializable.
#[derive(Clone)]
pub struct SceneDescriptor {
    /// Stable identifier, validated as 1..=128 ASCII bytes.
    pub id: String,
    /// Display name, independent of the identifier.
    pub name: String,
    /// Output settings, shared by preview, export and checks.
    pub composition: Composition,
    /// Optional automated keyframe policy, not an acceptance result.
    pub visual: Option<VisualCheckPolicy>,
    pub(crate) build: Arc<dyn Scene>,
}

impl SceneDescriptor {
    /// Registers metadata and a deterministic builder without rendering or I/O.
    /// Use [`Self::preflight`] to reject invalid IDs, settings or check metadata.
    pub fn new(
        id: impl Into<String>,
        name: impl Into<String>,
        composition: Composition,
        build: impl Scene + 'static,
    ) -> Self {
        Self {
            id: id.into(),
            name: name.into(),
            composition,
            visual: None,
            build: Arc::new(build),
        }
    }

    /// Attaches one visual-check policy without invoking the scene builder.
    pub fn visual_check(mut self, check: VisualCheckPolicy) -> Self {
        self.visual = Some(check);
        self
    }

    /// Calls the builder with explicit local/global time and resource inputs.
    /// Does not run [`Self::preflight`], check that the context belongs to this
    /// composition, or validate the returned scene. Callback panics propagate.
    pub fn build(&self, context: &SceneContext<'_>) -> Node {
        self.build.render(context)
    }

    /// Checks metadata and configuration without invoking callbacks or loading fonts.
    /// Does not validate generated scenes or run the visual checks. Catalog owners
    /// additionally check ID uniqueness across registrations.
    pub fn preflight(&self) -> Result<(), ValidationError> {
        if self.id.is_empty()
            || self.id.len() > 128
            || !self
                .id
                .bytes()
                .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_' | b'.'))
        {
            return Err(ValidationError::invalid(
                "catalog",
                "id",
                "Require 1..=128 ASCII letters/digits, '-', '_' or '.'",
            ));
        }
        if self.name.is_empty() || self.name.len() > 256 || self.name.chars().any(char::is_control)
        {
            return Err(ValidationError::invalid(
                format!("catalog[{}]", self.id),
                "name",
                "Require 1..=256 bytes without control characters",
            ));
        }
        self.composition.preflight()?;
        if let Some(check) = &self.visual {
            check.validate(
                self.composition.output_size(),
                self.composition.frame_count(),
                &self.id,
            )?;
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn spec() -> SceneDescriptor {
        SceneDescriptor::new(
            "study.v1",
            "Study",
            Composition::new(16, 8, crate::render::FrameClock::new(30, 7)),
            |_: &SceneContext<'_>| Node::empty(),
        )
        .visual_check(VisualCheckPolicy::new(
            vec![0, 6],
            vec![VisualRegion::new("art", Rect::new(0.0, 0.0, 16.0, 8.0))],
        ))
    }

    #[test]
    fn specs_validate_custom_dimensions_timing_and_pixel_regions_without_fonts() {
        assert!(spec().preflight().is_ok());
        let mut invalid = spec();
        invalid.id = "../invalid id".to_owned();
        assert_eq!(invalid.preflight().unwrap_err().field, "id");
        let mut invalid = spec();
        invalid.visual.as_mut().unwrap().frames = vec![0, 7];
        assert_eq!(invalid.preflight().unwrap_err().field, "visual.frames");
        invalid.visual.as_mut().unwrap().frames = vec![0, 0];
        assert!(invalid.preflight().is_err());
        let mut invalid = spec();
        invalid.visual.as_mut().unwrap().regions[0].bounds.right = 17.0;
        assert_eq!(invalid.preflight().unwrap_err().field, "visual.regions[0]");
    }

    #[test]
    fn rational_catalog_preflight_never_evaluates_scene() {
        let spec = SceneDescriptor::new(
            "ntsc",
            "Rational",
            Composition::new(
                16,
                8,
                crate::render::FrameClock::new(crate::FrameRate::NTSC, 7),
            ),
            |_: &SceneContext<'_>| -> Node {
                panic!("metadata validation must not evaluate scenes")
            },
        )
        .visual_check(spec().visual.unwrap());
        spec.preflight().unwrap();
        let mut fractional = spec.clone();
        fractional.visual.as_mut().unwrap().regions[0].bounds.left = 0.125;
        assert_eq!(
            fractional.preflight().unwrap_err().field,
            "visual.regions[0]"
        );
    }
}
