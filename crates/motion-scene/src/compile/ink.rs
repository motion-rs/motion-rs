//! Compilation of bounded ink recipes with retained full-stroke material and explicit clocks.

use super::Compiler;
use super::element::Built;
use super::style::PaintProp;
#[cfg(feature = "ink")]
use super::value::Lift;
use crate::{BrushStrokeSpec, InkFillSpec, PigmentSpec};
#[cfg(feature = "ink")]
use crate::{BrushTimingSpec, InkSpec, PaperSpec};
#[cfg(feature = "ink")]
use motion_content::vector::ink::{
    BrushStroke, BrushStrokeBuilder, BrushTiming, Ink, InputSample, Paper,
};
#[cfg(feature = "ink")]
use motion_core::{Duration, Node, Paint, Path2D, Time, Transform, ValidationError};
#[cfg(feature = "ink")]
use std::sync::Arc;

/// Largest accepted pen, pressure or timing control list.
#[cfg(feature = "ink")]
const MAX_INK_SAMPLES: usize = 512;

/// Largest accepted recording, reveal or material age in seconds.
#[cfg(feature = "ink")]
const MAX_INK_SECONDS: f64 = 86400.0;

/// Largest accepted generated paper texture side, in texels.
#[cfg(feature = "ink")]
const MAX_PAPER_SIDE: u32 = 512;

impl Compiler<'_> {
    #[cfg(feature = "ink")]
    pub(super) fn brush_stroke(&mut self, spec: &BrushStrokeSpec, path: &str) -> Option<Built> {
        if spec.samples.len() > MAX_INK_SAMPLES || spec.pressure_curve.len() > MAX_INK_SAMPLES {
            self.limit(
                path,
                "samples",
                "Pen samples and pressure controls take at most 512 entries",
            );
            return None;
        }
        let sources = usize::from(spec.path.is_some())
            + usize::from(spec.points.is_some())
            + usize::from(!spec.samples.is_empty());
        if sources != 1 {
            self.error(path, "path", "Give exactly one of path, points or samples");
            return None;
        }
        if usize::from(spec.pressure.is_some())
            + usize::from(!spec.pressure_curve.is_empty())
            + usize::from(spec.pressure_from.is_some())
            > 1
        {
            self.error(path, "pressure", "Give at most one pressure treatment");
            return None;
        }
        if spec.write.is_some() && spec.write_time.is_some()
            || spec.timing.is_some() && spec.write_time.is_none()
            || spec.write_time.is_some() && spec.timing.is_none() && spec.samples.is_empty()
        {
            self.error(
                path,
                "write_time",
                "Choose write or write_time; write_time requires recorded samples or timing",
            );
            return None;
        }
        let paint = self.paint(&spec.paint, &format!("{path}.paint"))?;
        let initial_paint = self.ink_checked(path, paint.at(Time::ZERO))?;
        let mut builder = if let Some(data) = &spec.path {
            BrushStroke::builder(
                self.svg_path(data, &format!("{path}.path"))?,
                spec.width,
                initial_paint,
            )
        } else if let Some(points) = &spec.points {
            if points.animated.is_some() {
                self.error(path, "points", "Brush points must be fixed at compilation");
                return None;
            }
            let points = self.point_source(points, &format!("{path}.points"))?;
            if points.len() < 2 {
                self.error(path, "points", "Brush paths require at least two points");
                return None;
            }
            let mut geometry = Path2D::new();
            for (index, point) in points.into_iter().enumerate() {
                let result = if index == 0 {
                    geometry.try_move_to(point)
                } else {
                    geometry.try_line_to(point)
                };
                self.ink_checked(path, result)?;
            }
            BrushStroke::builder(geometry, spec.width, initial_paint)
        } else {
            let samples = spec
                .samples
                .iter()
                .enumerate()
                .map(|(index, sample)| {
                    let time = self.bounded(
                        sample.time,
                        0.0,
                        MAX_INK_SECONDS,
                        path,
                        &format!("samples[{index}].time"),
                    )?;
                    Some(InputSample {
                        position: sample.position,
                        pressure: sample.pressure,
                        tilt: sample.tilt.to_radians(),
                        time: Time::from_seconds(time),
                    })
                })
                .collect::<Option<Vec<_>>>()?;
            self.ink_checked(
                path,
                BrushStrokeBuilder::from_samples(&samples, spec.width, initial_paint),
            )?
        };
        if let Some([start, middle, end]) = spec.pressure {
            builder = builder.pressure(start, middle, end);
        }
        if !spec.pressure_curve.is_empty() {
            builder = builder.pressure_curve(spec.pressure_curve.clone());
        }
        if let Some(source) = &spec.pressure_from {
            let parsed = super::expr::parse(source, self.params)
                .map_err(|message| self.error(path, "pressure_from", message))
                .ok()?;
            if !parsed.references.is_empty() {
                self.error(
                    path,
                    "pressure_from",
                    "Arc-pressure expressions cannot bind scene elements",
                );
                return None;
            }
            for index in 0..=MAX_INK_SAMPLES {
                self.bounded(
                    parsed.expr.eval(index as f64 / MAX_INK_SAMPLES as f64, &[]),
                    0.0,
                    4.0,
                    path,
                    "pressure_from",
                )?;
            }
            builder = builder.pressure_from(move |progress| parsed.expr.eval(progress, &[]));
        }
        builder = builder.nib(spec.nib_aspect.unwrap_or(1.0), 0.0);
        if let Some(mode) = spec.nib_mode {
            use motion_content::NibMode;
            builder = builder.nib_mode(match mode {
                NibMode::Fixed(degrees) => NibMode::Fixed(degrees.to_radians()),
                NibMode::FollowPath(degrees) => NibMode::FollowPath(degrees.to_radians()),
                NibMode::Tilt => NibMode::Tilt,
            });
        }
        if let Some([start, end]) = spec.caps {
            builder = builder.caps(start, end);
        }
        if let Some([start, end]) = spec.ink_load {
            builder = builder.ink_load(start, end);
        }
        if let Some(ink) = &spec.ink {
            builder = builder.ink(self.ink_material(ink, &format!("{path}.ink"))?);
        }
        if let Some(bristles) = &spec.bristles {
            builder = builder.bristles(
                bristles.count,
                bristles.spread,
                bristles.jitter,
                bristles.seed,
            );
        }
        if let Some(limit) = spec.sample_limit {
            builder = builder.sample_limit(limit);
        }
        if let Some(limit) = spec.material_work_limit {
            builder = builder.material_work_limit(limit);
        }
        if let Some(response) = spec.speed_response {
            builder = builder.speed_response(response);
        }
        let wobble = spec
            .wobble
            .as_ref()
            .map(|spec| {
                builder = builder
                    .clone()
                    .wobble(spec.amplitude, spec.frequency, spec.seed);
                Some((
                    spec.clone(),
                    self.optional(spec.frame.as_ref(), &format!("{path}.wobble.frame"))?,
                ))
            })
            .lift()?;
        let wetness = spec
            .wetness
            .as_ref()
            .map(|spec| {
                self.bounded(
                    spec.drying,
                    f64::MIN_POSITIVE,
                    MAX_INK_SECONDS,
                    path,
                    "wetness.drying",
                )?;
                Some((
                    spec.clone(),
                    self.animated(&spec.age, &format!("{path}.wetness.age"))?,
                ))
            })
            .lift()?;
        let write = self.optional(spec.write.as_ref(), &format!("{path}.write"))?;
        let write_time = self.optional(spec.write_time.as_ref(), &format!("{path}.write_time"))?;
        let timing = spec
            .timing
            .as_ref()
            .map(|spec| self.ink_timing(spec, path))
            .lift()?;
        let material_varying = paint.varying()
            || wobble
                .as_ref()
                .and_then(|(_, frame)| frame.as_ref())
                .is_some_and(super::value::Prop::varying)
            || wetness.as_ref().is_some_and(|(_, age)| age.varying());
        let build = move |time| {
            let mut current = builder.clone().paint(paint.at(time)?);
            if let Some((spec, Some(frame))) = &wobble {
                let frame =
                    checked_number(frame.at(time), 0.0, u32::MAX as f64 * 2.0, "wobble.frame")?;
                current = current.boil(
                    spec.amplitude,
                    spec.frequency,
                    spec.seed,
                    frame.floor() as u64,
                );
            }
            if let Some((spec, age)) = &wetness {
                let age = checked_number(age.at(time), 0.0, MAX_INK_SECONDS, "wetness.age")?;
                current = current.try_wetness(
                    Duration::from_seconds(age),
                    spec.radius,
                    Duration::from_seconds(spec.drying),
                )?;
            }
            current.build()
        };
        let initial = self.ink_checked(path, build(Time::ZERO))?;
        let reveal = move |stroke: BrushStroke, time| {
            if let Some(write) = &write {
                Ok(stroke.write(checked_number(write.at(time), 0.0, 1.0, "write")?))
            } else if let Some(source) = &write_time {
                let time = Time::from_seconds(checked_number(
                    source.at(time),
                    0.0,
                    MAX_INK_SECONDS,
                    "write_time",
                )?);
                match &timing {
                    Some(timing) => stroke.write_timed(time, timing),
                    None => stroke.write_with(time),
                }
            } else {
                Ok(stroke)
            }
        };
        self.ink_checked(path, reveal(initial.clone(), Time::ZERO))?;
        let varying = material_varying
            || spec
                .write
                .as_ref()
                .is_some_and(|value| !matches!(value, crate::Animated::Fixed(_)))
            || spec
                .write_time
                .as_ref()
                .is_some_and(|value| !matches!(value, crate::Animated::Fixed(_)));
        let path = path.to_owned();
        Some(Built::new(varying, move |time| {
            let stroke = if material_varying {
                build(time)
            } else {
                Ok(initial.clone())
            };
            stroke
                .and_then(|stroke| reveal(stroke, time))
                .map(Node::from)
                .unwrap_or_else(|mut error| {
                    error.path = path.clone();
                    super::failure::failure(error)
                })
        }))
    }

    #[cfg(not(feature = "ink"))]
    pub(super) fn brush_stroke(&mut self, _: &BrushStrokeSpec, path: &str) -> Option<Built> {
        self.unsupported(path, "Brush strokes require feature `ink`");
        None
    }

    #[cfg(feature = "ink")]
    pub(super) fn ink_fill(&mut self, spec: &InkFillSpec, path: &str) -> Option<Built> {
        let geometry = self.svg_path(&spec.path, &format!("{path}.path"))?;
        let paint = self.paint(&spec.paint, &format!("{path}.paint"))?;
        let ink = self.ink_material(&spec.ink, &format!("{path}.ink"))?;
        let varying = paint.varying();
        let path = path.to_owned();
        Some(Built::new(varying, move |time| {
            paint
                .at(time)
                .map(|paint| {
                    Node::from(motion_content::vector::ink::InkFill::new(
                        geometry.clone(),
                        paint,
                        ink.clone(),
                    ))
                })
                .unwrap_or_else(|mut error| {
                    error.path = path.clone();
                    super::failure::failure(error)
                })
        }))
    }

    #[cfg(not(feature = "ink"))]
    pub(super) fn ink_fill(&mut self, _: &InkFillSpec, path: &str) -> Option<Built> {
        self.unsupported(path, "Ink fills require feature `ink`");
        None
    }

    #[cfg(feature = "ink")]
    pub(super) fn pigment(&mut self, spec: &PigmentSpec, path: &str) -> Option<PaintProp> {
        let amount = self.animated(&spec.amount, &format!("{path}.amount"))?;
        let (first, second, mode) = (spec.first, spec.second, spec.mode);
        let initial = self.ink_checked(path, mode.mix(first, second, amount.at(Time::ZERO)))?;
        if !amount.varying() {
            return Some(PaintProp::Fixed(Paint::Solid(initial)));
        }
        let path = path.to_owned();
        Some(PaintProp::Sampled(Arc::new(move |time| {
            mode.mix(first, second, amount.at(time))
                .map(Paint::Solid)
                .map_err(|mut error| {
                    error.path = path.clone();
                    error
                })
        })))
    }

    #[cfg(not(feature = "ink"))]
    pub(super) fn pigment(&mut self, _: &PigmentSpec, path: &str) -> Option<PaintProp> {
        self.unsupported(path, "Pigment mixing requires feature `ink`");
        None
    }

    #[cfg(feature = "ink")]
    fn ink_material(&mut self, spec: &InkSpec, path: &str) -> Option<Ink> {
        let mut ink = Ink::new(spec.seed)
            .dryness(spec.dryness)
            .bleed(spec.bleed)
            .edge_roughness(spec.edge_roughness)
            .granulation(spec.granulation)
            .fibers(spec.fibers)
            .build_up(spec.build_up)
            .edge_darkening(spec.edge_darkening)
            .edge_anisotropy(spec.edge_anisotropy.unwrap_or(1.0))
            .height_map(spec.height_map);
        if let Some(coefficients) = spec.paper_transform {
            ink = ink.paper_transform(Transform::from_coefficients(coefficients));
        }
        if let Some([strength, width]) = spec.edge_pooling {
            ink = ink.edge_pooling(strength, width);
        }
        if let Some(paper) = &spec.paper {
            ink = ink.paper(match paper {
                PaperSpec::Procedural {
                    seed,
                    strength,
                    scale,
                } => Paper::Procedural {
                    seed: *seed,
                    strength: *strength,
                    scale: *scale,
                },
                PaperSpec::Texture {
                    image,
                    grain_scale,
                    absorbency,
                } => Paper::Texture {
                    texture: self.inline_image(image, &format!("{path}.paper.texture.image"))?,
                    grain_scale: *grain_scale,
                    absorbency: *absorbency,
                },
                PaperSpec::NoiseTexture {
                    size: [width, height],
                    seed,
                    grain_scale,
                    absorbency,
                } => {
                    if !(1..=MAX_PAPER_SIDE).contains(width)
                        || !(1..=MAX_PAPER_SIDE).contains(height)
                    {
                        self.limit(
                            path,
                            "paper.noise_texture.size",
                            "Noise texture sides must be in 1..=512",
                        );
                        return None;
                    }
                    let mut rgba = Vec::with_capacity(*width as usize * *height as usize * 4);
                    for row in 0..*height {
                        for column in 0..*width {
                            let value =
                                (motion_core::noise::hash2(column as i32, row as i32, *seed) & 255)
                                    as u8;
                            rgba.extend_from_slice(&[value, value, value, 255]);
                        }
                    }
                    Paper::Texture {
                        texture: Arc::new(
                            motion_content::ImageAsset::from_rgba8(*width, *height, rgba)
                                .map_err(|error| {
                                    self.error(path, "paper.noise_texture", error.to_string())
                                })
                                .ok()?,
                        ),
                        grain_scale: *grain_scale,
                        absorbency: *absorbency,
                    }
                }
            });
        }
        // The owning brush validator checks the shared material without baking texels.
        self.ink_checked(
            path,
            BrushStroke::builder(Path2D::new(), 1.0, motion_core::Color::BLACK)
                .ink(ink.clone())
                .build(),
        )?;
        Some(ink)
    }

    #[cfg(feature = "ink")]
    fn ink_timing(&mut self, spec: &BrushTimingSpec, path: &str) -> Option<BrushTiming> {
        let duration = self.bounded(
            spec.duration,
            f64::MIN_POSITIVE,
            MAX_INK_SECONDS,
            path,
            "timing.duration",
        )?;
        let pause = self.bounded(
            spec.lift_pause,
            0.0,
            MAX_INK_SECONDS,
            path,
            "timing.lift_pause",
        )?;
        if spec.curve.len() > MAX_INK_SAMPLES {
            self.limit(
                path,
                "timing.curve",
                "Timing controls take at most 512 entries",
            );
            return None;
        }
        let mut timing = BrushTiming::new(Duration::from_seconds(duration));
        if !spec.curve.is_empty() {
            timing.curve = spec.curve.clone();
        }
        timing.lift_pause = Duration::from_seconds(pause);
        Some(timing)
    }

    #[cfg(feature = "ink")]
    fn ink_checked<T>(&mut self, path: &str, result: Result<T, ValidationError>) -> Option<T> {
        result
            .map_err(|error| self.error(path, &error.field, error.message))
            .ok()
    }
}

#[cfg(feature = "ink")]
fn checked_number(
    value: f64,
    minimum: f64,
    maximum: f64,
    field: &str,
) -> Result<f64, ValidationError> {
    if value.is_finite() && (minimum..=maximum).contains(&value) {
        Ok(value)
    } else {
        Err(ValidationError::invalid(
            "brush_stroke",
            field,
            format!("Value must be finite and in {minimum}..={maximum}"),
        ))
    }
}
