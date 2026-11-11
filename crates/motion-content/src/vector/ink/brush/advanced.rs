//! Advanced brush strokes: nib modes, recorded input, extended options, validation and drawing.

use super::{BrushStroke, BrushStrokeBuilder, Config, invalid};
use motion_core::{
    Canvas, Duration, FillRule, NodeModifiers, Point, Rect, Time, Transform, ValidationError,
    Vector2,
};
use std::sync::Arc;

mod geometry;
mod material;
mod pigment;
mod presets;
#[cfg(test)]
mod tests;
mod timing;

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

/// Analytic pressure treatment at the endpoints of a variable-width [`BrushStroke`].
///
/// This is not [`LineCap`](motion_core::LineCap): `LineCap` picks the end geometry (butt, round
/// or square) of a uniform-width [`StrokeStyle`](motion_core::StrokeStyle) outline, whereas
/// `BrushCap` shapes how the nib imprint and pressure end a brush stroke. `Round` resembles
/// `LineCap::Round`, `Flat` resembles `Butt`, and `Tapered` has no `LineCap` counterpart.
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
    bristles: usize,
    spread: f64,
    jitter: f64,
    seed: u32,
    tile_size: u32,
    parallel: bool,
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
            bristles: 1,
            spread: 0.0,
            jitter: 0.0,
            seed: 0,
            tile_size: 2048,
            parallel: false,
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

impl BrushStrokeBuilder {
    pub(super) fn configure(mut self, configure: impl FnOnce(&mut Options)) -> Self {
        configure(&mut self.config.advanced);
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

    /// Seeded analytic path displacement. Reuse seed for stable texture; change it for boil.
    pub fn wobble(self, amplitude: f64, frequency: f64, seed: u32) -> Self {
        self.configure(|options| options.wobble = Some((amplitude, frequency, seed)))
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
    pub fn try_wetness(
        self,
        age: Duration,
        radius: f32,
        drying: Duration,
    ) -> Result<Self, ValidationError> {
        if age < Duration::ZERO || drying <= Duration::ZERO || !radius.is_finite() || radius < 0.0 {
            return Err(invalid(
                "wetness",
                "Invalid age, wash radius or drying duration",
            ));
        }
        let extent = f64::from(radius) * (1.0 - libm::exp(-age.as_seconds() / drying.as_seconds()));
        let ink = self
            .config
            .ink
            .clone()
            .unwrap_or_default()
            .bleed(extent as f32);
        Ok(self.ink(ink))
    }
}

impl BrushStroke {
    /// Evaluates the reveal endpoint and tangent, suitable for a user-authored cursor or emitter.
    pub fn tip(&self) -> Result<(Point, Vector2), ValidationError> {
        let options = &self.config.advanced;
        if self.progress > 0.0 {
            self.validate()?;
            let groups = samples(&self.config, self.progress)?;
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
        self.config.path.try_point_at(self.progress)
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

    fn validate(&self) -> Result<(), ValidationError> {
        validate(&self.config)?;
        if !self.progress.is_finite() || !(0.0..=1.0).contains(&self.progress) {
            return Err(invalid("progress", "Require reveal progress in 0..=1"));
        }
        Ok(())
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

pub(super) fn validate(stroke: &Config) -> Result<(), ValidationError> {
    let options = &stroke.advanced;
    if !stroke.width.is_finite()
        || !(0.0..=512.0).contains(&stroke.width)
        || !stroke.aspect.is_finite()
        || !(f64::MIN_POSITIVE..=1.0).contains(&stroke.aspect)
        || !stroke.angle.is_finite()
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
    stroke.paint.validate()?;
    if let Some(ink) = &stroke.ink {
        ink.padding()
            .map_err(|error| invalid("ink", &error.to_string()))?;
    }
    Ok(())
}

pub(super) fn bounds(stroke: &BrushStroke) -> Result<Rect, ValidationError> {
    stroke.validate()?;
    let config = &stroke.config;
    let options = &config.advanced;
    let padding = config
        .ink
        .as_ref()
        .map_or(Ok(0.0), |ink| ink.padding().map(f64::from))
        .map_err(|error| invalid("ink", &error.to_string()))?;
    Ok(config.path.bounding_box().inset(
        -(config.width * 2.0
            + padding
            + options.spread
            + options.wobble.map_or(0.0, |value| value.0)
            + 1.0),
    ))
}

pub(super) fn draw(stroke: &BrushStroke, canvas: &mut Canvas<'_>) -> Result<(), ValidationError> {
    let bounds = bounds(stroke)?;
    let config = &stroke.config;
    let groups = samples(config, stroke.progress)?;
    let path = Arc::new(outline(config, &groups)?);
    if stroke.progress == 0.0 || config.width == 0.0 {
        return Ok(());
    }
    if config.ink.is_none() {
        return canvas.fill_path(
            path,
            Transform::IDENTITY,
            config.paint.clone(),
            FillRule::NonZero,
        );
    }
    let full = stroke
        .full_material
        .get_or_init(|| material(config, bounds))
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
