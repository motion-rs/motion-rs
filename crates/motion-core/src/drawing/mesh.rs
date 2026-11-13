//! Mesh gradients with analytic paint descriptors and explicit image baking.

use std::any::Any;
use std::sync::Arc;

use crate::budget::Budget;
use crate::{
    Color, GradientExtend, ImagePattern, ImagePixels, PixelSize, Point, RasterLimits, Rect,
    Sampling, Transform, ValidationError,
};

mod evaluate;

pub use evaluate::MeshPatch;

#[cfg(test)]
mod tests;

/// Maximum control points along one axis of a [`MeshGradient`].
pub const MAX_MESH_POINTS_PER_AXIS: u32 = 64;

/// Maximum subdivisions per lattice cell edge when rasterizing a [`MeshGradient`].
pub const MAX_MESH_SUBDIVISIONS: u32 = 32;

/// A [`MeshGradient`] evaluated independently by each paint backend.
///
/// Size and subdivisions remain the resolution of the explicit [`Self::pattern`] bake;
/// they do not quantize analytic paint evaluation.
#[derive(Clone, Debug, PartialEq)]
pub struct MeshPaint {
    /// Control lattice to paint with.
    pub mesh: MeshGradient,
    /// Baked image size in pixels.
    pub size: PixelSize,
    /// Subdivisions per lattice cell edge, in `1..=`[`MAX_MESH_SUBDIVISIONS`].
    pub subdivisions: u32,
}

impl MeshPaint {
    /// Pairs a mesh with its bake resolution.
    pub fn new(mesh: MeshGradient, size: PixelSize, subdivisions: u32) -> Self {
        Self {
            mesh,
            size,
            subdivisions,
        }
    }

    /// Checks the bake parameters without allocating.
    pub fn validate(&self) -> Result<(), ValidationError> {
        if !(1..=MAX_MESH_SUBDIVISIONS).contains(&self.subdivisions) {
            return Err(ValidationError::limit(
                LABEL,
                "subdivisions",
                "Subdivisions must be between 1 and 32",
            ));
        }
        if self.size.width == 0 || self.size.height == 0 {
            return Err(ValidationError::invalid(
                LABEL,
                "size",
                "Mesh image must have positive dimensions",
            ));
        }
        for row in 0..self.mesh.rows() - 1 {
            for column in 0..self.mesh.columns() - 1 {
                self.mesh.patch(column, row)?;
            }
        }
        Ok(())
    }

    /// Bakes the mesh over `bounds` as an extend-padded bilinear pattern.
    pub fn pattern(
        &self,
        bounds: Rect,
        limits: RasterLimits,
    ) -> Result<ImagePattern, ValidationError> {
        self.mesh
            .pattern(bounds, self.size, self.subdivisions, limits)
    }
}

/// Barycentric slack that keeps pixels on shared triangle edges covered.
const EDGE_SLACK: f64 = 1e-9;

const LABEL: &str = "mesh_gradient";

/// One lattice control point: a position in unit object space and its color.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct MeshVertex {
    /// Position as fractions of the painted bounds; finite, usually within `0..=1`.
    pub position: Point,
    /// Color assigned to this control point.
    pub color: Color,
}

/// A grid of colored control points blended smoothly across each cell.
///
/// Each cell blends its four corner colors in Oklab with smoothstep weights, so color
/// is continuous across cells and flat at every control point. Cell geometry is the
/// bilinear patch of its corner positions; moving control points warps the colors.
/// Alpha interpolates linearly. Authored paints evaluate inverse-bilinear patches
/// independently on CPU/GPU; explicit rasterization retains the tessellated image
/// baking path for existing artwork.
///
/// ```
/// use motion_core::{
///     Color, MeshGradient, PixelSize, RasterLimits, Rect,
/// };
/// let mesh = MeshGradient::grid(2, 2, &[
///     Color::from_hex(0xff0000), Color::from_hex(0x00ff00),
///     Color::from_hex(0x0000ff), Color::WHITE,
/// ])?;
/// let pattern = mesh.pattern(
///     Rect::new(0.0, 0.0, 64.0, 64.0),
///     PixelSize::new(16, 16),
///     4,
///     RasterLimits::default(),
/// )?;
/// assert_eq!(pattern.image.width(), 16);
/// # Ok::<(), motion_core::ValidationError>(())
/// ```
#[derive(Clone, Debug, PartialEq)]
pub struct MeshGradient {
    columns: u32,
    rows: u32,
    vertices: Vec<MeshVertex>,
}

impl MeshGradient {
    /// Creates a mesh from row-major control points.
    ///
    /// Requires at least two points per axis, at most [`MAX_MESH_POINTS_PER_AXIS`], and
    /// exactly `columns * rows` finite vertices.
    pub fn new(
        columns: u32,
        rows: u32,
        vertices: Vec<MeshVertex>,
    ) -> Result<Self, ValidationError> {
        if !(2..=MAX_MESH_POINTS_PER_AXIS).contains(&columns)
            || !(2..=MAX_MESH_POINTS_PER_AXIS).contains(&rows)
        {
            return Err(ValidationError::limit(
                LABEL,
                "size",
                "Mesh needs between 2 and 64 control points per axis",
            ));
        }
        if vertices.len() != columns as usize * rows as usize {
            return Err(ValidationError::invalid(
                LABEL,
                "vertices",
                "Vertex count must equal columns times rows",
            ));
        }
        if vertices
            .iter()
            .any(|vertex| !vertex.position.x.is_finite() || !vertex.position.y.is_finite())
        {
            return Err(ValidationError::invalid(
                LABEL,
                "position",
                "Mesh positions must be finite",
            ));
        }
        Ok(Self {
            columns,
            rows,
            vertices,
        })
    }

    /// Creates a regular lattice covering the unit square from row-major colors.
    pub fn grid(columns: u32, rows: u32, colors: &[Color]) -> Result<Self, ValidationError> {
        let vertices = colors
            .iter()
            .enumerate()
            .map(|(index, color)| {
                let column = index as u32 % columns.max(1);
                let row = index as u32 / columns.max(1);
                MeshVertex {
                    position: Point::new(
                        f64::from(column) / f64::from(columns.max(2) - 1),
                        f64::from(row) / f64::from(rows.max(2) - 1),
                    ),
                    color: *color,
                }
            })
            .collect();
        Self::new(columns, rows, vertices)
    }

    /// Control points along the horizontal axis.
    pub fn columns(&self) -> u32 {
        self.columns
    }

    /// Control points along the vertical axis.
    pub fn rows(&self) -> u32 {
        self.rows
    }

    /// Row-major control points.
    pub fn vertices(&self) -> &[MeshVertex] {
        &self.vertices
    }

    /// Creates one admitted analytic patch without allocating an image or lattice.
    ///
    /// Rejects out-of-range indices or coordinates/colors outside shared shader support.
    pub fn patch(&self, column: u32, row: u32) -> Result<MeshPatch, ValidationError> {
        if column >= self.columns - 1 || row >= self.rows - 1 {
            return Err(ValidationError::invalid(
                LABEL,
                "patch",
                "Patch index is outside the mesh",
            ));
        }
        let at = |dx, dy| self.vertices[((row + dy) * self.columns + column + dx) as usize];
        MeshPatch::new([at(0, 0), at(1, 0), at(0, 1), at(1, 1)])
    }

    /// Rasterizes the unit square into a `size` image, splitting every cell edge into
    /// `subdivisions` pieces. Pixels outside the mesh stay transparent.
    ///
    /// Admits pixels, storage and triangle work through `limits` before allocating.
    pub fn rasterize(
        &self,
        size: PixelSize,
        subdivisions: u32,
        limits: RasterLimits,
    ) -> Result<MeshImage, ValidationError> {
        if !(1..=MAX_MESH_SUBDIVISIONS).contains(&subdivisions) {
            return Err(ValidationError::limit(
                LABEL,
                "subdivisions",
                "Subdivisions must be between 1 and 32",
            ));
        }
        if size.width == 0 || size.height == 0 {
            return Err(ValidationError::invalid(
                LABEL,
                "size",
                "Mesh image must have positive dimensions",
            ));
        }
        let mut budget = Budget::new(size, limits)?;
        let texels = u64::from(size.width) * u64::from(size.height);
        let lattice_columns = u64::from((self.columns - 1) * subdivisions + 1);
        let lattice_rows = u64::from((self.rows - 1) * subdivisions + 1);
        budget.bytes(texels * 16, LABEL)?;
        budget.bytes(lattice_columns * lattice_rows * 48, LABEL)?;
        budget.work(lattice_columns * lattice_rows, LABEL)?;
        let mut lattice = Vec::new();
        lattice
            .try_reserve((lattice_columns * lattice_rows) as usize)
            .map_err(|_| {
                ValidationError::limit(LABEL, "max_bytes", "Could not reserve mesh lattice")
            })?;
        for row in 0..lattice_rows as u32 {
            for column in 0..lattice_columns as u32 {
                lattice.push(self.sample(column, row, subdivisions)?);
            }
        }
        let mut pixels = Vec::new();
        pixels.try_reserve(texels as usize).map_err(|_| {
            ValidationError::limit(LABEL, "max_bytes", "Could not reserve mesh pixels")
        })?;
        pixels.resize(texels as usize, [0.0f32; 4]);
        let stride = lattice_columns as usize;
        for row in 0..lattice_rows as usize - 1 {
            for column in 0..stride - 1 {
                let corners = [
                    lattice[row * stride + column],
                    lattice[row * stride + column + 1],
                    lattice[(row + 1) * stride + column],
                    lattice[(row + 1) * stride + column + 1],
                ];
                for triangle in [[0, 1, 2], [1, 3, 2]] {
                    let corners = triangle.map(|index| corners[index]);
                    fill_triangle(&mut pixels, size, corners, &mut budget)?;
                }
            }
        }
        Ok(MeshImage {
            width: size.width,
            height: size.height,
            pixels,
        })
    }

    /// Rasterizes the mesh and maps it onto `bounds` as an extend-padded bilinear pattern.
    pub fn pattern(
        &self,
        bounds: Rect,
        size: PixelSize,
        subdivisions: u32,
        limits: RasterLimits,
    ) -> Result<ImagePattern, ValidationError> {
        let image = self.rasterize(size, subdivisions, limits)?;
        let scale_x = bounds.width() / f64::from(size.width);
        let scale_y = bounds.height() / f64::from(size.height);
        let transform = Transform::scale_non_uniform(scale_x, scale_y)
            .then(Transform::translate(bounds.left, bounds.top));
        let pattern = ImagePattern::from_pixels(Arc::new(image))
            .transform(transform)
            .extend(GradientExtend::Pad)
            .sampling(Sampling::Bilinear);
        pattern.validate()?;
        Ok(pattern)
    }

    fn sample(&self, column: u32, row: u32, subdivisions: u32) -> Result<Lattice, ValidationError> {
        let cell_column = (column / subdivisions).min(self.columns - 2);
        let cell_row = (row / subdivisions).min(self.rows - 2);
        let u = f64::from(column - cell_column * subdivisions) / f64::from(subdivisions);
        let v = f64::from(row - cell_row * subdivisions) / f64::from(subdivisions);
        let at = |dx: u32, dy: u32| {
            self.vertices[((cell_row + dy) * self.columns + cell_column + dx) as usize]
        };
        let [a, b, c, d] = [at(0, 0), at(1, 0), at(0, 1), at(1, 1)];
        let (su, sv) = (smooth(u), smooth(v));
        let top = a.color.try_interpolate(&b.color, su)?;
        let bottom = c.color.try_interpolate(&d.color, su)?;
        let color = top.try_interpolate(&bottom, sv)?;
        let mix = |p: fn(&MeshVertex) -> f64| {
            let upper = p(&a) + (p(&b) - p(&a)) * u;
            let lower = p(&c) + (p(&d) - p(&c)) * u;
            upper + (lower - upper) * v
        };
        Ok(Lattice {
            x: mix(|vertex| vertex.position.x),
            y: mix(|vertex| vertex.position.y),
            color: color.to_linear_premultiplied(),
        })
    }
}

/// A single-level linear-premultiplied image produced by [`MeshGradient::rasterize`].
#[derive(Clone, Debug, PartialEq)]
pub struct MeshImage {
    width: u32,
    height: u32,
    pixels: Vec<[f32; 4]>,
}

impl ImagePixels for MeshImage {
    fn width(&self) -> u32 {
        self.width
    }

    fn height(&self) -> u32 {
        self.height
    }

    fn level_count(&self) -> usize {
        1
    }

    fn level_size(&self, _level: usize) -> (u32, u32) {
        (self.width, self.height)
    }

    fn texel(&self, _level: usize, column: usize, row: usize) -> [f32; 4] {
        self.pixels[row * self.width as usize + column]
    }

    fn content_eq(&self, other: &dyn ImagePixels) -> bool {
        other
            .as_any()
            .downcast_ref::<Self>()
            .is_some_and(|other| self == other)
    }

    fn as_any(&self) -> &dyn Any {
        self
    }
}

#[derive(Clone, Copy)]
struct Lattice {
    x: f64,
    y: f64,
    color: [f64; 4],
}

fn smooth(value: f64) -> f64 {
    value * value * (3.0 - 2.0 * value)
}

fn fill_triangle(
    pixels: &mut [[f32; 4]],
    size: PixelSize,
    corners: [Lattice; 3],
    budget: &mut Budget,
) -> Result<(), ValidationError> {
    let [a, b, c] = corners.map(|corner| {
        (
            corner.x * f64::from(size.width),
            corner.y * f64::from(size.height),
        )
    });
    let area = (b.0 - a.0) * (c.1 - a.1) - (b.1 - a.1) * (c.0 - a.0);
    if area == 0.0 {
        return Ok(());
    }
    let clamp = |low: f64, high: f64, limit: u32| {
        (
            (libm::floor(low - 0.5).max(0.0)) as u32,
            (libm::ceil(high - 0.5).max(-1.0).min(f64::from(limit) - 1.0)) as i64,
        )
    };
    let (first_column, last_column) =
        clamp(a.0.min(b.0).min(c.0), a.0.max(b.0).max(c.0), size.width);
    let (first_row, last_row) = clamp(a.1.min(b.1).min(c.1), a.1.max(b.1).max(c.1), size.height);
    if last_column < i64::from(first_column) || last_row < i64::from(first_row) {
        return Ok(());
    }
    let columns = last_column as u64 - u64::from(first_column) + 1;
    let rows = last_row as u64 - u64::from(first_row) + 1;
    budget.work(columns * rows, LABEL)?;
    for row in first_row..=last_row as u32 {
        for column in first_column..=last_column as u32 {
            let point = (f64::from(column) + 0.5, f64::from(row) + 0.5);
            let w1 = ((point.0 - a.0) * (c.1 - a.1) - (point.1 - a.1) * (c.0 - a.0)) / area;
            let w2 = ((b.0 - a.0) * (point.1 - a.1) - (b.1 - a.1) * (point.0 - a.0)) / area;
            let w0 = 1.0 - w1 - w2;
            if w0 < -EDGE_SLACK || w1 < -EDGE_SLACK || w2 < -EDGE_SLACK {
                continue;
            }
            let texel = &mut pixels[row as usize * size.width as usize + column as usize];
            for (channel, slot) in texel.iter_mut().enumerate() {
                *slot = (w0 * corners[0].color[channel]
                    + w1 * corners[1].color[channel]
                    + w2 * corners[2].color[channel]) as f32;
            }
        }
    }
    Ok(())
}
