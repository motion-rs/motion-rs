//! Pigment mixing and opt-in multi-stroke pigment compositions baked into one material.

use super::super::{BrushStroke, Prepared, invalid};
use super::bounds;
use super::geometry::{outline, samples};
use super::material::material;
use crate::ImageAsset;
pub use crate::PigmentMix;
use crate::vector::ink::paint::paint_sample;
use motion_core::{
    Canvas, Color, DrawContext, Drawable, FillRule, ImagePattern, Paint, Path2D, PathVerb, Point,
    Rect, Transform, ValidationError,
};
use std::sync::{Arc, OnceLock};

impl PigmentMix {
    /// Mixes approximate reflectances; endpoints remain exact, and this is not a calibrated model.
    pub fn mix(self, first: Color, second: Color, amount: f64) -> Result<Color, ValidationError> {
        if !amount.is_finite() || !(0.0..=1.0).contains(&amount) {
            return Err(invalid("pigment", "Require amount in 0..=1"));
        }
        if amount == 0.0 || first == second {
            return Ok(first);
        }
        if amount == 1.0 {
            return Ok(second);
        }
        let mut first = first.to_rgba_float();
        let mut second = second.to_rgba_float();
        for value in &mut first[..3] {
            *value = motion_core::color::srgb_to_linear(*value);
        }
        for value in &mut second[..3] {
            *value = motion_core::color::srgb_to_linear(*value);
        }
        let mut result = [0.0; 4];
        let reflectance = |rgb: [f64; 4]| {
            let [red, green, blue, _] = rgb.map(|value| value.clamp(0.0, 1.0));
            let white = red.min(green).min(blue);
            let red = red - white;
            let green = green - white;
            let blue = blue - white;
            let (secondary, remaining, secondary_basis, primary_basis) =
                if red <= green && red <= blue {
                    if green <= blue {
                        (
                            green,
                            blue - green,
                            [1.0, 1.0, 1.0, 0.85, 0.1, 0.02],
                            [0.9, 1.0, 0.6, 0.1, 0.01, 0.01],
                        )
                    } else {
                        (
                            blue,
                            green - blue,
                            [1.0, 1.0, 1.0, 0.85, 0.1, 0.02],
                            [0.01, 0.12, 0.9, 1.0, 0.16, 0.02],
                        )
                    }
                } else if green <= red && green <= blue {
                    if red <= blue {
                        (
                            red,
                            blue - red,
                            [1.0, 0.7, 0.1, 0.1, 0.75, 1.0],
                            [0.9, 1.0, 0.6, 0.1, 0.01, 0.01],
                        )
                    } else {
                        (
                            blue,
                            red - blue,
                            [1.0, 0.7, 0.1, 0.1, 0.75, 1.0],
                            [0.02, 0.01, 0.02, 0.25, 0.8, 1.0],
                        )
                    }
                } else if red <= green {
                    (
                        red,
                        green - red,
                        [0.03, 0.15, 0.95, 1.0, 1.0, 0.95],
                        [0.01, 0.12, 0.9, 1.0, 0.16, 0.02],
                    )
                } else {
                    (
                        green,
                        red - green,
                        [0.03, 0.15, 0.95, 1.0, 1.0, 0.95],
                        [0.02, 0.01, 0.02, 0.25, 0.8, 1.0],
                    )
                };
            std::array::from_fn::<_, 6, _>(|index| {
                (white + secondary * secondary_basis[index] + remaining * primary_basis[index])
                    .clamp(1e-6, 1.0)
            })
        };
        let spectra = if matches!(self, Self::KubelkaMunk) {
            let first = reflectance(first);
            let second = reflectance(second);
            Some(std::array::from_fn::<_, 6, _>(|index| {
                let ratio = |value: f64| (1.0 - value).powi(2) / (2.0 * value);
                let absorption =
                    ratio(first[index]) * (1.0 - amount) + ratio(second[index]) * amount;
                1.0 / (1.0 + absorption + libm::sqrt(absorption * absorption + 2.0 * absorption))
            }))
        } else {
            None
        };
        for channel in 0..3 {
            result[channel] = match self {
                Self::Linear => first[channel] * (1.0 - amount) + second[channel] * amount,
                Self::KubelkaMunk => {
                    let spectrum = spectra.as_ref().expect("Kubelka-Munk spectra");
                    match channel {
                        0 => spectrum[3] * 0.15 + spectrum[4] * 0.35 + spectrum[5] * 0.5,
                        1 => {
                            spectrum[1] * 0.1
                                + spectrum[2] * 0.4
                                + spectrum[3] * 0.4
                                + spectrum[4] * 0.1
                        }
                        _ => spectrum[0] * 0.5 + spectrum[1] * 0.4 + spectrum[2] * 0.1,
                    }
                }
            };
        }
        result[3] = first[3] * (1.0 - amount) + second[3] * amount;
        let rgb = [result[0], result[1], result[2]].map(motion_core::color::linear_to_srgb);
        Ok(Color::from_rgb_float(rgb[0], rgb[1], rgb[2]).with_alpha(result[3]))
    }

    /// Composes up to 32 strokes; only covered intersections use pigment mixing.
    pub fn painting(self, strokes: Vec<BrushStroke>) -> PigmentPainting {
        PigmentPainting {
            strokes,
            mix: self,
            prepared: Arc::new(OnceLock::new()),
        }
    }
}

/// Immutable opt-in pigment composition, shared by CPU/GPU as one baked material.
#[derive(Clone, Debug)]
pub struct PigmentPainting {
    strokes: Vec<BrushStroke>,
    mix: PigmentMix,
    prepared: Arc<OnceLock<Result<Prepared, ValidationError>>>,
}

impl Drawable for PigmentPainting {
    fn bounds(&self, _: &DrawContext) -> Result<Rect, ValidationError> {
        if !(1..=32).contains(&self.strokes.len()) {
            return Err(invalid("pigment_strokes", "Require 1..=32 pigment strokes"));
        }
        let mut result = bounds(&self.strokes[0])?;
        for stroke in &self.strokes[1..] {
            result = result.union(bounds(stroke)?);
        }
        Ok(result)
    }

    fn draw(&self, context: &DrawContext, canvas: &mut Canvas<'_>) -> Result<(), ValidationError> {
        let bounds = self.bounds(context)?;
        let prepared = self
            .prepared
            .get_or_init(|| {
                let width = bounds.width().ceil().max(1.0) as u32;
                let height = bounds.height().ceil().max(1.0) as u32;
                if width > 2048 || height > 2048 || u64::from(width) * u64::from(height) > 262144 {
                    return Err(invalid(
                        "pigment_pixels",
                        "Pigment composition exceeds 262144 texels",
                    ));
                }
                let mut layers = Vec::new();
                let mut work = 0_u64;
                for stroke in &self.strokes {
                    let path = outline(&stroke.config, &samples(&stroke.config, stroke.progress)?)?;
                    work = work.saturating_add(
                        path.verbs().len() as u64 * u64::from(width) * u64::from(height),
                    );
                    if work > 32_000_000 {
                        return Err(invalid(
                            "pigment_work",
                            "Pigment coverage exceeds evaluation allowance",
                        ));
                    }
                    layers.push((path, material(&stroke.config, self::bounds(stroke)?)?));
                }
                let mut pixels = Vec::with_capacity(width as usize * height as usize);
                for row in 0..height {
                    for column in 0..width {
                        let point = Point::new(
                            bounds.left + f64::from(column) + 0.5,
                            bounds.top + f64::from(row) + 0.5,
                        );
                        let mut color = Color::TRANSPARENT;
                        for (path, tiles) in &layers {
                            if !inside_outline(path, point) {
                                continue;
                            }
                            let mut source = [0.0; 4];
                            for tile in tiles {
                                if let Paint::Pattern(pattern) = &tile.paint {
                                    let sample = paint_sample(&tile.paint, point, tile.bounds)?;
                                    if sample[3] > 0.0 {
                                        source = sample;
                                        break;
                                    }
                                    let _ = pattern;
                                }
                            }
                            if source[3] <= 0.0 {
                                continue;
                            }
                            let source = Color::from_rgb_float(source[0], source[1], source[2])
                                .with_alpha(source[3]);
                            let previous = color.to_rgba_float();
                            let incoming = source.to_rgba_float();
                            let alpha = incoming[3] + previous[3] * (1.0 - incoming[3]);
                            let mixed = self.mix.mix(color, source, 0.5)?.to_rgba_float();
                            let overlap = previous[3] * incoming[3];
                            let mut output = [0.0; 3];
                            for index in 0..3 {
                                output[index] =
                                    (previous[index] * previous[3] * (1.0 - incoming[3])
                                        + incoming[index] * incoming[3] * (1.0 - previous[3])
                                        + mixed[index] * overlap)
                                        / alpha;
                            }
                            color = Color::from_rgb_float(output[0], output[1], output[2])
                                .with_alpha(alpha);
                        }
                        pixels.push(color.to_rgba_float());
                    }
                }
                let image = ImageAsset::from_rgba_float(width, height, pixels)
                    .map_err(|error| invalid("pigment_image", &error.to_string()))?;
                let paint = ImagePattern::from_asset(Arc::new(image))
                    .transform(Transform::translate(bounds.left, bounds.top))
                    .into();
                let mut path = Path2D::new();
                path.try_move_to(Point::new(bounds.left, bounds.top))?;
                path.try_line_to(Point::new(bounds.right, bounds.top))?;
                path.try_line_to(Point::new(bounds.right, bounds.bottom))?;
                path.try_line_to(Point::new(bounds.left, bounds.bottom))?;
                path.try_close()?;
                Ok(Prepared {
                    path: Arc::new(path),
                    paint,
                    bounds,
                })
            })
            .as_ref()
            .map_err(Clone::clone)?;
        canvas.fill_path(
            prepared.path.clone(),
            Transform::IDENTITY,
            prepared.paint.clone(),
            FillRule::NonZero,
        )
    }
}

fn inside_outline(path: &Path2D, point: Point) -> bool {
    let mut winding = 0_i32;
    let mut first = Point::new(0.0, 0.0);
    let mut previous = first;
    for verb in path.verbs() {
        let end = match verb {
            PathVerb::MoveTo(position) => {
                first = *position;
                previous = first;
                continue;
            }
            PathVerb::LineTo(position) => *position,
            PathVerb::Close => first,
            _ => continue,
        };
        let cross = (end.x - previous.x) * (point.y - previous.y)
            - (point.x - previous.x) * (end.y - previous.y);
        if previous.y <= point.y && end.y > point.y && cross > 0.0 {
            winding += 1;
        } else if previous.y > point.y && end.y <= point.y && cross < 0.0 {
            winding -= 1;
        }
        previous = end;
    }
    winding != 0
}
