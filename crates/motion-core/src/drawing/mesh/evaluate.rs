//! Analytic inverse-bilinear mesh patches shared by independent paint backends.

use crate::{MeshVertex, Point, ValidationError, color};

/// Maximum absolute normalized coordinate admitted by analytic patch arithmetic.
const MAX_POSITION: f64 = 1e6;

/// Shared root slack for points on patch boundaries.
const ROOT_SLACK: f64 = 1e-6;

/// Maximum absolute Oklab component, keeping cubed shader intermediates finite.
const MAX_LAB_COMPONENT: f64 = 1e10;

/// Newton-free inverse-bilinear patch descriptor with straight Oklab and alpha.
#[derive(Clone, Debug, PartialEq)]
pub struct MeshPatch {
    positions: [Point; 4],
    colors: [[f64; 4]; 4],
}

impl MeshPatch {
    pub(super) fn new(vertices: [MeshVertex; 4]) -> Result<Self, ValidationError> {
        let positions = vertices.map(|vertex| vertex.position);
        let colors = vertices.map(|vertex| {
            let [red, green, blue, alpha] = vertex.color.to_rgba_float();
            let [lightness, horizontal, vertical] = color::srgb_to_oklab([red, green, blue]);
            [lightness, horizontal, vertical, alpha]
        });
        if positions.iter().any(|point| {
            !point.x.is_finite()
                || !point.y.is_finite()
                || point.x.abs() > MAX_POSITION
                || point.y.abs() > MAX_POSITION
        }) || colors
            .iter()
            .flatten()
            .any(|value| !value.is_finite() || value.abs() > MAX_LAB_COMPONENT)
        {
            return Err(ValidationError::invalid(
                "mesh_gradient",
                "patch",
                "Analytic patch coordinates and colors exceed finite working support",
            ));
        }
        Ok(Self { positions, colors })
    }

    /// Corners in top-left, top-right, bottom-left, bottom-right lattice order.
    pub fn positions(&self) -> &[Point; 4] {
        &self.positions
    }

    /// Straight Oklab plus linear alpha, in the same corner order as positions.
    pub fn colors(&self) -> &[[f64; 4]; 4] {
        &self.colors
    }

    /// Evaluates a unit-object-space point; returns `None` outside this patch.
    ///
    /// Folded patches choose the first valid quadratic root. Degenerate patches
    /// have no area. Color weights are cubic smoothstep in each patch coordinate.
    /// Rejects nonfinite sample coordinates or nonfinite working-color results.
    pub fn sample(&self, point: Point) -> Result<Option<[f32; 4]>, ValidationError> {
        if !point.x.is_finite() || !point.y.is_finite() {
            return Err(ValidationError::invalid(
                "mesh_gradient",
                "point",
                "Mesh samples must be finite",
            ));
        }
        let [a, b, c, d] = self.positions;
        let b = [b.x - a.x, b.y - a.y];
        let c = [c.x - a.x, c.y - a.y];
        let d = [d.x - a.x - b[0] - c[0], d.y - a.y - b[1] - c[1]];
        let q = [point.x - a.x, point.y - a.y];
        let aa = -cross(b, d);
        let bb = cross(q, d) - cross(b, c);
        let cc = cross(q, c);
        let scale = aa.abs().max(bb.abs()).max(cc.abs());
        if scale == 0.0 {
            return Ok(None);
        }
        let roots = if aa.abs() <= scale * 1e-12 {
            if bb.abs() <= scale * 1e-12 {
                return Ok(None);
            }
            [-cc / bb; 2]
        } else {
            let discriminant = bb * bb - 4.0 * aa * cc;
            if discriminant < 0.0 {
                return Ok(None);
            }
            let root = libm::sqrt(discriminant);
            // Avoid cancellation near affine patches, retaining positive-root-first order.
            let quotient = -0.5 * (bb + if bb >= 0.0 { root } else { -root });
            if quotient == 0.0 {
                [0.0; 2]
            } else if bb >= 0.0 {
                [cc / quotient, quotient / aa]
            } else {
                [quotient / aa, cc / quotient]
            }
        };
        for u in roots {
            let direction = [c[0] + d[0] * u, c[1] + d[1] * u];
            let axis = usize::from(direction[1].abs() > direction[0].abs());
            if direction[axis] == 0.0 {
                continue;
            }
            let v = (q[axis] - b[axis] * u) / direction[axis];
            if !(-ROOT_SLACK..=1.0 + ROOT_SLACK).contains(&u)
                || !(-ROOT_SLACK..=1.0 + ROOT_SLACK).contains(&v)
            {
                continue;
            }
            let smooth = |value: f64| {
                let value = value.clamp(0.0, 1.0);
                value * value * (3.0 - 2.0 * value)
            };
            let (u, v) = (smooth(u), smooth(v));
            let mixed: [f64; 4] = std::array::from_fn(|channel| {
                let top = self.colors[0][channel]
                    + (self.colors[1][channel] - self.colors[0][channel]) * u;
                let bottom = self.colors[2][channel]
                    + (self.colors[3][channel] - self.colors[2][channel]) * u;
                top + (bottom - top) * v
            });
            let rgb =
                color::oklab_to_srgb([mixed[0], mixed[1], mixed[2]]).map(color::srgb_to_linear);
            let result = [
                rgb[0] * mixed[3],
                rgb[1] * mixed[3],
                rgb[2] * mixed[3],
                mixed[3],
            ]
            .map(|value| value as f32);
            if result.iter().any(|value| !value.is_finite()) {
                return Err(ValidationError::invalid(
                    "mesh_gradient",
                    "color",
                    "Mesh sample exceeds finite working storage",
                ));
            }
            return Ok(Some(result));
        }
        Ok(None)
    }
}

fn cross(first: [f64; 2], second: [f64; 2]) -> f64 {
    first[0] * second[1] - first[1] * second[0]
}
