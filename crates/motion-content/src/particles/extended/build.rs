//! Conversion of particle states and style options into one validated extended instance batch.

use super::super::{
    BokehShape, MAX_INSTANCE_COUNT, Particle, ParticleShape, ParticleStyle, StyleOptions, invalid,
    limit,
};
use super::ExtendedBatch;
use super::sampling::support;
use motion_core::instances::{
    ExtendedInstanceRecord, InstanceBatch, InstanceBlend, InstanceKind, InstanceRecord,
};
use motion_core::{Node, SceneContext, Time, ValidationError};
use std::sync::Arc;

pub(crate) fn render(
    style: &ParticleStyle,
    options: &StyleOptions,
    context: &SceneContext<'_>,
    states: Arc<[Particle]>,
    mut sample: impl FnMut(Time) -> Result<Arc<[Particle]>, ValidationError>,
) -> Result<Node, ValidationError> {
    if options.parallax.iter().any(|value| !value.is_finite())
        || !(1..=64).contains(&options.trails.segments)
    {
        return Err(invalid("style", "Invalid parallax or trail segment count"));
    }
    if options.sheet.is_some() && !matches!(style, ParticleStyle::Sprites { .. }) {
        return Err(invalid("sheet", "Flipbook requires Sprites"));
    }
    if options.flip
        && !matches!(
            style,
            ParticleStyle::Shape {
                shape: ParticleShape::Quad { .. },
                ..
            }
        )
    {
        return Err(invalid("flip", "Confetti flip requires Quad"));
    }
    if let Some(stretch) = options.stretch
        && (!stretch.scale.is_finite()
            || stretch.scale < 0.0
            || !stretch.maximum.is_finite()
            || !(1.0..=64.0).contains(&stretch.maximum))
    {
        return Err(invalid("stretch", "Invalid velocity stretch"));
    }
    let mut sprite = None;
    let (radius, kind, extra, align) = match style {
        ParticleStyle::Dots { radius } => {
            (*radius, InstanceKind::Disc, [0.0, 0.0, 1.0, 0.0], false)
        }
        ParticleStyle::GlowDots { radius } => {
            (*radius, InstanceKind::Glow, [0.0, 0.0, 1.0, 0.0], false)
        }
        ParticleStyle::Sprites {
            image,
            size,
            align_to_velocity,
        } => {
            sprite = Some(
                image
                    .get()
                    .map_err(|_| invalid("sprite", "Configured sprite material is unavailable"))?,
            );
            (
                *size * 0.5,
                InstanceKind::Sprite,
                [0.0, 0.0, 1.0, 0.0],
                *align_to_velocity,
            )
        }
        ParticleStyle::Shape { radius, shape } => {
            let (kind, extra) = match shape {
                ParticleShape::Polygon { sides } if (3..=32).contains(sides) => {
                    (InstanceKind::Polygon, [f32::from(*sides), 0.0, 1.0, 0.0])
                }
                ParticleShape::Ring { thickness }
                    if thickness.is_finite() && *thickness > 0.0 && *thickness <= 1.0 =>
                {
                    (InstanceKind::Ring, [*thickness, 0.0, 1.0, 0.0])
                }
                ParticleShape::Capsule { aspect }
                    if aspect.is_finite() && (1.0..=64.0).contains(aspect) =>
                {
                    (InstanceKind::Capsule, [0.0, 0.0, *aspect, 0.0])
                }
                ParticleShape::Soft { hardness }
                    if hardness.is_finite() && (0.0..=1.0).contains(hardness) =>
                {
                    (InstanceKind::Soft, [*hardness, 0.0, 1.0, 0.0])
                }
                ParticleShape::Quad { aspect }
                    if aspect.is_finite() && *aspect > 0.0 && *aspect <= 64.0 =>
                {
                    (InstanceKind::Quad, [0.0, 0.0, *aspect, 0.0])
                }
                _ => return Err(invalid("shape", "Invalid analytic shape parameters")),
            };
            (*radius, kind, extra, true)
        }
        ParticleStyle::Bokeh {
            radius,
            aperture,
            focus,
            shape,
            ring,
            hardness,
            highlight,
        } => {
            if [aperture, focus, ring, hardness, highlight]
                .iter()
                .any(|value| !value.is_finite())
                || *aperture < 0.0
                || *highlight < 0.0
                || *highlight > 65504.0
                || !(0.0..=1.0).contains(ring)
                || !(0.0..=1.0).contains(hardness)
            {
                return Err(invalid("bokeh", "Invalid finite aperture/focus/energy"));
            }
            let sides = match shape {
                BokehShape::Disc => 0.0,
                BokehShape::Polygon(sides) if (3..=32).contains(sides) => f32::from(*sides),
                _ => return Err(invalid("bokeh", "Require 3..=32 aperture blades")),
            };
            (
                *radius,
                InstanceKind::Bokeh,
                [sides, *ring, *hardness, 0.0],
                false,
            )
        }
        ParticleStyle::Trails { width, .. } => {
            (*width * 0.5, InstanceKind::TaperedSegment, [0.0; 4], false)
        }
        ParticleStyle::Plexus { radius, .. } => {
            (*radius, InstanceKind::Disc, [0.0, 0.0, 1.0, 0.0], false)
        }
        _ => {
            return Err(invalid(
                "style",
                "Configured style must be a built-in, non-nested style",
            ));
        }
    };
    if !radius.is_finite() || !(0.0..=4096.0).contains(&radius) {
        return Err(invalid("style", "Style radius must be finite in 0..=4096"));
    }
    if let Some(sheet) = options.sheet {
        sheet.frame(&Particle::default())?;
        if let Some(image) = &sprite
            && (image.width() % u32::from(sheet.columns) != 0
                || image.height() % u32::from(sheet.rows) != 0)
        {
            return Err(invalid(
                "sheet",
                "Texture dimensions must divide into whole cells",
            ));
        }
    }
    let mut records = Vec::new();
    let mut sorted = states.to_vec();
    if options.sort_depth || kind == InstanceKind::Bokeh {
        sorted.sort_by(|first, second| {
            second.custom[1]
                .total_cmp(&first.custom[1])
                .then(first.index.cmp(&second.index))
        });
    }
    let mut append = |mut particle: Particle,
                      start: [f32; 2],
                      endpoint: Option<([f32; 4], f32)>,
                      progress: f32|
     -> Result<(), ValidationError> {
        particle.validate()?;
        if records.len() >= MAX_INSTANCE_COUNT {
            return Err(limit("Extended particle instance allowance exhausted"));
        }
        let focus = if let ParticleStyle::Bokeh { focus, .. } = style {
            *focus
        } else {
            0.0
        };
        let start = std::array::from_fn(|axis| {
            start[axis] + options.parallax[axis] * (particle.custom[1] - focus)
        });
        for axis in 0..2 {
            particle.position[axis] += options.parallax[axis] * (particle.custom[1] - focus);
        }
        let mut record = ExtendedInstanceRecord {
            base: InstanceRecord {
                position: particle.position,
                start,
                color: particle.color,
                radius: radius as f32 * particle.size,
                rotation: particle.rotation,
                width: 1.0,
                kind,
            },
            extra,
        };
        if (align || options.stretch.is_some()) && particle.velocity != [0.0; 2] {
            record.base.rotation += libm::atan2f(particle.velocity[1], particle.velocity[0]);
        }
        if let Some(stretch) = options.stretch {
            record.extra[2] = (1.0
                + libm::hypotf(particle.velocity[0], particle.velocity[1]) * stretch.scale)
                .min(stretch.maximum);
            if kind == InstanceKind::Disc {
                record.base.kind = InstanceKind::Capsule;
            }
        }
        if let Some(sheet) = options.sheet {
            record.base.width = record.extra[2].max(1.0);
            record.extra = sheet.uv(&particle)?;
            record.base.kind = InstanceKind::SpriteSheet;
        }
        if options.flip {
            let flip = libm::cosf(particle.age * 5.0 + particle.custom[2]).abs();
            record.extra[2] *= flip.max(0.01);
            for channel in &mut record.base.color[..3] {
                *channel *= 0.35 + flip * 0.65;
            }
        }
        if let ParticleStyle::Bokeh {
            aperture,
            highlight,
            ..
        } = style
        {
            let coc = (1.0 + aperture * (particle.custom[1] - focus).abs()).min(64.0);
            record.base.radius *= coc;
            for channel in &mut record.base.color[..3] {
                *channel *= highlight / (coc * coc);
            }
        }
        if let Some((color, width)) = endpoint {
            record.base.kind = InstanceKind::TaperedSegment;
            record.extra = color;
            record.base.width = width;
            if let Some(taper) = &options.trails.taper {
                record.base.radius *= taper.sample(progress)?;
            }
            if let Some(fade) = &options.trails.fade {
                let fade = fade.sample(progress)?;
                if fade > 1.0 {
                    return Err(invalid("trail", "Trail fade must be in 0..=1"));
                }
                record.base.color = record.base.color.map(|value| value * fade);
            }
            if options.trails.ribbon {
                record.base.kind = InstanceKind::Ribbon;
            }
        }
        records
            .try_reserve(1)
            .map_err(|_| limit("Extended record allocation failed"))?;
        records.push(record);
        Ok(())
    };
    if let ParticleStyle::Trails { length, .. } = style {
        if *length <= Time::ZERO || length.as_seconds() > 60.0 {
            return Err(invalid("trail", "Trail duration must be in (0,60]"));
        }
        let count = options.trails.segments;
        // Flat ribbon ends leave wedge gaps at bends, so ribbons subdivide twice as finely.
        let max_bends = if options.trails.ribbon { 8.0 } else { 4.0 };
        let mut retained = 0_u64;
        let samples = (0..=count)
            .map(|index| {
                let states = sample(
                    context.time() - *length
                        + Time::from_ticks(length.ticks() * i64::from(index) / i64::from(count)),
                )?;
                retained = retained
                    .checked_add(states.len() as u64 * 160)
                    .ok_or_else(|| limit("Historical particle payload overflow"))?;
                if retained > 64 * 1024 * 1024 {
                    return Err(limit("Historical samples and ID lookups exceed 64 MiB"));
                }
                Ok(states)
            })
            .collect::<Result<Vec<_>, _>>()?;
        let lookups = samples
            .iter()
            .map(|states| {
                states
                    .iter()
                    .map(|particle| (particle.index, particle))
                    .collect::<std::collections::BTreeMap<_, _>>()
            })
            .collect::<Vec<_>>();
        for segment in 1..=usize::from(count) {
            for particle in samples[segment].iter() {
                if let Some(before) = lookups[segment - 1].get(&particle.index) {
                    let bends = if options.trails.smooth {
                        let previous = lookups[segment.saturating_sub(2)]
                            .get(&particle.index)
                            .copied()
                            .unwrap_or(before);
                        let next = lookups[(segment + 1).min(usize::from(count))]
                            .get(&particle.index)
                            .copied()
                            .unwrap_or(particle);
                        (libm::hypotf(
                            particle.position[0]
                                - before.position[0]
                                - (next.position[0] - previous.position[0]) * 0.5,
                            particle.position[1]
                                - before.position[1]
                                - (next.position[1] - previous.position[1]) * 0.5,
                        ) / radius.max(0.1) as f32)
                            .ceil()
                            .clamp(1.0, max_bends) as usize
                    } else {
                        1
                    };
                    let mut prior = before.position;
                    for division in 1..=bends {
                        let amount = division as f32 / bends as f32;
                        let progress = (segment as f32 - 1.0 + amount) / f32::from(count);
                        let previous_progress = (segment as f32 - 1.0
                            + (division - 1) as f32 / bends as f32)
                            / f32::from(count);
                        let mut endpoint = *particle;
                        endpoint.color = std::array::from_fn(|channel| {
                            before.color[channel] * (1.0 - amount)
                                + particle.color[channel] * amount
                        });
                        endpoint.size = before.size * (1.0 - amount) + particle.size * amount;
                        let previous_amount = (division - 1) as f32 / bends as f32;
                        let start_color = std::array::from_fn::<_, 4, _>(|channel| {
                            before.color[channel] * (1.0 - previous_amount)
                                + particle.color[channel] * previous_amount
                        });
                        let start_size =
                            before.size * (1.0 - previous_amount) + particle.size * previous_amount;
                        if !options.trails.smooth {
                            endpoint.position = std::array::from_fn(|axis| {
                                before.position[axis] * (1.0 - amount)
                                    + particle.position[axis] * amount
                            });
                        }
                        if options.trails.smooth {
                            let previous = lookups[segment.saturating_sub(2)]
                                .get(&particle.index)
                                .copied()
                                .unwrap_or(before);
                            let next = lookups[(segment + 1).min(usize::from(count))]
                                .get(&particle.index)
                                .copied()
                                .unwrap_or(particle);
                            endpoint.position = std::array::from_fn(|axis| {
                                catmull(
                                    previous.position[axis],
                                    before.position[axis],
                                    particle.position[axis],
                                    next.position[axis],
                                    amount,
                                )
                            });
                        }
                        let taper = options
                            .trails
                            .taper
                            .as_ref()
                            .map_or(Ok(1.0), |curve| curve.sample(previous_progress))?;
                        let fade = options
                            .trails
                            .fade
                            .as_ref()
                            .map_or(Ok(1.0), |curve| curve.sample(previous_progress))?;
                        if fade > 1.0 {
                            return Err(invalid("trail", "Trail fade must be in 0..=1"));
                        }
                        append(
                            endpoint,
                            prior,
                            Some((
                                start_color.map(|value| value * fade),
                                radius as f32 * 2.0 * start_size * taper,
                            )),
                            progress,
                        )?;
                        prior = endpoint.position;
                    }
                }
            }
        }
    } else {
        for particle in &sorted {
            append(*particle, particle.position, None, 0.0)?;
        }
        if let ParticleStyle::Plexus { max_distance, .. } = style {
            for (first, second) in
                super::super::neighbors::pairs(&sorted, *max_distance, 100_000_000)?
            {
                append(
                    sorted[first],
                    sorted[second].position,
                    Some((sorted[second].color, radius as f32 * 2.0)),
                    0.0,
                )?;
            }
        }
    }
    if records.is_empty() {
        return Ok(Node::empty());
    }
    let bounds = records
        .iter()
        .map(|record| support(record, 0.0))
        .reduce(|first, second| first.union(second))
        .unwrap_or_default();
    let batch = ExtendedBatch::new(
        records.into(),
        sprite,
        bounds,
        options
            .blend
            .unwrap_or(if matches!(style, ParticleStyle::GlowDots { .. }) {
                InstanceBlend::Plus
            } else {
                InstanceBlend::SourceOver
            }),
    );
    batch.validate()?;
    Ok(Node::from(batch))
}

fn catmull(first: f32, second: f32, third: f32, fourth: f32, amount: f32) -> f32 {
    0.5 * ((2.0 * second)
        + (-first + third) * amount
        + (2.0 * first - 5.0 * second + 4.0 * third - fourth) * amount * amount
        + (-first + 3.0 * second - 3.0 * third + fourth) * amount * amount * amount)
}
