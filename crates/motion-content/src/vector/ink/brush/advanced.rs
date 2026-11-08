//! Advanced brush strokes: nib modes, recorded input, extended options, validation and drawing.

use super::{BrushStroke, invalid};
use motion_core::{
    Canvas, Duration, FillRule, NodeModifiers, Paint, Point, Rect, Shape, Time, Transform,
    ValidationError, Vector2,
};
use std::sync::Arc;

mod geometry;
mod material;
mod pigment;
mod presets;
#[cfg(test)]
mod tests;
mod timing;

pub use material::Paper;
pub use pigment::{PigmentMix, PigmentPainting};
pub use presets::BrushPreset;
pub use timing::BrushTiming;

use geometry::{outline, samples};
use material::material;

/// Per-sample nib orientation for explicitly extended strokes.
#[derive(Clone, Copy, Debug)]
pub enum NibMode {
    /// Fixed clockwise radians.
    Fixed(f64),
    /// Tangent angle plus a fixed offset.
    FollowPath(f64),
    /// Recorded nib azimuth, interpolated over the input samples.
    Tilt,
}

/// Analytic pressure treatment at the stroke endpoints.
#[derive(Clone, Copy, Debug)]
pub enum BrushCap {
    /// Full rounded nib imprint.
    Round,
    /// Clips the nib and connector geometry at an endpoint tangent plane.
    Flat,
    /// Smoothly tapers over the specified arc length.
    Tapered(f64),
}

/// One recorded pen sample. Time is relative to the first input sample.
#[derive(Clone, Copy, Debug)]
pub struct InputSample {
    /// Finite local position.
    pub position: Point,
    /// Normalized pressure in 0..=4.
    pub pressure: f64,
    /// Nib azimuth in clockwise radians, not an altitude angle.
    pub tilt: f64,
    /// Strictly increasing nonnegative recording time.
    pub time: Time,
}

#[derive(Clone)]
struct PressureFunction(Arc<dyn Fn(f64) -> f64 + Send + Sync>);

impl std::fmt::Debug for PressureFunction {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("PressureFunction")
            .finish_non_exhaustive()
    }
}

#[derive(Clone, Debug)]
pub(super) struct Options {
    pressure: Vec<(f64, f64)>,
    function: Option<PressureFunction>,
    nib: NibMode,
    caps: [BrushCap; 2],
    samples: usize,
    input: Vec<InputSample>,
    input_arcs: Vec<f64>,
    speed_response: f64,
    wobble: Option<(f64, f64, u32)>,
    paint: Option<Paint>,
    bristles: usize,
    spread: f64,
    jitter: f64,
    seed: u32,
    tile_size: u32,
    parallel: bool,
    paper: Option<Paper>,
    build_up: f64,
    edge_darkening: f64,
    anisotropy: f64,
    height: f64,
    paper_transform: Transform,
}

impl Default for Options {
    fn default() -> Self {
        Self {
            pressure: Vec::new(),
            function: None,
            nib: NibMode::Fixed(0.0),
            caps: [BrushCap::Round; 2],
            samples: 256,
            input: Vec::new(),
            input_arcs: Vec::new(),
            speed_response: 0.0,
            wobble: None,
            paint: None,
            bristles: 1,
            spread: 0.0,
            jitter: 0.0,
            seed: 0,
            tile_size: 2048,
            parallel: false,
            paper: None,
            build_up: 0.0,
            edge_darkening: 0.0,
            anisotropy: 1.0,
            height: 0.0,
            paper_transform: Transform::IDENTITY,
        }
    }
}

impl Options {
    pub(super) fn reset_pressure(&mut self) {
        self.pressure.clear();
        self.function = None;
    }

    pub(super) fn set_fixed_angle(&mut self, angle: f64) {
        if matches!(self.nib, NibMode::Fixed(_)) {
            self.nib = NibMode::Fixed(angle);
        }
    }
}

impl BrushStroke {
    fn configure(mut self, configure: impl FnOnce(&mut Options)) -> Self {
        configure(&mut self.advanced);
        self.reset();
        self
    }

    /// Sets endpoint-inclusive ordered pressure control points, interpolated smoothly.
    pub fn pressure_curve(self, curve: Vec<(f64, f64)>) -> Self {
        self.configure(|options| {
            options.pressure = curve;
            options.function = None;
        })
    }

    /// Sets a pure, finite pressure function in 0..=4, evaluated at bounded arc samples.
    pub fn pressure_from(self, function: impl Fn(f64) -> f64 + Send + Sync + 'static) -> Self {
        self.configure(|options| {
            options.pressure.clear();
            options.function = Some(PressureFunction(Arc::new(function)));
        })
    }

    /// Sets fixed, tangent-following or recorded nib orientation.
    pub fn nib_mode(self, mode: NibMode) -> Self {
        self.configure(|options| options.nib = mode)
    }

    /// Sets analytic start/end cap pressure treatment.
    pub fn caps(self, start: BrushCap, end: BrushCap) -> Self {
        self.configure(|options| options.caps = [start, end])
    }

    /// Maximum samples per path; at most 512 and global outline verbs remain bounded.
    pub fn sample_limit(self, samples: usize) -> Self {
        self.configure(|options| options.samples = samples)
    }

    /// Multiplies pressure by 1/(1+response*speed); unrecorded paths use unit-duration speed.
    pub fn speed_response(self, response: f64) -> Self {
        self.configure(|options| options.speed_response = response)
    }

    /// Replaces solid stroke color with a spatial Paint, before material attenuation.
    pub fn paint(self, paint: impl Into<Paint>) -> Self {
        self.configure(|options| options.paint = Some(paint.into()))
    }

    /// Splits the stroke into 1..=64 seeded offset bristles.
    pub fn bristles(self, count: usize, spread: f64, jitter: f64, seed: u32) -> Self {
        self.configure(|options| {
            options.bristles = count;
            options.spread = spread;
            options.jitter = jitter;
            options.seed = seed;
        })
    }

    /// Sets the material tile edge in 16..=2048 texels; the default bakes most strokes as one tile.
    pub fn tiled_material(self, tile_size: u32) -> Self {
        self.configure(|options| options.tile_size = tile_size)
    }

    /// Evaluates independent texel rows in scoped CPU threads, with identical arithmetic.
    pub fn parallel_material(self, enabled: bool) -> Self {
        self.configure(|options| options.parallel = enabled)
    }

    /// Samples a resolved local-coordinate paper height map.
    pub fn paper_texture(self, paper: Paper) -> Self {
        self.configure(|options| options.paper = Some(paper))
    }

    /// Supplies the authored local-to-canvas paper coordinates; identity uses untransformed design space.
    /// Keep this equal to the enclosing node transform when paper must stay fixed on the canvas.
    pub fn paper_transform(self, transform: Transform) -> Self {
        self.configure(|options| options.paper_transform = transform)
    }

    /// Adds density at self-intersections using independent segment coverage.
    pub fn build_up(self, strength: f64) -> Self {
        self.configure(|options| options.build_up = strength)
    }

    /// Darkens RGB near the geometric rim, in 0..=1.
    pub fn edge_darkening(self, strength: f64) -> Self {
        self.configure(|options| options.edge_darkening = strength)
    }

    /// Scales directional material coordinates; a positive ratio in 0.1..=10.
    pub fn edge_anisotropy(self, ratio: f64) -> Self {
        self.configure(|options| options.anisotropy = ratio)
    }

    /// Produces a grayscale rounded-ridge height drawable for `Optics::Relight { height_from:Luma }`.
    pub fn height_map(self, strength: f64) -> Self {
        self.configure(|options| options.height = strength)
    }

    /// Seeded analytic path displacement. Reuse seed for stable texture; change it for boil.
    pub fn wobble(self, amplitude: f64, frequency: f64, seed: u32) -> Self {
        self.configure(|options| options.wobble = Some((amplitude, frequency, seed)))
    }

    /// Evaluates the reveal endpoint and tangent, suitable for a user-authored cursor or emitter.
    pub fn tip(&self) -> Result<(Point, Vector2), ValidationError> {
        let options = &self.advanced;
        if self.progress > 0.0 {
            validate(self)?;
            let groups = samples(self, self.progress)?;
            let tips: Vec<_> = groups
                .iter()
                .rev()
                .take(options.bristles)
                .filter_map(|group| group.last())
                .collect();
            if let Some(sample) = tips.first() {
                let mut point = Point::new(0.0, 0.0);
                for tip in &tips {
                    point.x += tip.point.x / tips.len() as f64;
                    point.y += tip.point.y / tips.len() as f64;
                }
                return Ok((point, sample.tangent));
            }
        }
        self.path.try_point_at(self.progress)
    }

    /// Positions a user-authored cursor at the deterministic reveal endpoint.
    pub fn tip_node(
        &self,
        node: impl Into<motion_core::Node>,
    ) -> Result<motion_core::Node, ValidationError> {
        let (point, tangent) = self.tip()?;
        Ok(node
            .into()
            .pivot(motion_core::Pivot::Point(Point::new(0.0, 0.0)))
            .rotate(libm::atan2(tangent.y, tangent.x))
            .translate(point.x, point.y))
    }

    /// Uses this stroke as an inverted-alpha erase mask; no backend blend changes.
    pub fn erase(self, content: impl Into<motion_core::Node>) -> motion_core::Node {
        content.into().mask(
            motion_core::Node::from(self),
            motion_core::MaskMode::InvertedAlpha,
        )
    }

    /// Seeded two-frame boil; frame order never contributes history.
    pub fn boil(self, amplitude: f64, frequency: f64, seed: u32, frame: u64) -> Self {
        self.wobble(
            amplitude,
            frequency,
            seed ^ motion_core::noise::hash32((frame / 2) as u32),
        )
    }

    /// Analytic absorption: the material depends on age, not previous frames.
    pub fn wetness(
        self,
        age: Duration,
        radius: f32,
        drying: Duration,
    ) -> Result<Self, ValidationError> {
        if age < Time::ZERO || drying <= Time::ZERO || !radius.is_finite() || radius < 0.0 {
            return Err(invalid(
                "wetness",
                "Invalid age, wash radius or drying duration",
            ));
        }
        let extent = f64::from(radius) * (1.0 - libm::exp(-age.as_seconds() / drying.as_seconds()));
        let ink = self.ink.unwrap_or_default().bleed(extent as f32);
        Ok(self.ink(ink))
    }
}

fn validate_input(samples: &[InputSample]) -> Result<(), ValidationError> {
    if !(2..=512).contains(&samples.len())
        || samples
            .first()
            .is_some_and(|sample| sample.time != Time::ZERO)
        || samples.iter().any(|sample| {
            !sample.position.x.is_finite()
                || !sample.position.y.is_finite()
                || !sample.pressure.is_finite()
                || !(0.0..=4.0).contains(&sample.pressure)
                || !sample.tilt.is_finite()
                || sample.time < Time::ZERO
                || sample.time.as_seconds() > 86400.0
        })
        || samples.windows(2).any(|pair| pair[0].time >= pair[1].time)
    {
        return Err(invalid(
            "input",
            "Require 2..=512 finite pen samples with strictly increasing time starting at zero",
        ));
    }
    Ok(())
}

fn validate(stroke: &BrushStroke) -> Result<(), ValidationError> {
    let options = &stroke.advanced;
    if !options.height.is_finite() || !(0.0..=16.0).contains(&options.height) {
        return Err(invalid("height", "Require finite height in 0..=16"));
    }
    if !options.paper_transform.is_finite() {
        return Err(invalid(
            "paper_transform",
            "Paper coordinates must be finite",
        ));
    }
    stroke.path.validate()?;
    if !stroke.width.is_finite()
        || !(0.0..=512.0).contains(&stroke.width)
        || !stroke.aspect.is_finite()
        || !(f64::MIN_POSITIVE..=1.0).contains(&stroke.aspect)
        || !stroke.angle.is_finite()
        || !stroke.progress.is_finite()
        || !(0.0..=1.0).contains(&stroke.progress)
        || stroke.material_work_limit == 0
        || stroke
            .pressure
            .iter()
            .any(|value| !value.is_finite() || !(0.0..=4.0).contains(value))
        || stroke
            .load
            .iter()
            .any(|value| !value.is_finite() || !(0.0..=1.0).contains(value))
        || !(2..=512).contains(&options.samples)
        || !(1..=64).contains(&options.bristles)
        || !(16..=2048).contains(&options.tile_size)
        || !options.spread.is_finite()
        || !(0.0..=512.0).contains(&options.spread)
        || !options.jitter.is_finite()
        || !(0.0..=1.0).contains(&options.jitter)
        || !options.speed_response.is_finite()
        || !(0.0..=1.0).contains(&options.speed_response)
        || !options.build_up.is_finite()
        || !(0.0..=1.0).contains(&options.build_up)
        || !options.edge_darkening.is_finite()
        || !(0.0..=1.0).contains(&options.edge_darkening)
        || !options.anisotropy.is_finite()
        || !(0.1..=10.0).contains(&options.anisotropy)
    {
        return Err(invalid(
            "extended",
            "Invalid bounded geometry or material configuration",
        ));
    }
    if !options.pressure.is_empty()
        && (options.pressure.len() > 512
            || options.pressure.first().unwrap().0 != 0.0
            || options.pressure.last().unwrap().0 != 1.0
            || options.pressure.iter().any(|(time, pressure)| {
                !time.is_finite()
                    || !(0.0..=1.0).contains(time)
                    || !pressure.is_finite()
                    || !(0.0..=4.0).contains(pressure)
            })
            || options
                .pressure
                .windows(2)
                .any(|pair| pair[0].0 >= pair[1].0))
    {
        return Err(invalid(
            "pressure_curve",
            "Require ordered finite controls spanning 0..=1",
        ));
    }
    if !options.input.is_empty() {
        validate_input(&options.input)?;
    }
    match options.nib {
        NibMode::Fixed(angle) | NibMode::FollowPath(angle) if !angle.is_finite() => {
            return Err(invalid("nib", "Nib angle must be finite"));
        }
        NibMode::Tilt if options.input.is_empty() => {
            return Err(invalid("nib", "Tilt requires recorded input"));
        }
        _ => {}
    }
    if options
        .caps
        .iter()
        .any(|cap| matches!(cap,BrushCap::Tapered(length) if !length.is_finite() || *length<=0.0))
    {
        return Err(invalid("caps", "Taper length must be positive and finite"));
    }
    if options.wobble.is_some_and(|(amount, frequency, _)| {
        !amount.is_finite()
            || !(0.0..=512.0).contains(&amount)
            || !frequency.is_finite()
            || !(f64::MIN_POSITIVE..=100.0).contains(&frequency)
    }) {
        return Err(invalid("wobble", "Invalid wobble amplitude or frequency"));
    }
    if options.paper.as_ref().is_some_and(|paper| {
        !paper.grain_scale.is_finite()
            || paper.grain_scale <= 0.0
            || !paper.absorbency.is_finite()
            || !(0.0..=1.0).contains(&paper.absorbency)
    }) {
        return Err(invalid("paper", "Invalid paper scale or absorbency"));
    }
    if let Some(paint) = &options.paint {
        paint.validate()?;
    }
    if let Some(ink) = stroke.ink {
        ink.padding()
            .map_err(|error| invalid("ink", &error.to_string()))?;
    }
    Ok(())
}

pub(super) fn bounds(stroke: &BrushStroke) -> Result<Rect, ValidationError> {
    validate(stroke)?;
    let options = &stroke.advanced;
    let padding = stroke
        .ink
        .map_or(Ok(0.0), |ink| ink.padding().map(f64::from))
        .map_err(|error| invalid("ink", &error.to_string()))?;
    Ok(stroke.path.bounding_box().inset(
        -(stroke.width * 2.0
            + padding
            + options.spread
            + options.wobble.map_or(0.0, |value| value.0)
            + 1.0),
    ))
}

pub(super) fn draw(stroke: &BrushStroke, canvas: &mut Canvas<'_>) -> Result<(), ValidationError> {
    let bounds = bounds(stroke)?;
    let groups = samples(stroke, stroke.progress)?;
    let path = Arc::new(outline(stroke, &groups)?);
    if stroke.progress == 0.0 || stroke.width == 0.0 {
        return Ok(());
    }
    let options = &stroke.advanced;
    let paint = options.paint.clone().unwrap_or_else(|| stroke.color.into());
    if stroke.ink.is_none()
        && options.paper.is_none()
        && options.build_up == 0.0
        && options.edge_darkening == 0.0
        && options.height == 0.0
    {
        return canvas.fill_path(path, Transform::IDENTITY, paint, FillRule::NonZero);
    }
    let full = stroke
        .full_material
        .get_or_init(|| material(stroke, bounds))
        .as_ref()
        .map_err(Clone::clone)?;
    for tile in full {
        canvas.fill_path(
            path.clone(),
            Transform::IDENTITY,
            tile.paint.clone(),
            FillRule::NonZero,
        )?;
    }
    Ok(())
}
