//! Compilation of elements into time-sampled scene nodes with transforms and effects.

use super::Compiler;
use super::style::point;
use super::value::Lift;
use super::value::Prop;
use crate::effects::{ClipShapeSpec, FilterSpec, MaskSpec};
use crate::element::Element;
use motion_core::{
    BlendMode, Cdl, Circle, ClipMode, Color, Filter, MaskMode, Node, NodeModifiers, Path2D, Pivot,
    Rect, SceneCache, SceneContext, Size, Time, Vector2,
};
use std::sync::Arc;

/// Deepest accepted element nesting, including masks.
const MAX_DEPTH: usize = 64;

/// A compiled element: a snapshot shared by every frame, or a per-context builder.
#[derive(Clone)]
pub(crate) enum Built {
    Fixed(Node),
    Varying(Arc<dyn Fn(&SceneContext<'_>) -> Node + Send + Sync>),
}

impl Built {
    /// Builds from scene-local time alone, evaluating once when nothing varies.
    pub(crate) fn new(varying: bool, build: impl Fn(Time) -> Node + Send + Sync + 'static) -> Self {
        Self::with_context(varying, move |context| build(context.time()))
    }

    /// Builds from the full scene context, evaluating once when nothing varies.
    ///
    /// A non-varying build runs against a placeholder context, so it must not read
    /// the clock, cache or assets; content that does must report itself as varying.
    pub(crate) fn with_context(
        varying: bool,
        build: impl Fn(&SceneContext<'_>) -> Node + Send + Sync + 'static,
    ) -> Self {
        if varying {
            return Self::Varying(Arc::new(build));
        }
        let cache = SceneCache::default();
        let placeholder = SceneContext::new(
            motion_core::Time::ZERO,
            motion_core::fps!(1),
            Size::new(1.0, 1.0),
            &cache,
        );
        Self::Fixed(build(&placeholder))
    }

    pub(crate) fn at(&self, context: &SceneContext<'_>) -> Node {
        match self {
            Self::Fixed(node) => node.clone(),
            Self::Varying(build) => build(context),
        }
    }

    pub(crate) fn varying(&self) -> bool {
        matches!(self, Self::Varying(_))
    }
}

impl Compiler<'_> {
    /// Compiles an element tree, recording every diagnostic below `path`.
    pub(crate) fn element(&mut self, element: &Element, path: &str, depth: usize) -> Option<Built> {
        if depth > MAX_DEPTH {
            self.limit(path, "depth", "Element nesting exceeds 64 levels");
            return None;
        }
        let style = self.style(element, path);
        let content = self.content(&element.content, style, path, depth);
        let modifiers = self.modifiers(element, path, depth);
        if element.hidden {
            return content.and(modifiers).map(|_| Built::Fixed(Node::empty()));
        }
        let (content, modifiers) = (content?, modifiers?);
        let varying = content.varying() || modifiers.varying();
        Some(Built::with_context(varying, move |context| {
            modifiers.apply(content.at(context), context)
        }))
    }

    fn modifiers(&mut self, element: &Element, path: &str, depth: usize) -> Option<Modifiers> {
        let position = self.optional(element.position.as_ref(), &format!("{path}.position"));
        let translate = self.optional(element.translate.as_ref(), &format!("{path}.translate"));
        let rotate = self.optional(element.rotate.as_ref(), &format!("{path}.rotate"));
        let scale = self.optional(element.scale.as_ref(), &format!("{path}.scale"));
        let scale_xy = self.optional(element.scale_xy.as_ref(), &format!("{path}.scale_xy"));
        let opacity = self.optional(element.opacity.as_ref(), &format!("{path}.opacity"));
        let filters = element
            .filters
            .iter()
            .enumerate()
            .map(|(index, filter)| self.filter(filter, &format!("{path}.filters[{index}]")))
            .collect::<Vec<_>>();
        let clip = element.clip.as_ref().map(|clip| {
            let shape = self.clip_shape(&clip.shape, &format!("{path}.clip.shape"))?;
            Some((shape, clip.mode.unwrap_or_default()))
        });
        let mask = element
            .mask
            .as_deref()
            .map(|mask| self.mask(mask, &format!("{path}.mask"), depth));
        Some(Modifiers {
            name: element.name.as_deref().map(Arc::from),
            position: position?,
            translate: translate?,
            rotate: rotate?,
            scale: scale?,
            scale_xy: scale_xy?,
            pivot: element.pivot.map(Pivot::from),
            opacity: opacity?,
            blend: element.blend,
            z_index: element.z_index,
            filters: filters.into_iter().collect::<Option<_>>()?,
            clip: clip.lift()?,
            mask: mask.lift()?,
        })
    }

    fn mask(&mut self, mask: &MaskSpec, path: &str, depth: usize) -> Option<(Built, MaskMode)> {
        let built = self.element(&mask.element, &format!("{path}.element"), depth + 1)?;
        Some((built, mask.mode.unwrap_or_default()))
    }

    fn clip_shape(&mut self, shape: &ClipShapeSpec, path: &str) -> Option<ClipProp> {
        match shape {
            ClipShapeSpec::Rect(bounds) => self
                .animated(bounds, &format!("{path}.rect"))
                .map(ClipProp::Rect),
            ClipShapeSpec::Circle(circle) => {
                let center = self.animated(&circle.center, &format!("{path}.circle.center"));
                let radius = self.animated(&circle.radius, &format!("{path}.circle.radius"));
                Some(ClipProp::Circle(center?, radius?))
            }
            ClipShapeSpec::Path(data) => self
                .svg_path(data, &format!("{path}.path"))
                .map(ClipProp::Path),
        }
    }

    pub(crate) fn svg_path(&mut self, data: &str, path: &str) -> Option<Path2D> {
        Path2D::parse_svg(data)
            .map_err(|error| self.error(path, "data", error.message))
            .ok()
    }

    fn filter(&mut self, filter: &FilterSpec, path: &str) -> Option<FilterProp> {
        match filter {
            FilterSpec::Blur { sigma } => self
                .animated(sigma, &format!("{path}.blur.sigma"))
                .map(FilterProp::Blur),
            FilterSpec::DropShadow {
                offset,
                sigma,
                color,
            } => {
                let path = format!("{path}.drop_shadow");
                let offset = self.animated(offset, &format!("{path}.offset"));
                let sigma = self.animated(sigma, &format!("{path}.sigma"));
                let color = self.animated(color, &format!("{path}.color"));
                Some(FilterProp::DropShadow(offset?, sigma?, color?))
            }
            FilterSpec::Glow {
                threshold,
                sigma,
                intensity,
            } => {
                let path = format!("{path}.glow");
                let threshold = self.animated(threshold, &format!("{path}.threshold"));
                let sigma = self.animated(sigma, &format!("{path}.sigma"));
                let intensity = self.animated(intensity, &format!("{path}.intensity"));
                Some(FilterProp::Glow(threshold?, sigma?, intensity?))
            }
            FilterSpec::Bloom(bloom) => Some(FilterProp::Fixed(Filter::Bloom {
                threshold: bloom.threshold.unwrap_or(0.9),
                knee: bloom.knee.unwrap_or(0.25),
                radius: bloom.radius.unwrap_or(4.0),
                levels: bloom.levels.unwrap_or(5),
                falloff: bloom.falloff.unwrap_or(0.5),
                intensity: bloom.intensity.unwrap_or(0.15),
                tint: bloom
                    .tint
                    .as_ref()
                    .map_or(Color::WHITE, |tint| tint.color()),
            })),
            FilterSpec::ColorAdjust(adjust) => {
                let path = format!("{path}.color_adjust");
                let exposure =
                    self.animated_or(adjust.exposure.as_ref(), 0.0, &format!("{path}.exposure"));
                let contrast =
                    self.animated_or(adjust.contrast.as_ref(), 1.0, &format!("{path}.contrast"));
                let saturation = self.animated_or(
                    adjust.saturation.as_ref(),
                    1.0,
                    &format!("{path}.saturation"),
                );
                let hue = self.animated_or(adjust.hue.as_ref(), 0.0, &format!("{path}.hue"));
                Some(FilterProp::ColorAdjust([
                    exposure?,
                    contrast?,
                    saturation?,
                    hue?,
                ]))
            }
            FilterSpec::Cdl(cdl) => {
                let path = format!("{path}.cdl");
                let slope = self.triple(cdl.slope.unwrap_or([1.0; 3]), &path, "slope");
                let offset = self.triple(cdl.offset.unwrap_or([0.0; 3]), &path, "offset");
                let power = self.triple(cdl.power.unwrap_or([1.0; 3]), &path, "power");
                let saturation =
                    self.triple([cdl.saturation.unwrap_or(1.0); 3], &path, "saturation");
                Some(FilterProp::Fixed(Filter::Cdl(Cdl {
                    slope: slope?,
                    offset: offset?,
                    power: power?,
                    saturation: saturation?[0],
                })))
            }
            FilterSpec::Halation { strength, radius } => Filter::halation(*strength, *radius)
                .map(FilterProp::Fixed)
                .map_err(|error| {
                    self.error(&format!("{path}.halation"), "parameters", error.message)
                })
                .ok(),
        }
    }

    fn triple(&mut self, values: [f64; 3], path: &str, field: &str) -> Option<[f32; 3]> {
        let narrowed = values.map(|value| value as f32);
        if narrowed.iter().all(|value| value.is_finite()) {
            Some(narrowed)
        } else {
            self.error(
                path,
                field,
                "Grade values must be finite in single precision",
            );
            None
        }
    }

    fn optional<T: super::value::Value>(
        &mut self,
        value: Option<&crate::value::Animated<T>>,
        path: &str,
    ) -> Option<Option<Prop<T::Engine>>> {
        match value {
            None => Some(None),
            Some(value) => self.animated(value, path).map(Some),
        }
    }
}

/// Compiled transforms, compositing and effects in their application order.
struct Modifiers {
    name: Option<Arc<str>>,
    position: Option<Prop<[f64; 2]>>,
    translate: Option<Prop<[f64; 2]>>,
    rotate: Option<Prop<f64>>,
    scale: Option<Prop<f64>>,
    scale_xy: Option<Prop<[f64; 2]>>,
    pivot: Option<Pivot>,
    opacity: Option<Prop<f64>>,
    blend: Option<BlendMode>,
    z_index: Option<i32>,
    filters: Vec<FilterProp>,
    clip: Option<(ClipProp, ClipMode)>,
    mask: Option<(Built, MaskMode)>,
}

impl Modifiers {
    fn varying(&self) -> bool {
        self.position.as_ref().is_some_and(Prop::varying)
            || self.translate.as_ref().is_some_and(Prop::varying)
            || self.rotate.as_ref().is_some_and(Prop::varying)
            || self.scale.as_ref().is_some_and(Prop::varying)
            || self.scale_xy.as_ref().is_some_and(Prop::varying)
            || self.opacity.as_ref().is_some_and(Prop::varying)
            || self.filters.iter().any(FilterProp::varying)
            || self.clip.as_ref().is_some_and(|(clip, _)| clip.varying())
            || self.mask.as_ref().is_some_and(|(mask, _)| mask.varying())
    }

    fn apply(&self, node: Node, context: &SceneContext<'_>) -> Node {
        let time = context.time();
        let mut node = node;
        if let Some(pivot) = self.pivot {
            node = node.pivot(pivot);
        }
        if let Some(position) = &self.position {
            node = node.at(point(position.at(time)));
        }
        if let Some(scale) = &self.scale {
            node = node.scale(scale.at(time));
        }
        if let Some(scale) = &self.scale_xy {
            let [horizontal, vertical] = scale.at(time);
            node = node.scale_xy(horizontal, vertical);
        }
        if let Some(rotate) = &self.rotate {
            node = node.rotate(rotate.at(time).to_radians());
        }
        if let Some(translate) = &self.translate {
            let [horizontal, vertical] = translate.at(time);
            node = node.translate(horizontal, vertical);
        }
        for filter in &self.filters {
            node = node.filter(filter.at(time));
        }
        if let Some((clip, mode)) = &self.clip {
            node = clip.apply(node, *mode, time);
        }
        if let Some((mask, mode)) = &self.mask {
            node = node.mask(mask.at(context), *mode);
        }
        if let Some(opacity) = &self.opacity {
            node = node.opacity(opacity.at(time));
        }
        if let Some(blend) = self.blend {
            node = node.blend_mode(blend);
        }
        if let Some(index) = self.z_index {
            node = node.z_index(index);
        }
        if let Some(name) = &self.name {
            node = node.named(name);
        }
        node
    }
}

enum FilterProp {
    Blur(Prop<f64>),
    DropShadow(Prop<[f64; 2]>, Prop<f64>, Prop<Color>),
    Glow(Prop<f64>, Prop<f64>, Prop<f64>),
    ColorAdjust([Prop<f64>; 4]),
    Fixed(Filter),
}

impl FilterProp {
    fn varying(&self) -> bool {
        match self {
            Self::Blur(sigma) => sigma.varying(),
            Self::DropShadow(offset, sigma, color) => {
                offset.varying() || sigma.varying() || color.varying()
            }
            Self::Glow(threshold, sigma, intensity) => {
                threshold.varying() || sigma.varying() || intensity.varying()
            }
            Self::ColorAdjust(values) => values.iter().any(Prop::varying),
            Self::Fixed(_) => false,
        }
    }

    fn at(&self, time: Time) -> Filter {
        match self {
            Self::Blur(sigma) => Filter::Blur {
                sigma: sigma.at(time),
            },
            Self::DropShadow(offset, sigma, color) => {
                let [x, y] = offset.at(time);
                Filter::DropShadow {
                    offset: Vector2::new(x, y),
                    sigma: sigma.at(time),
                    color: color.at(time),
                }
            }
            Self::Glow(threshold, sigma, intensity) => Filter::Glow {
                threshold: threshold.at(time),
                sigma: sigma.at(time),
                intensity: intensity.at(time),
            },
            Self::ColorAdjust([exposure, contrast, saturation, hue]) => Filter::ColorAdjust {
                exposure: exposure.at(time),
                contrast: contrast.at(time),
                saturation: saturation.at(time),
                hue_degrees: hue.at(time),
            },
            Self::Fixed(filter) => filter.clone(),
        }
    }
}

enum ClipProp {
    Rect(Prop<[f64; 4]>),
    Circle(Prop<[f64; 2]>, Prop<f64>),
    Path(Path2D),
}

impl ClipProp {
    fn varying(&self) -> bool {
        match self {
            Self::Rect(bounds) => bounds.varying(),
            Self::Circle(center, radius) => center.varying() || radius.varying(),
            Self::Path(_) => false,
        }
    }

    fn apply(&self, node: Node, mode: ClipMode, time: Time) -> Node {
        match self {
            Self::Rect(bounds) => {
                let [left, top, right, bottom] = bounds.at(time);
                node.clip_with(Rect::new(left, top, right, bottom), mode)
            }
            Self::Circle(center, radius) => {
                match Circle::new(point(center.at(time)), radius.at(time)) {
                    Ok(circle) => node.clip_with(circle, mode),
                    Err(error) => super::failure::failure(error),
                }
            }
            Self::Path(path) => node.clip_with(path.clone(), mode),
        }
    }
}
