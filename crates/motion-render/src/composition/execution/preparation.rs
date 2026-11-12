//! Scene evaluation and preparation: builds the scene, compiles it to a display list and
//! prepares coverage under the remaining raster limits.

use super::{FrameExecution, PreparedOutput};
use crate::RasterLimits;
use crate::composition::FrameTimings;
use motion_core::{Color, Node, Result, SceneContext, Time, ValidationError};

impl FrameExecution<'_> {
    pub(in crate::composition) fn prepare_node(
        &mut self,
        node: Node,
        background: Option<Color>,
    ) -> Result<(PreparedOutput, RasterLimits)> {
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
            .with_assets(assets)
            .with_invalid_geometry_marker(self.composition.options.mark_invalid_geometry);
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
        let tolerance = if self.composition.options.quality == motion_core::RenderQuality::Draft {
            0.35
        } else {
            1.0 / 256.0
        };
        let renderer = if let Some(renderer) = self.preparation_backend {
            renderer
        } else {
            #[cfg(feature = "gpu")]
            {
                self.backend.as_ref().map_or_else(
                    || {
                        if self.composition.options.backend.wants_gpu() {
                            crate::Renderer::Gpu
                        } else {
                            crate::Renderer::Cpu
                        }
                    },
                    |decision| decision.renderer,
                )
            }
            #[cfg(not(feature = "gpu"))]
            {
                self.select_backend()?
            }
        };
        let quality = self.composition.options.quality;
        let blend_space = self.composition.options.blend_space;
        let prepared = match renderer {
            crate::Renderer::Cpu => PreparedOutput::Cpu(
                crate::raster::prepare::prepare_validated_with_cache(
                    &list,
                    validation,
                    self.limits,
                    tolerance,
                    Some(&mut self.coverage_cache),
                )?
                .quality(quality)
                .blend_space(blend_space),
            ),
            crate::Renderer::Gpu => {
                #[cfg(feature = "gpu")]
                {
                    let frame = crate::gpu::raster::prepare::prepare_validated_device(
                        &list,
                        validation,
                        self.limits,
                        tolerance,
                        Some(&mut self.coverage_cache),
                    )?
                    .quality(quality)
                    .blend_space(blend_space);
                    PreparedOutput::Gpu(frame)
                }
                #[cfg(not(feature = "gpu"))]
                {
                    return Err(motion_core::ValidationError::unsupported(
                        "prepared",
                        "backend",
                        "GPU preparation requires the gpu feature",
                    )
                    .into());
                }
            }
        };
        if let Some(started) = started {
            self.timings.as_mut().expect("enabled timings").coverage += started.elapsed();
        }
        #[cfg(feature = "gpu")]
        if let PreparedOutput::Gpu(frame) = &prepared {
            self.gpu_preparation = Some(frame.geometry.stats(frame.stats()));
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
    ) -> Result<(PreparedOutput, RasterLimits)> {
        self.last_stats = None;
        self.gpu_preparation = None;
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
        .with_clip_duration(self.duration.since_origin().magnitude())
        .with_assets(&assets)
        .with_diagnostics(&diagnostics)
        .with_quality(self.composition.options.quality);
        let started = self.timings.map(|_| std::time::Instant::now());
        let node = build(&context).with_scene_time(context.time());
        if let Some(started) = started {
            self.timings.as_mut().expect("enabled timings").evaluation += started.elapsed();
        }
        let scene_errors = diagnostics.scene_errors();
        if !scene_errors.is_empty() {
            return Err(motion_core::Error::from(scene_errors).at_frame(u64::from(index)));
        }
        let result = self
            .prepare_node(node, background)
            .map_err(|error| error.at_frame(u64::from(index)));
        let failures = diagnostics.errors();
        let scene_errors = diagnostics.scene_errors();
        self.asset_diagnostics = failures
            .into_iter()
            .map(|error| {
                std::sync::Arc::new(
                    motion_core::Error::asset("resource", "frame", error)
                        .at_frame(u64::from(index)),
                )
            })
            .collect();
        if !scene_errors.is_empty() {
            return Err(motion_core::Error::from(scene_errors).at_frame(u64::from(index)));
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
