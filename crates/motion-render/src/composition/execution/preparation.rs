//! Scene evaluation and preparation: builds the scene, compiles it to a display list and
//! prepares coverage under the remaining raster limits.

use super::FrameExecution;
use crate::RasterLimits;
use crate::composition::FrameTimings;
use motion_core::{Color, Node, Result, SceneContext, Time, ValidationError};

impl FrameExecution<'_> {
    pub(in crate::composition) fn prepare_node(
        &mut self,
        node: Node,
        background: Option<Color>,
    ) -> Result<(crate::PreparedFrame, RasterLimits)> {
        let validation = self.context().validation_limits();
        let assets = self
            .composition
            .options
            .resources
            .as_ref()
            .unwrap_or_else(|| self.context().assets());
        let mut drawing = motion_core::DrawContext::default()
            .with_quality(self.composition.options.quality)
            .with_render_cache(self.options.cache.clone())
            .with_font_usage(self.options.font_usage.clone())
            .with_diagnostics(self.diagnostics.clone())
            .with_assets(assets);
        if let Some(fonts) = assets.fonts() {
            drawing = drawing.with_fonts(fonts);
        }
        let started = self.timings.map(|_| std::time::Instant::now());
        let list = node
            .compile_with_context(self.design_size, self.size, validation, &drawing)?
            .with_background(background);
        if let Some(started) = started {
            self.timings.as_mut().expect("enabled timings").compile += started.elapsed();
        }
        let started = self.timings.map(|_| std::time::Instant::now());
        let prepared = crate::prepared::prepare_validated_with_cache(
            &list,
            validation,
            self.limits,
            if self.composition.options.quality == motion_core::RenderQuality::Draft {
                0.35
            } else {
                1.0 / 256.0
            },
            Some(&mut self.coverage_cache),
        )?
        .quality(self.composition.options.quality)
        .blend_space(self.composition.options.blend_space);
        if let Some(started) = started {
            self.timings.as_mut().expect("enabled timings").coverage += started.elapsed();
        }
        let limits = remaining(self.limits, prepared.stats().work)?;
        Ok((prepared, limits))
    }

    pub(in crate::composition) fn prepare_frame(
        &mut self,
        index: u32,
        time: Option<Time>,
        background: Option<Color>,
        build: impl FnOnce(&SceneContext<'_>) -> Node,
    ) -> Result<(crate::PreparedFrame, RasterLimits)> {
        self.last_stats = None;
        if self.timings.is_some() {
            self.timings = Some(FrameTimings::default());
        }
        self.asset_diagnostics.clear();
        self.composition.validate_frame_index(index)?;
        self.limits.validate_size(self.size)?;
        let rate = self.composition.clock.rate;
        let diagnostics = motion_core::RequestDiagnostics::default();
        self.diagnostics = diagnostics.clone();
        let assets = self
            .composition
            .options
            .resources
            .as_ref()
            .unwrap_or_else(|| self.context().assets())
            .clone()
            .with_diagnostics(diagnostics.clone());
        let context = SceneContext::new(
            time.or(self.time)
                .unwrap_or_else(|| Time::from_frame(i64::from(index), rate)),
            rate,
            self.design_size,
            &self.scene_cache,
        )
        .with_clip_duration(self.duration)
        .with_assets(&assets)
        .with_diagnostics(&diagnostics)
        .with_quality(self.composition.options.quality);
        let started = self.timings.map(|_| std::time::Instant::now());
        let node = build(&context).with_scene_time(context.time());
        if let Some(started) = started {
            self.timings.as_mut().expect("enabled timings").evaluation += started.elapsed();
        }
        let scene_errors = diagnostics
            .errors()
            .into_iter()
            .flat_map(|error| error.build_errors().to_vec())
            .collect::<Vec<_>>();
        if !scene_errors.is_empty() {
            return Err(motion_core::Error::BuildErrors(scene_errors).at_frame(u64::from(index)));
        }
        let result = self
            .prepare_node(node, background)
            .map_err(|error| error.at_frame(u64::from(index)));
        let failures = diagnostics.errors();
        let scene_errors = failures
            .iter()
            .flat_map(|error| error.build_errors().to_vec())
            .collect::<Vec<_>>();
        self.asset_diagnostics = failures
            .into_iter()
            .filter(|error| error.build_errors().is_empty())
            .map(|error| {
                std::sync::Arc::new(
                    motion_core::Error::asset("resource", "frame", error)
                        .at_frame(u64::from(index)),
                )
            })
            .collect();
        if !scene_errors.is_empty() {
            return Err(motion_core::Error::BuildErrors(scene_errors).at_frame(u64::from(index)));
        }
        let result = result?;
        if !self.composition.options.allow_missing_assets
            && let Some(error) = self.asset_diagnostics.first()
        {
            return Err(
                motion_core::Error::asset("resource", "frame", error.clone())
                    .at_frame(u64::from(index)),
            );
        }
        Ok(result)
    }
}

pub(super) fn remaining(
    limits: RasterLimits,
    spent: u64,
) -> std::result::Result<RasterLimits, ValidationError> {
    let max_work = limits
        .max_work
        .checked_sub(spent)
        .filter(|work| *work > 0)
        .ok_or_else(|| {
            ValidationError::limit(
                "_frame",
                "max_work",
                "frame exhausted its shared work budget",
            )
        })?;
    Ok(RasterLimits { max_work, ..limits })
}
