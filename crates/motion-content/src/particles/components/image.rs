//! Cached weighted image distributions with bounded preparation.

use super::*;

/// Density policy, independent of inherited color.
#[derive(Clone, Copy, Debug)]
pub enum ImageDensity {
    /// Premultiplied alpha density.
    Alpha,
    /// Nonnegative linear luminance, including source alpha.
    Luminance,
}

#[derive(Clone)]
pub(super) struct ImageDistribution {
    image: Arc<crate::ImageAsset>,
    bounds: Rect,
    ends: Arc<[f64]>,
    total: f64,
}

impl ImageEmitter {
    /// Prepares a CDF under logical budgets; zero-weight images fail explicitly.
    pub fn new(
        image: Arc<crate::ImageAsset>,
        bounds: Rect,
        density: ImageDensity,
        limits: SimulationLimits,
    ) -> Result<Self, ValidationError> {
        motion_core::drawing::validate_rect(bounds, "particles.image", "bounds")?;
        let count = u64::from(image.width()) * u64::from(image.height());
        if bounds.width() <= 0.0
            || bounds.height() <= 0.0
            || count > 1_000_000
            || count * 64 > limits.max_bytes
            || count * 8 > limits.max_work
        {
            return Err(limit("Image emitter preparation allowance exhausted"));
        }
        let mut ends = Vec::new();
        ends.try_reserve_exact(count as usize)
            .map_err(|_| limit("Image CDF allocation failed"))?;
        let mut total = 0.0;
        for row in 0..image.height() {
            for column in 0..image.width() {
                let pixel = image.levels()[0]
                    .pixel(column, row)
                    .ok_or_else(|| invalid("image", "Missing source pixel"))?;
                let weight = match density {
                    ImageDensity::Alpha => f64::from(pixel[3]),
                    ImageDensity::Luminance => (f64::from(pixel[0]) * 0.2126
                        + f64::from(pixel[1]) * 0.7152
                        + f64::from(pixel[2]) * 0.0722)
                        .max(0.0),
                };
                let next = total + weight;
                if !next.is_finite() || (weight > 0.0 && next == total) {
                    return Err(invalid(
                        "image",
                        "Image density cannot represent finite positive weights",
                    ));
                }
                total = next;
                ends.push(total);
            }
        }
        if total == 0.0 {
            return Err(invalid(
                "image",
                "Image contains no positive emission density",
            ));
        }
        Ok(Self(EmitterSettings::new(EmitterSource::Image(Arc::new(
            ImageDistribution {
                image,
                bounds,
                ends: ends.into(),
                total,
            },
        )))))
    }
}

impl ImageDistribution {
    pub(super) fn sample(&self, random: &mut Random) -> Result<(Point, [f32; 4]), ValidationError> {
        let choice = random.next_float() * self.total;
        let index = self
            .ends
            .partition_point(|end| *end <= choice)
            .min(self.ends.len() - 1);
        let column = index as u32 % self.image.width();
        let row = index as u32 / self.image.width();
        let point = Point::new(
            self.bounds.left
                + (f64::from(column) + random.next_float()) / f64::from(self.image.width())
                    * self.bounds.width(),
            self.bounds.top
                + (f64::from(row) + random.next_float()) / f64::from(self.image.height())
                    * self.bounds.height(),
        );
        Ok((
            point,
            self.image.levels()[0]
                .pixel(column, row)
                .ok_or_else(|| invalid("image", "Image CDF pixel outside source"))?,
        ))
    }
}
