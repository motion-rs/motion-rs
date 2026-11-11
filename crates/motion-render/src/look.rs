//! Original look recipes. No bundled third-party LUTs or implicit asset loading.

use crate::{DisplayTransform, FilmGrain, RenderOptions, ToneMap};
use motion_core::{
    Color, Filter, MotionBlur, Node, NodeModifiers, PixelSize, SceneContext, ShutterCurve,
    ShutterSampling,
};

/// Deterministic gate drift applied to the whole frame.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct GateWeave {
    /// Explicit sequence seed.
    pub seed: u64,
    /// Drift amplitude in design units.
    pub amount: f64,
}

/// Scene-side stages of a look: shutter, spatial filters and gate drift.
///
/// These change the scene graph. Output grading lives in [`DisplayTransform`]: its `look`
/// grade runs on the composited scene-linear frame before tone mapping, whereas a
/// [`Filter::Cdl`] in `filters` is an ordinary node filter applied to the shutter-accumulated
/// frame before composition and display processing.
#[derive(Clone, Debug)]
pub struct SceneEffects {
    /// Ordered spatial filters, applied once to the shutter-accumulated frame.
    pub filters: Vec<Filter>,
    /// Shutter exposure, if requested.
    pub shutter: Option<MotionBlur>,
    /// Temporal exposure policy used with `shutter`.
    pub sampling: ShutterSampling,
    /// Optional deterministic gate drift.
    pub gate_weave: Option<GateWeave>,
}

/// A pure, editable recipe pairing [`SceneEffects`] with an output [`DisplayTransform`].
///
/// Use [`Look::scene`] for the scene-side stages and [`Look::render_options`] for the
/// output-side stage; a look needs both. [`Look::into_parts`] returns the two values.
#[derive(Clone, Debug)]
pub struct Look {
    /// Shutter, spatial filters and gate drift.
    pub effects: SceneEffects,
    /// Output grading and grain.
    pub display: DisplayTransform,
}

impl Look {
    /// Neutral display-only output, no shutter and no spatial film effects.
    pub fn clean() -> Self {
        Self {
            effects: SceneEffects {
                filters: Vec::new(),
                shutter: None,
                sampling: ShutterSampling::default(),
                gate_weave: None,
            },
            display: DisplayTransform {
                tone_map: ToneMap::PbrNeutral,
                ..DisplayTransform::default()
            },
        }
    }

    /// Restrained AgX highlight shoulder, normalized white, bloom and 180-degree shutter.
    /// Use an explicit 24fps timeline when a traditional film cadence is desired.
    pub fn cinematic() -> Self {
        Self::graded(0.05)
    }

    /// Seeded encoded-domain grain, stronger warm halation and restrained physical gate drift.
    pub fn film(seed: u64) -> Self {
        let mut look = Self::graded(0.08);
        look.display.grain = Some(FilmGrain {
            seed,
            amount: 0.012,
            size: 1.5,
            luma_response: 1.0,
            chroma: 0.1,
        });
        look.effects.gate_weave = Some(GateWeave { seed, amount: 0.2 });
        look
    }

    fn graded(halation: f64) -> Self {
        Self {
            effects: SceneEffects {
                filters: vec![
                    Filter::Cdl(motion_core::Cdl {
                        slope: [1.015, 1.0, 0.99],
                        saturation: 0.98,
                        ..motion_core::Cdl::default()
                    }),
                    Filter::Bloom {
                        threshold: 0.9,
                        knee: 0.25,
                        radius: 4.0,
                        levels: 5,
                        falloff: 0.5,
                        intensity: 0.15,
                        tint: Color::WHITE,
                    },
                    Filter::halation(halation, 12.0).expect("valid fixed halation"),
                    Filter::Optics(motion_core::Optics::Lens {
                        distortion: 0.0,
                        chromatic: 0.0005,
                        auto_fit: true,
                        center: None,
                    }),
                    Filter::Optics(motion_core::Optics::LensVignette {
                        angle_of_view: 45.0,
                        strength: 0.12,
                        roundness: 0.5,
                    }),
                ],
                shutter: Some(MotionBlur::default()),
                sampling: ShutterSampling {
                    jitter_seed: Some(0x6d6f74696f6e),
                    adaptive: true,
                    curve: ShutterCurve::Cosine,
                },
                gate_weave: None,
            },
            display: DisplayTransform {
                tone_map: ToneMap::AgX,
                reference_white: Some(1.0),
                saturation: 0.96,
                ..DisplayTransform::default()
            },
        }
    }

    /// Splits the look into its scene-side effects and output transform.
    pub fn into_parts(self) -> (SceneEffects, DisplayTransform) {
        (self.effects, self.display)
    }

    /// Applies only the output-side stage: display transform. Spatial and shutter stages
    /// belong to [`Look::scene`].
    pub fn render_options(&self, options: RenderOptions) -> RenderOptions {
        options.display(self.display.clone())
    }

    /// Builds one frame with [`SceneEffects::build`].
    pub fn scene(
        &self,
        context: &SceneContext<'_>,
        output: PixelSize,
        build: impl FnMut(&SceneContext<'_>) -> Node,
    ) -> motion_core::Result<Node> {
        self.effects.build(context, output, build)
    }
}

impl SceneEffects {
    /// Builds one frame: captures the shutter over `build`, then applies the spatial filters
    /// and gate drift once to the accumulated result. Filtering after accumulation costs one
    /// filter chain per frame instead of one per shutter sample.
    ///
    /// `output` is the render size in pixels, used to estimate motion for adaptive sampling.
    pub fn build(
        &self,
        context: &SceneContext<'_>,
        output: PixelSize,
        build: impl FnMut(&SceneContext<'_>) -> Node,
    ) -> motion_core::Result<Node> {
        let mut build = build;
        let mut node = match self.shutter {
            Some(blur) => self.sampling.capture(blur, context, output, &mut build)?,
            None => build(context),
        };
        for filter in &self.filters {
            node = node.filter(filter.clone());
        }
        if let Some(weave) = self.gate_weave {
            let frame = context.time().to_frame(context.frame_rate()) as u64;
            node = node.gate_weave(weave.seed, frame, weave.amount);
        }
        Ok(node)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn film_strengthens_halation_without_adding_a_second_chain() {
        let cinematic = Look::cinematic();
        let film = Look::film(7);
        assert_eq!(film.effects.filters.len(), cinematic.effects.filters.len());
        assert_ne!(
            format!("{:?}", film.effects.filters),
            format!("{:?}", cinematic.effects.filters)
        );
        assert_eq!(
            film.effects.gate_weave,
            Some(GateWeave {
                seed: 7,
                amount: 0.2
            })
        );
    }
}
