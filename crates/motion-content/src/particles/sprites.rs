//! Deterministic procedural sprite materials and bounded immutable atlases.

use super::*;

/// Stateless sprite-frame selection.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum SpriteSheetMode {
    /// Clamp the normalized lifetime to one authored animation pass.
    OverLifetime,
    /// Loop using a finite nonnegative frame rate.
    Fps(f64),
    /// Pick once from stable particle.index and an explicit seed.
    Random(u64),
    /// Select an atlas cell using `floor(particle.custom[0])`.
    Custom,
}

/// Uniform row-major sheet cells. Sampling clamps inside the selected cell.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct SpriteSheet {
    /// Positive column count.
    pub columns: u16,
    /// Positive row count.
    pub rows: u16,
    /// Active frames, at most columns*rows.
    pub frames: u16,
    /// Pure time/ID selection policy.
    pub mode: SpriteSheetMode,
}

impl SpriteSheet {
    /// Checks layout and computes a frame without mutating particle state.
    pub fn frame(&self, particle: &Particle) -> Result<u16, ValidationError> {
        if self.columns == 0
            || self.rows == 0
            || self.frames == 0
            || u32::from(self.frames) > u32::from(self.columns) * u32::from(self.rows)
        {
            return Err(invalid(
                "sheet",
                "Require positive sheet dimensions and bounded frame count",
            ));
        }
        particle.validate()?;
        Ok(match self.mode {
            SpriteSheetMode::OverLifetime => {
                ((particle.age / particle.lifetime).clamp(0.0, 1.0) * f32::from(self.frames)) as u16
            }
            SpriteSheetMode::Fps(fps) => {
                if !fps.is_finite() || !(0.0..=10000.0).contains(&fps) {
                    return Err(invalid("sheet", "Frame rate must be finite in 0..=10000"));
                }
                (libm::floor(f64::from(particle.age) * fps).rem_euclid(f64::from(self.frames)))
                    as u16
            }
            SpriteSheetMode::Random(seed) => {
                (Random::for_particle(seed, particle.index, 0).next_float()
                    * f64::from(self.frames)) as u16
            }
            SpriteSheetMode::Custom => {
                libm::floorf(particle.custom[0]).rem_euclid(f32::from(self.frames)) as u16
            }
        }
        .min(self.frames - 1))
    }

    pub(super) fn uv(&self, particle: &Particle) -> Result<[f32; 4], ValidationError> {
        let frame = self.frame(particle)?;
        let column = f32::from(frame % self.columns);
        let row = f32::from(frame / self.columns);
        Ok([
            column / f32::from(self.columns),
            row / f32::from(self.rows),
            (column + 1.0) / f32::from(self.columns),
            (row + 1.0) / f32::from(self.rows),
        ])
    }
}

/// Built-in image materials requiring no external assets.
#[derive(Clone, Copy, Debug)]
pub enum ProceduralSprite {
    /// Radial soft circular material.
    SoftCircle,
    /// Seeded smoke: spatial fBm over a time-evolving three-dimensional Perlin field.
    Smoke {
        /// Positive spatial scale in normalized sprite coordinates, in 0.25..=64.
        scale: f64,
        /// Signed time evolution speed.
        evolution: f64,
        /// Bounded spatial octave recipe.
        fractal: motion_core::noise::Fractal,
    },
    /// Cross-shaped star highlight.
    Star,
    /// Soft annular material.
    Ring,
    /// Vertically elongated warm fire core.
    FireCore,
}

impl ProceduralSprite {
    /// Builds a finite linear sprite; size is 2..=512 and all noise uses an explicit seed.
    pub fn generate(self, size: u32, seed: u64) -> Result<Arc<crate::ImageAsset>, ValidationError> {
        self.generate_at(size, seed, Time::ZERO, motion_core::RasterLimits::default())
    }

    /// Builds a sprite at explicit time after pixel, mip storage and work admission.
    /// Only `Smoke` depends on time; the other materials are static.
    pub fn generate_at(
        self,
        size: u32,
        seed: u64,
        time: Time,
        limits: motion_core::RasterLimits,
    ) -> Result<Arc<crate::ImageAsset>, ValidationError> {
        if !(2..=512).contains(&size) {
            return Err(invalid(
                "sprite",
                "Procedural sprite size must be in 2..=512",
            ));
        }
        let factor = if let Self::Smoke {
            scale,
            evolution,
            fractal,
        } = self
        {
            fractal.validate()?;
            let phase = time.as_seconds() * evolution;
            if !scale.is_finite()
                || !(0.25..=64.0).contains(&scale)
                || !evolution.is_finite()
                || !phase.is_finite()
                || phase < f64::from(i32::MIN)
                || phase >= f64::from(i32::MAX)
            {
                return Err(invalid(
                    "sprite",
                    "Require bounded smoke scale, finite supported phase and fractal",
                ));
            }
            u64::from(fractal.octaves) * 320
        } else {
            32
        };
        let pixels_count = u64::from(size) * u64::from(size);
        let mut mip_size = size;
        let mut mip_pixels = pixels_count;
        while mip_size > 1 {
            mip_size = mip_size.div_ceil(2);
            mip_pixels += u64::from(mip_size).pow(2);
        }
        let mut budget =
            motion_core::budget::Budget::new(motion_core::PixelSize::new(size, size), limits)?;
        budget.bytes(pixels_count * 32 + mip_pixels * 16, "particles.sprite")?;
        budget.work(pixels_count * factor + mip_pixels * 128, "particles.sprite")?;
        let mut pixels = Vec::new();
        pixels
            .try_reserve_exact((size * size) as usize)
            .map_err(|_| limit("Sprite allocation failed"))?;
        for row in 0..size {
            for column in 0..size {
                let horizontal = (f64::from(column) + 0.5) / f64::from(size) * 2.0 - 1.0;
                let vertical = (f64::from(row) + 0.5) / f64::from(size) * 2.0 - 1.0;
                let distance = libm::hypot(horizontal, vertical);
                let alpha = match self {
                    Self::SoftCircle => (1.0 - distance).max(0.0).powi(2),
                    Self::Smoke {
                        scale,
                        evolution,
                        fractal,
                    } => {
                        let phase = time.as_seconds() * evolution;
                        let seed = seed as u32 ^ (seed >> 32) as u32;
                        let field = motion_core::noise::fbm(
                            |point| motion_core::noise::perlin3([point[0], point[1], phase], seed),
                            [horizontal * scale, vertical * scale],
                            fractal,
                        )?;
                        (1.0 - distance).max(0.0).powi(2) * (0.65 + 0.35 * field).clamp(0.0, 1.0)
                    }
                    Self::Star => ((1.0 - distance)
                        / (0.15 + horizontal.abs().min(vertical.abs()) * 8.0))
                        .clamp(0.0, 1.0),
                    Self::Ring => (1.0 - (distance - 0.7).abs() * 12.0).clamp(0.0, 1.0),
                    Self::FireCore => (1.0 - libm::hypot(horizontal * 1.6, vertical))
                        .max(0.0)
                        .powi(2),
                };
                pixels.push(if matches!(self, Self::FireCore) {
                    [1.0, 0.4 + alpha * 0.6, alpha * 0.4, alpha]
                } else {
                    [1.0, 1.0, 1.0, alpha]
                });
            }
        }
        crate::ImageAsset::from_rgba_float(size, size, pixels)
            .map(Arc::new)
            .map_err(|_| invalid("sprite", "Cannot construct finite sprite material"))
    }
}

/// An immutable, uniformly sized atlas; unused cells are transparent.
#[derive(Clone)]
pub struct SpriteAtlas {
    /// Single GPU/CPU shared material.
    pub image: Arc<crate::ImageAsset>,
    /// Selects cells from `custom[0]`.
    pub sheet: SpriteSheet,
}

impl SpriteAtlas {
    /// Packs 1..=64 equally sized images under a conservative construction/mipmap budget.
    pub fn new(images: &[Arc<crate::ImageAsset>], max_bytes: u64) -> Result<Self, ValidationError> {
        let Some(first) = images.first() else {
            return Err(invalid("atlas", "Require 1..=64 images"));
        };
        let cell = [first.width(), first.height()];
        if images
            .iter()
            .any(|image| [image.width(), image.height()] != cell)
        {
            return Err(invalid("atlas", "Atlas cells must have equal dimensions"));
        }
        Self::build(images, cell, max_bytes)
    }

    /// Packs 1..=64 images of any size: every cell is as large as the biggest
    /// image and smaller images are centred on transparent padding, so the
    /// uniform-cell sheet protocol (and CPU/GPU sampling) is unchanged. Sprites
    /// keep their relative pixel scale; a sprite drawn at `size` spans the whole
    /// cell.
    pub fn new_padded(
        images: &[Arc<crate::ImageAsset>],
        max_bytes: u64,
    ) -> Result<Self, ValidationError> {
        if images.is_empty() {
            return Err(invalid("atlas", "Require 1..=64 images"));
        }
        let cell = images.iter().fold([0, 0], |cell, image| {
            [cell[0].max(image.width()), cell[1].max(image.height())]
        });
        Self::build(images, cell, max_bytes)
    }

    /// Selects sub-images by stable particle index and `seed`, so a particle keeps
    /// the same texture for its whole life.
    pub fn random(mut self, seed: u64) -> Self {
        self.sheet.mode = SpriteSheetMode::Random(seed);
        self
    }

    /// Builds a `Sprites` style that selects cells according to `self.sheet.mode`
    /// (`Custom` reads `particle.custom[0]`) and carries the sheet already.
    pub fn style(
        &self,
        size: f64,
        align_to_velocity: bool,
    ) -> Result<ParticleStyle, ValidationError> {
        let (width, height) = (self.image.width(), self.image.height());
        let mut hash = 0xcbf29ce484222325_u64;
        let level = &self.image.levels()[0];
        for value in [width, height, u32::from(self.sheet.frames)]
            .into_iter()
            .chain(
                [level.pixel(0, 0), level.pixel(width / 2, height / 2)]
                    .into_iter()
                    .flatten()
                    .flatten()
                    .map(f32::to_bits),
            )
        {
            hash = (hash ^ u64::from(value)).wrapping_mul(0x100000001b3);
        }
        let image = motion_core::Assets::default()
            .preload(
                motion_core::Source::Bytes {
                    key: format!("particle-atlas:{width}x{height}:{hash:016x}").into(),
                    data: Arc::from([]),
                },
                self.image.clone(),
            )
            .map_err(|_| invalid("atlas", "Atlas exceeds resource allowance"))?;
        Ok(ParticleStyle::Sprites {
            image,
            size,
            align_to_velocity,
        }
        .sheet(self.sheet))
    }

    fn build(
        images: &[Arc<crate::ImageAsset>],
        cell: [u32; 2],
        max_bytes: u64,
    ) -> Result<Self, ValidationError> {
        if images.is_empty() || images.len() > 64 {
            return Err(invalid("atlas", "Require 1..=64 images"));
        }
        let columns = libm::ceil(libm::sqrt(images.len() as f64)) as u16;
        let rows = (images.len() as u16).div_ceil(columns);
        let width = cell[0]
            .checked_mul(u32::from(columns))
            .ok_or_else(|| limit("Atlas width overflow"))?;
        let height = cell[1]
            .checked_mul(u32::from(rows))
            .ok_or_else(|| limit("Atlas height overflow"))?;
        if width > 4096 || height > 4096 || u64::from(width) * u64::from(height) * 64 > max_bytes {
            return Err(limit("Atlas construction exceeds size/byte allowance"));
        }
        let mut pixels = Vec::new();
        pixels
            .try_reserve_exact((width * height) as usize)
            .map_err(|_| limit("Atlas allocation failed"))?;
        pixels.resize((width * height) as usize, [0.0; 4]);
        for (index, image) in images.iter().enumerate() {
            let offset = [
                (cell[0] - image.width()) / 2,
                (cell[1] - image.height()) / 2,
            ];
            for row in 0..image.height() {
                for column in 0..image.width() {
                    let color = image.levels()[0]
                        .pixel(column, row)
                        .ok_or_else(|| invalid("atlas", "Missing image pixel"))?;
                    let alpha = f64::from(color[3]);
                    let target = ((index as u32 / u32::from(columns) * cell[1] + offset[1] + row)
                        * width
                        + index as u32 % u32::from(columns) * cell[0]
                        + offset[0]
                        + column) as usize;
                    let straight = |channel: f32| {
                        if alpha == 0.0 {
                            0.0
                        } else {
                            f64::from(channel) / alpha
                        }
                    };
                    pixels[target] = [
                        straight(color[0]),
                        straight(color[1]),
                        straight(color[2]),
                        alpha,
                    ];
                }
            }
        }
        let image = crate::ImageAsset::from_rgba_float(width, height, pixels)
            .map(Arc::new)
            .map_err(|_| invalid("atlas", "Invalid atlas material"))?;
        Ok(Self {
            image,
            sheet: SpriteSheet {
                columns,
                rows,
                frames: images.len() as u16,
                mode: SpriteSheetMode::Custom,
            },
        })
    }
}
