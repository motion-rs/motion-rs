//! Conservative support bounds and analytic CPU coverage sampling for extended particle records.

use super::super::{ValidationError, invalid};
use motion_core::Point;
use motion_core::Rect;
use motion_core::instances::{ExtendedInstanceRecord, InstanceKind};

pub(super) fn support(record: &ExtendedInstanceRecord, antialias: f64) -> Rect {
    let base = record.base;
    let line = matches!(
        base.kind,
        InstanceKind::TaperedSegment | InstanceKind::Ribbon | InstanceKind::Segment
    );
    let reach = if line {
        base.radius.max(base.width * 0.5)
    } else {
        base.radius
            * match base.kind {
                InstanceKind::Glow => 3.0,
                InstanceKind::SpriteSheet => base.width.max(1.0) * 2.0,
                InstanceKind::Sprite | InstanceKind::Quad | InstanceKind::Capsule => {
                    record.extra[2].max(1.0) * 2.0
                }
                _ => 2.0,
            }
    };
    let low = if line {
        [
            base.position[0].min(base.start[0]),
            base.position[1].min(base.start[1]),
        ]
    } else {
        base.position
    };
    let high = if line {
        [
            base.position[0].max(base.start[0]),
            base.position[1].max(base.start[1]),
        ]
    } else {
        base.position
    };
    let guard = low
        .into_iter()
        .chain(high)
        .fold(1.0_f64, |value, coordinate| {
            value.max(f64::from(coordinate).abs())
        })
        * 0.000244140625
        + antialias.abs() * 4.0
        + 0.01;
    let margin = f64::from(reach) + guard;
    Rect::new(
        f64::from(low[0]) - margin,
        f64::from(low[1]) - margin,
        f64::from(high[0]) + margin,
        f64::from(high[1]) + margin,
    )
}

fn polygon_distance(local: [f64; 2], radius: f64, sides: f64) -> f64 {
    let sector = std::f64::consts::TAU / sides;
    let angle = libm::atan2(local[1], local[0]);
    libm::cos(libm::floor(angle / sector + 0.5) * sector - angle) * libm::hypot(local[0], local[1])
        - radius * libm::cos(std::f64::consts::PI / sides)
}

/// Cosine and sine of a record's rotation. Rotation is per instance, so batches evaluate it
/// once and reuse it for every sampled point; `libm` makes the values identical either way.
pub(super) fn rotation(record: &ExtendedInstanceRecord) -> [f64; 2] {
    let rotation = f64::from(record.base.rotation);
    [libm::cos(rotation), libm::sin(rotation)]
}

pub(super) fn sample_record(
    record: &ExtendedInstanceRecord,
    [cosine, sine]: [f64; 2],
    sprite: Option<&crate::ImageAsset>,
    point: Point,
    antialias: f64,
) -> Result<[f32; 4], ValidationError> {
    let base = record.base;
    let radius = f64::from(base.radius);
    let line = matches!(
        base.kind,
        InstanceKind::Segment | InstanceKind::TaperedSegment | InstanceKind::Ribbon
    );
    if (radius == 0.0 && (!line || base.width == 0.0))
        || (base.color[3] == 0.0 && (!line || record.extra[3] == 0.0))
    {
        return Ok([0.0; 4]);
    }
    let relative = [
        point.x - f64::from(base.position[0]),
        point.y - f64::from(base.position[1]),
    ];
    let local = [
        relative[0] * cosine + relative[1] * sine,
        -relative[0] * sine + relative[1] * cosine,
    ];
    let mut color = base.color;
    let edge = |distance: f64| ((-distance / antialias.max(1e-12)) + 0.5).clamp(0.0, 1.0);
    let coverage = match base.kind {
        InstanceKind::Segment | InstanceKind::TaperedSegment | InstanceKind::Ribbon => {
            let delta = [
                f64::from(base.position[0] - base.start[0]),
                f64::from(base.position[1] - base.start[1]),
            ];
            let square = delta[0] * delta[0] + delta[1] * delta[1];
            let progress = if square == 0.0 {
                0.0
            } else {
                ((point.x - f64::from(base.start[0])) * delta[0]
                    + (point.y - f64::from(base.start[1])) * delta[1])
                    / square
            };
            let amount = progress.clamp(0.0, 1.0);
            let width = f64::from(base.width) * 0.5 * (1.0 - amount) + radius * amount;
            if base.kind != InstanceKind::Segment {
                color = std::array::from_fn(|channel| {
                    record.extra[channel] * (1.0 - amount as f32) + color[channel] * amount as f32
                });
            }
            if base.kind == InstanceKind::Ribbon && !(0.0..=1.0).contains(&progress) {
                0.0
            } else {
                edge(
                    libm::hypot(
                        point.x - f64::from(base.start[0]) - delta[0] * amount,
                        point.y - f64::from(base.start[1]) - delta[1] * amount,
                    ) - width,
                )
            }
        }
        InstanceKind::Sprite | InstanceKind::SpriteSheet => {
            let aspect = if base.kind == InstanceKind::Sprite {
                f64::from(record.extra[2]).max(1.0)
            } else {
                f64::from(base.width).max(1e-6)
            };
            if radius == 0.0 || local[0].abs() >= radius * aspect || local[1].abs() >= radius {
                0.0
            } else {
                let sprite =
                    sprite.ok_or_else(|| invalid("sprite", "Missing shared sprite material"))?;
                let uv = if base.kind == InstanceKind::SpriteSheet {
                    record.extra
                } else {
                    [0.0, 0.0, 1.0, 1.0]
                };
                let coordinate = [
                    (local[0] / (2.0 * radius * aspect) + 0.5) as f32,
                    (local[1] / (2.0 * radius) + 0.5) as f32,
                ];
                let color_sample = sprite_sample(sprite, coordinate, uv)?;
                color = std::array::from_fn(|channel| color[channel] * color_sample[channel]);
                1.0
            }
        }
        InstanceKind::Glow => {
            let square = (local[0] * local[0] + local[1] * local[1]) / radius.max(1e-12).powi(2);
            if square > 9.0 {
                0.0
            } else {
                libm::exp(-2.0 * square)
            }
        }
        InstanceKind::Polygon => edge(polygon_distance(local, radius, f64::from(record.extra[0]))),
        InstanceKind::Ring => edge(
            (libm::hypot(local[0], local[1]) - radius * (1.0 - f64::from(record.extra[0]) * 0.5))
                .abs()
                - radius * f64::from(record.extra[0]) * 0.5,
        ),
        InstanceKind::Capsule => edge(
            libm::hypot(
                (local[0].abs() - radius * (f64::from(record.extra[2]) - 1.0)).max(0.0),
                local[1],
            ) - radius,
        ),
        InstanceKind::Quad => edge(
            (local[0].abs() - radius * f64::from(record.extra[2])).max(local[1].abs() - radius),
        ),
        InstanceKind::Soft => {
            let distance = libm::hypot(local[0], local[1]);
            edge(distance - radius)
                * ((radius - distance)
                    / (radius * (1.0 - f64::from(record.extra[0]))).max(antialias))
                .clamp(0.0, 1.0)
        }
        InstanceKind::Bokeh => {
            let distance = if record.extra[0] >= 3.0 {
                polygon_distance(local, radius, f64::from(record.extra[0]))
            } else {
                libm::hypot(local[0], local[1]) - radius
            };
            let normalized = libm::hypot(local[0], local[1]) / radius.max(1e-12);
            let rim = libm::exp(-((normalized - 0.9) * 12.0).powi(2));
            let softness = (1.0 - f64::from(record.extra[2])) * radius;
            let energy = ((1.0 + f64::from(record.extra[1]) * rim)
                / (1.0 + f64::from(record.extra[1]) * 0.3)) as f32;
            for channel in &mut color[..3] {
                *channel *= energy;
            }
            edge(distance) * (-distance / softness.max(antialias) + 0.5).clamp(0.0, 1.0)
        }
        _ => edge(libm::hypot(local[0], local[1]) - radius),
    } as f32;
    Ok(color.map(|value| value * coverage))
}

fn sprite_sample(
    sprite: &crate::ImageAsset,
    point: [f32; 2],
    uv: [f32; 4],
) -> Result<[f32; 4], ValidationError> {
    let dimensions = [sprite.width() as f32, sprite.height() as f32];
    let coordinate = std::array::from_fn::<_, 2, _>(|axis| {
        ((uv[axis] + point[axis] * (uv[axis + 2] - uv[axis])) * dimensions[axis] - 0.5).clamp(
            uv[axis] * dimensions[axis],
            uv[axis + 2] * dimensions[axis] - 1.0,
        )
    });
    let origin = coordinate.map(|value| libm::floorf(value) as u32);
    let fraction = [
        coordinate[0] - origin[0] as f32,
        coordinate[1] - origin[1] as f32,
    ];
    let texel = |column, row| {
        sprite.levels()[0]
            .pixel(column, row)
            .ok_or_else(|| invalid("sprite", "Sprite cell outside texture"))
    };
    let high = [
        (origin[0] + 1).min((uv[2] * dimensions[0] - 1.0) as u32),
        (origin[1] + 1).min((uv[3] * dimensions[1] - 1.0) as u32),
    ];
    let first = texel(origin[0], origin[1])?;
    let second = texel(high[0], origin[1])?;
    let third = texel(origin[0], high[1])?;
    let fourth = texel(high[0], high[1])?;
    Ok(std::array::from_fn(|channel| {
        (first[channel] * (1.0 - fraction[0]) + second[channel] * fraction[0]) * (1.0 - fraction[1])
            + (third[channel] * (1.0 - fraction[0]) + fourth[channel] * fraction[0]) * fraction[1]
    }))
}
