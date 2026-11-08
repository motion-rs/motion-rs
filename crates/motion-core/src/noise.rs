//! Stateless integer-hashed procedural noise with analytic spatial derivatives.
//!
//! This is the single noise implementation shared by animation, particles, ink and
//! filters. Fractal sampling validates finite signed-32-bit lattice support.
//! Spatial primitives use f64; [`film_grain`] uses the display renderer's
//! f32 sequence. WGSL twins use f32 interpolation, so hash equality is bitwise
//! while continuous samples require a numerical tolerance. Large coordinates
//! lose fractional resolution in f32 and are not a cross-backend precision promise.
//!
//! ```
//! use motion_core::noise::{Fractal, fbm, perlin2, perlin2_d};
//! let point = [0.25, -0.75];
//! let (value, gradient) = perlin2_d(point, 42);
//! assert_eq!(value, perlin2(point, 42));
//! let cloud = fbm(|point| perlin2(point, 42), point, Fractal::default())?;
//! assert!(cloud.is_finite() && gradient.iter().all(|value| value.is_finite()));
//! # Ok::<(), motion_core::ValidationError>(())
//! ```
//!
//! Warping is independent of the subsequently sampled material:
//! ```
//! use motion_core::noise::{DomainWarp, domain_warp, simplex2, worley2, curl2};
//! let point = domain_warp([0.37, -1.23], DomainWarp {
//!     seed: 42, strength: 0.8, ..DomainWarp::default()
//! })?;
//! let cloud = simplex2(point, 19);
//! let (nearest, second) = worley2(point, 19);
//! let velocity = curl2(point, 19);
//! assert!(cloud.is_finite() && nearest <= second);
//! assert!(velocity.iter().all(|value| value.is_finite()));
//! # Ok::<(), motion_core::ValidationError>(())
//! ```

mod advanced;
mod film;

pub use advanced::{DomainWarp, domain_warp, perlin3, ridged2, simplex2, turbulence2, worley2};

pub use film::{film_grain, film_grain_multi};

const GRADIENTS: [[f64; 2]; 12] = [
    [1.0, 0.0],
    [0.8660254037844386, 0.5],
    [0.5, 0.8660254037844386],
    [0.0, 1.0],
    [-0.5, 0.8660254037844386],
    [-0.8660254037844386, 0.5],
    [-1.0, 0.0],
    [-0.8660254037844386, -0.5],
    [-0.5, -0.8660254037844386],
    [0.0, -1.0],
    [0.5, -0.8660254037844386],
    [0.8660254037844386, -0.5],
];

/// Fixed wrapping 32-bit avalanche hash.
pub fn hash32(mut value: u32) -> u32 {
    value ^= value >> 16;
    value = value.wrapping_mul(0x7feb_352d);
    value ^= value >> 15;
    value = value.wrapping_mul(0x846c_a68b);
    value ^ (value >> 16)
}

/// Hashes a signed lattice coordinate and seed using only wrapping integers.
pub fn hash2(horizontal: i32, vertical: i32, seed: u32) -> u32 {
    hash32(
        (horizontal as u32).wrapping_mul(0x9e37_79b9)
            ^ (vertical as u32).wrapping_mul(0x85eb_ca6b)
            ^ seed,
    )
}

/// Quintic-interpolated lattice noise in [-1,1], with no retained history.
/// Unsupported nonfinite or out-of-lattice coordinates return zero.
pub fn value2(point: [f64; 2], seed: u32) -> f64 {
    if !lattice_supported(point) {
        return 0.0;
    }
    let cell = [point[0].floor(), point[1].floor()];
    let amount = [point[0] - cell[0], point[1] - cell[1]]
        .map(|value| value * value * value * (value * (value * 6.0 - 15.0) + 10.0));
    let value = |horizontal: i32, vertical: i32| {
        f64::from(hash2(horizontal, vertical, seed)) / f64::from(u32::MAX) * 2.0 - 1.0
    };
    let horizontal = cell[0] as i32;
    let vertical = cell[1] as i32;
    let top = value(horizontal, vertical) * (1.0 - amount[0])
        + value(horizontal.wrapping_add(1), vertical) * amount[0];
    let bottom = value(horizontal, vertical.wrapping_add(1)) * (1.0 - amount[0])
        + value(horizontal.wrapping_add(1), vertical.wrapping_add(1)) * amount[0];
    top * (1.0 - amount[1]) + bottom * amount[1]
}

/// Hashes a signed three-dimensional lattice coordinate with wrapping integers.
pub fn hash3(horizontal: i32, vertical: i32, depth: i32, seed: u32) -> u32 {
    hash32(
        (horizontal as u32).wrapping_mul(0x9e37_79b9)
            ^ (vertical as u32).wrapping_mul(0x85eb_ca6b)
            ^ (depth as u32).wrapping_mul(0xc2b2_ae35)
            ^ seed,
    )
}

fn fade(value: f64) -> (f64, f64) {
    (
        value * value * value * (value * (value * 6.0 - 15.0) + 10.0),
        30.0 * value * value * (value - 1.0) * (value - 1.0),
    )
}

/// Value noise and its analytic [horizontal, vertical] derivative.
/// The returned value is bit-identical to `value2`.
pub fn value2_d(point: [f64; 2], seed: u32) -> (f64, [f64; 2]) {
    if !lattice_supported(point) {
        return (0.0, [0.0; 2]);
    }
    let cell = point.map(f64::floor);
    let (horizontal, dh) = fade(point[0] - cell[0]);
    let (vertical, dv) = fade(point[1] - cell[1]);
    let sample = |dx: i32, dy: i32| {
        f64::from(hash2(
            (cell[0] as i32).wrapping_add(dx),
            (cell[1] as i32).wrapping_add(dy),
            seed,
        )) / f64::from(u32::MAX)
            * 2.0
            - 1.0
    };
    let samples = [sample(0, 0), sample(1, 0), sample(0, 1), sample(1, 1)];
    let top = samples[0] * (1.0 - horizontal) + samples[1] * horizontal;
    let bottom = samples[2] * (1.0 - horizontal) + samples[3] * horizontal;
    (
        value2(point, seed),
        [
            dh * ((samples[1] - samples[0]) * (1.0 - vertical)
                + (samples[3] - samples[2]) * vertical),
            dv * (bottom - top),
        ],
    )
}

/// Whether every coordinate is finite and inside the signed 32-bit lattice.
pub fn lattice_supported(point: [f64; 2]) -> bool {
    point.into_iter().all(|value| {
        value.is_finite() && value >= f64::from(i32::MIN) && value < f64::from(i32::MAX)
    })
}

/// Quintic gradient noise with twelve equally spaced unit gradients.
/// Unsupported nonfinite or out-of-lattice coordinates return zero.
pub fn perlin2(point: [f64; 2], seed: u32) -> f64 {
    perlin2_d(point, seed).0
}

/// Divergence-free two-dimensional curl of a shared Perlin potential.
/// Returns [d potential / dy, -d potential / dx], without finite differences.
pub fn curl2(point: [f64; 2], seed: u32) -> [f64; 2] {
    let derivative = perlin2_d(point, seed).1;
    [derivative[1], -derivative[0]]
}

/// Perlin noise and its analytic spatial derivative, without finite differencing.
/// Unsupported nonfinite or out-of-lattice coordinates return zero.
pub fn perlin2_d(point: [f64; 2], seed: u32) -> (f64, [f64; 2]) {
    if !lattice_supported(point) {
        return (0.0, [0.0; 2]);
    }
    let cell = point.map(f64::floor);
    let fraction = [point[0] - cell[0], point[1] - cell[1]];
    let (horizontal, dh) = fade(fraction[0]);
    let (vertical, dv) = fade(fraction[1]);
    let corner = |dx: i32, dy: i32| {
        let gradient = GRADIENTS[(hash2(
            (cell[0] as i32).wrapping_add(dx),
            (cell[1] as i32).wrapping_add(dy),
            seed,
        ) % 12) as usize];
        (
            gradient[0] * (fraction[0] - f64::from(dx))
                + gradient[1] * (fraction[1] - f64::from(dy)),
            gradient,
        )
    };
    let corners = [corner(0, 0), corner(1, 0), corner(0, 1), corner(1, 1)];
    let row = |left: usize, right: usize| {
        let value = corners[left].0 * (1.0 - horizontal) + corners[right].0 * horizontal;
        let derivative = [
            corners[left].1[0] * (1.0 - horizontal)
                + corners[right].1[0] * horizontal
                + dh * (corners[right].0 - corners[left].0),
            corners[left].1[1] * (1.0 - horizontal) + corners[right].1[1] * horizontal,
        ];
        (value, derivative)
    };
    let top = row(0, 1);
    let bottom = row(2, 3);
    (
        top.0 * (1.0 - vertical) + bottom.0 * vertical,
        [
            top.1[0] * (1.0 - vertical) + bottom.1[0] * vertical,
            top.1[1] * (1.0 - vertical) + bottom.1[1] * vertical + dv * (bottom.0 - top.0),
        ],
    )
}

/// Bounded normalized fractal Brownian motion parameters.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Fractal {
    /// Number of octaves, in 1..=8.
    pub octaves: u8,
    /// Frequency multiplier, in 1..=8.
    pub lacunarity: f64,
    /// Successive amplitude multiplier, in 0..=1.
    pub gain: f64,
}

impl Default for Fractal {
    fn default() -> Self {
        Self {
            octaves: 4,
            lacunarity: 2.0,
            gain: 0.5,
        }
    }
}

impl Fractal {
    /// Rejects nonfinite, unbounded or empty fractal configurations.
    pub fn validate(self) -> Result<(), crate::ValidationError> {
        if !(1..=8).contains(&self.octaves)
            || !self.lacunarity.is_finite()
            || !(1.0..=8.0).contains(&self.lacunarity)
            || !self.gain.is_finite()
            || !(0.0..=1.0).contains(&self.gain)
        {
            return Err(crate::ValidationError::invalid(
                "noise",
                "fractal",
                "Require 1..=8 octaves, lacunarity 1..=8 and gain 0..=1",
            ));
        }
        Ok(())
    }
}

/// Evaluates amplitude-normalized fBm, rejecting invalid coordinates or samples.
/// Each octave invokes the supplied stateless noise function exactly once.
pub fn fbm(
    sample: impl Fn([f64; 2]) -> f64,
    mut point: [f64; 2],
    fractal: Fractal,
) -> Result<f64, crate::ValidationError> {
    fractal.validate()?;
    let mut amplitude = 1.0;
    let mut total = 0.0;
    let mut weight = 0.0;
    for _ in 0..fractal.octaves {
        if !lattice_supported(point) {
            return Err(crate::ValidationError::invalid(
                "noise",
                "point",
                "Fractal coordinates exceed signed 32-bit lattice support",
            ));
        }
        let value = sample(point);
        if !value.is_finite() {
            return Err(crate::ValidationError::invalid(
                "noise",
                "sample",
                "Noise samples must be finite",
            ));
        }
        total += value * amplitude;
        weight += amplitude;
        amplitude *= fractal.gain;
        point = point.map(|value| value * fractal.lacunarity);
    }
    let value = total / weight;
    if !value.is_finite() {
        return Err(crate::ValidationError::invalid(
            "noise",
            "sample",
            "Fractal accumulation must remain finite",
        ));
    }
    Ok(value)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hashes_and_value_noise_are_stable() {
        assert_eq!(hash32(0), 0);
        assert_eq!(hash32(1), 0x6889_90c0);
        assert_eq!(hash2(-3, 7, 19), 1_084_314_139);
        assert_eq!(hash3(-3, 7, 1, 19), 3_845_583_026);
        assert!((value2([-3.4, 2.7], 42) - 0.0184252545877103).abs() < 1e-15);
        assert!((value2([10.125, -2.75], 42) + 0.4827757548364509).abs() < 1e-15);
        assert_eq!(hash3(-3, 7, 0, 19), hash2(-3, 7, 19));
        assert_ne!(hash3(-3, 7, 1, 19), hash3(-3, 7, 0, 19));
        for point in [[-3.4, 2.7], [0.0, 0.0], [10.125, -2.75]] {
            assert_eq!(value2(point, 42).to_bits(), value2_d(point, 42).0.to_bits());
        }
    }

    #[test]
    fn analytic_derivatives_match_central_differences_and_join_cells() {
        for (sample, derivative) in [
            (
                value2 as fn([f64; 2], u32) -> f64,
                value2_d as fn([f64; 2], u32) -> (f64, [f64; 2]),
            ),
            (perlin2, perlin2_d),
        ] {
            for point in [[-3.4, 2.7], [0.0, 0.0], [10.125, -2.75], [1.0, -2.0]] {
                let actual = derivative(point, 42).1;
                for axis in 0..2 {
                    let mut before = point;
                    let mut after = point;
                    before[axis] -= 1e-5;
                    after[axis] += 1e-5;
                    let expected = (sample(after, 42) - sample(before, 42)) / 2e-5;
                    assert!(
                        (actual[axis] - expected).abs() < 1e-7,
                        "{point:?} {axis}: {actual:?} {expected}"
                    );
                }
            }
            let before = derivative([1.0 - 1e-8, 0.37], 17);
            let after = derivative([1.0 + 1e-8, 0.37], 17);
            assert!((before.0 - after.0).abs() < 1e-7);
            for axis in 0..2 {
                assert!((before.1[axis] - after.1[axis]).abs() < 1e-6);
            }
        }
    }

    #[test]
    fn noise_statistics_and_fractal_limits() {
        for sample in [value2 as fn([f64; 2], u32) -> f64, perlin2] {
            let mut sum = 0.0;
            let mut squared = 0.0;
            for index in 0..8192 {
                let value = sample(
                    [f64::from(index % 128) * 0.37, f64::from(index / 128) * 0.41],
                    733,
                );
                assert!(value.is_finite() && value.abs() <= 1.0);
                sum += value;
                squared += value * value;
            }
            assert!((sum / 8192.0).abs() < 0.06);
            assert!(squared / 8192.0 > 0.01);
        }
        assert_eq!(fbm(|_| 0.75, [0.0; 2], Fractal::default()).unwrap(), 0.75);
        for octaves in [0, 9, 255] {
            assert!(
                fbm(
                    |_| 0.0,
                    [0.0; 2],
                    Fractal {
                        octaves,
                        ..Fractal::default()
                    }
                )
                .is_err()
            );
        }
        assert!(fbm(|_| f64::NAN, [0.0; 2], Fractal::default()).is_err());
        assert!(fbm(|_| 0.0, [f64::INFINITY, 0.0], Fractal::default()).is_err());
        assert!(fbm(|_| 0.0, [1e9; 2], Fractal::default()).is_err());
    }
}
