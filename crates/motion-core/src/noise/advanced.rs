//! Cellular, simplex, evolving three-dimensional and warped fractal noise.

use super::*;

/// Euclidean distances to the nearest two seeded cell features, in increasing order.
/// One feature per unit cell; a 5x5 search includes both nearest neighbors.
/// Unsupported coordinates return zero distances.
pub fn worley2(point: [f64; 2], seed: u32) -> (f64, f64) {
    if !lattice_supported(point) {
        return (0.0, 0.0);
    }
    let cell = point.map(f64::floor);
    let fraction = [point[0] - cell[0], point[1] - cell[1]];
    let mut nearest = [f64::INFINITY; 2];
    for vertical in -2i32..=2 {
        for horizontal in -2i32..=2 {
            let hash = hash2(
                (cell[0] as i32).wrapping_add(horizontal),
                (cell[1] as i32).wrapping_add(vertical),
                seed,
            );
            let feature = [
                f64::from(hash32(hash ^ 0xa511_e9b3)) / 4294967296.0,
                f64::from(hash32(hash ^ 0x63d8_3595)) / 4294967296.0,
            ];
            let distance = (f64::from(horizontal) + feature[0] - fraction[0]).powi(2)
                + (f64::from(vertical) + feature[1] - fraction[1]).powi(2);
            if distance < nearest[0] {
                nearest = [distance, nearest[0]];
            } else if distance < nearest[1] {
                nearest[1] = distance;
            }
        }
    }
    (libm::sqrt(nearest[0]), libm::sqrt(nearest[1]))
}

/// Seeded triangular-lattice simplex noise with compact fourth-power kernels.
/// Unsupported coordinates or skewed cells return zero.
pub fn simplex2(point: [f64; 2], seed: u32) -> f64 {
    if !lattice_supported(point) {
        return 0.0;
    }
    let skew = (point[0] + point[1]) * 0.3660254037844386;
    let cell = [(point[0] + skew).floor(), (point[1] + skew).floor()];
    if !lattice_supported(cell) {
        return 0.0;
    }
    let unskew = (cell[0] + cell[1]) * 0.21132486540518713;
    let local = [point[0] - (cell[0] - unskew), point[1] - (cell[1] - unskew)];
    let middle = if local[0] > local[1] {
        [1i32, 0]
    } else {
        [0, 1]
    };
    let corner = |offset: [i32; 2], shift: f64| {
        let delta = [
            local[0] - f64::from(offset[0]) + shift,
            local[1] - f64::from(offset[1]) + shift,
        ];
        let weight = (0.5 - delta[0] * delta[0] - delta[1] * delta[1]).max(0.0);
        let gradient = GRADIENTS[(hash2(
            (cell[0] as i32).wrapping_add(offset[0]),
            (cell[1] as i32).wrapping_add(offset[1]),
            seed,
        ) % 12) as usize];
        weight.powi(4) * (gradient[0] * delta[0] + gradient[1] * delta[1])
    };
    70.0 * (corner([0, 0], 0.0)
        + corner(middle, 0.21132486540518713)
        + corner([1, 1], 0.42264973081037427))
}

/// Quintic three-dimensional Perlin, with the third coordinate usable as time.
/// Twelve normalized edge gradients; unsupported lattice coordinates return zero.
pub fn perlin3(point: [f64; 3], seed: u32) -> f64 {
    if point.into_iter().any(|value| {
        !value.is_finite() || value < f64::from(i32::MIN) || value >= f64::from(i32::MAX)
    }) {
        return 0.0;
    }
    let cell = point.map(f64::floor);
    let fraction: [f64; 3] = std::array::from_fn(|axis| point[axis] - cell[axis]);
    let amount = fraction.map(|value| fade(value).0);
    let gradients = [
        [1.0, 1.0, 0.0],
        [-1.0, 1.0, 0.0],
        [1.0, -1.0, 0.0],
        [-1.0, -1.0, 0.0],
        [1.0, 0.0, 1.0],
        [-1.0, 0.0, 1.0],
        [1.0, 0.0, -1.0],
        [-1.0, 0.0, -1.0],
        [0.0, 1.0, 1.0],
        [0.0, -1.0, 1.0],
        [0.0, 1.0, -1.0],
        [0.0, -1.0, -1.0],
    ];
    let mut result = 0.0;
    for depth in 0i32..=1 {
        for vertical in 0i32..=1 {
            for horizontal in 0i32..=1 {
                let offset = [horizontal, vertical, depth];
                let gradient = gradients[(hash3(
                    (cell[0] as i32).wrapping_add(horizontal),
                    (cell[1] as i32).wrapping_add(vertical),
                    (cell[2] as i32).wrapping_add(depth),
                    seed,
                ) % 12) as usize];
                let dot = (0..3)
                    .map(|axis| gradient[axis] * (fraction[axis] - f64::from(offset[axis])))
                    .sum::<f64>()
                    * std::f64::consts::FRAC_1_SQRT_2;
                let weight = (0..3)
                    .map(|axis| {
                        if offset[axis] == 0 {
                            1.0 - amount[axis]
                        } else {
                            amount[axis]
                        }
                    })
                    .product::<f64>();
                result += dot * weight;
            }
        }
    }
    result
}

/// Bounded displacement by two independent shared Perlin fractals.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct DomainWarp {
    /// Displacement in input-coordinate units, in 0..=1000000.
    pub strength: f64,
    /// Frequency applied only to the displacement potential, in (0, 1000000].
    pub frequency: f64,
    /// Explicit displacement seed; the second axis uses a decorrelated seed.
    pub seed: u32,
    /// Bounded displacement octave recipe.
    pub fractal: Fractal,
}

impl Default for DomainWarp {
    fn default() -> Self {
        Self {
            strength: 0.5,
            frequency: 1.0,
            seed: 0,
            fractal: Fractal::default(),
        }
    }
}

impl DomainWarp {
    /// Validates finite bounded displacement parameters and its octave recipe.
    pub fn validate(self) -> Result<(), crate::ValidationError> {
        self.fractal.validate()?;
        if !self.strength.is_finite()
            || !(0.0..=1e6).contains(&self.strength)
            || !self.frequency.is_finite()
            || self.frequency <= 0.0
            || self.frequency > 1e6
        {
            return Err(crate::ValidationError::invalid(
                "noise",
                "warp",
                "Require strength 0..=1000000 and frequency (0,1000000]",
            ));
        }
        Ok(())
    }
}

/// Warps a point without changing the sampling algorithm applied afterwards.
/// Rejects unsupported input, octave coordinates and displaced output.
pub fn domain_warp(point: [f64; 2], warp: DomainWarp) -> Result<[f64; 2], crate::ValidationError> {
    warp.validate()?;
    if !lattice_supported(point) {
        return Err(crate::ValidationError::invalid(
            "noise",
            "point",
            "Warp coordinates exceed lattice support",
        ));
    }
    if warp.strength == 0.0 {
        return Ok(point);
    }
    let source = point.map(|value| value * warp.frequency);
    let horizontal = fbm(|point| perlin2(point, warp.seed), source, warp.fractal)?;
    let vertical = fbm(
        |point| perlin2(point, warp.seed ^ 0xa511_e9b3),
        source,
        warp.fractal,
    )?;
    let output = [
        point[0] + horizontal * warp.strength,
        point[1] + vertical * warp.strength,
    ];
    if !lattice_supported(output) {
        return Err(crate::ValidationError::invalid(
            "noise",
            "point",
            "Displaced coordinates exceed lattice support",
        ));
    }
    Ok(output)
}

/// Amplitude-normalized absolute Perlin fractal in `[0, 1]`.
pub fn turbulence2(
    point: [f64; 2],
    seed: u32,
    fractal: Fractal,
) -> Result<f64, crate::ValidationError> {
    fbm(|point| perlin2(point, seed).abs(), point, fractal)
}

/// Amplitude-normalized squared ridge fractal in `[0, 1]`.
pub fn ridged2(
    point: [f64; 2],
    seed: u32,
    fractal: Fractal,
) -> Result<f64, crate::ValidationError> {
    fbm(
        |point| (1.0 - perlin2(point, seed).abs()).max(0.0).powi(2),
        point,
        fractal,
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn worley_nearest_pair_matches_larger_reference_search() {
        for index in 0..257 {
            let point = [
                f64::from(index % 17) * 0.173 - 1.2,
                f64::from(index / 17) * 0.137 - 0.9,
            ];
            let cell = point.map(f64::floor);
            let mut distances = Vec::new();
            for vertical in -4i32..=4 {
                for horizontal in -4i32..=4 {
                    let hash = hash2(cell[0] as i32 + horizontal, cell[1] as i32 + vertical, 42);
                    let feature = [
                        cell[0]
                            + f64::from(horizontal)
                            + f64::from(hash32(hash ^ 0xa511_e9b3)) / 4294967296.0,
                        cell[1]
                            + f64::from(vertical)
                            + f64::from(hash32(hash ^ 0x63d8_3595)) / 4294967296.0,
                    ];
                    distances.push(libm::hypot(feature[0] - point[0], feature[1] - point[1]));
                }
            }
            distances.sort_by(f64::total_cmp);
            let actual = worley2(point, 42);
            assert!((actual.0 - distances[0]).abs() < 1e-12);
            assert!((actual.1 - distances[1]).abs() < 1e-12);
            assert!(actual.0 >= 0.0 && actual.0 <= actual.1);
        }
    }

    #[test]
    fn simplex_perlin3_and_warp_are_finite_continuous_and_seeded() {
        assert_eq!(simplex2([0.0; 2], 42), 0.0);
        assert_eq!(perlin3([1.0, -2.0, 3.0], 42), 0.0);
        let mut sum = 0.0;
        let mut energy = 0.0;
        for index in 0..4096 {
            let point = [
                f64::from(index % 64) * 0.173 - 4.0,
                f64::from(index / 64) * 0.137 - 3.0,
            ];
            let value = simplex2(point, 42);
            assert!(value.is_finite() && value.abs() <= 1.0);
            sum += value;
            energy += value * value;
            assert!((value - simplex2([point[0] + 1e-7, point[1]], 42)).abs() < 1e-5);
            assert!(perlin3([point[0], point[1], 0.37], 42).abs() <= 1.0);
        }
        assert!((sum / 4096.0).abs() < 0.06 && energy / 4096.0 > 0.01);
        let point = [0.37, -1.23];
        assert_ne!(
            perlin3([point[0], point[1], 0.3], 42),
            perlin3([point[0], point[1], 0.4], 42)
        );
        assert_ne!(simplex2(point, 42), simplex2(point, 43));
        let warp = DomainWarp {
            seed: 42,
            ..DomainWarp::default()
        };
        assert_eq!(
            domain_warp(
                point,
                DomainWarp {
                    strength: 0.0,
                    ..warp
                }
            )
            .unwrap(),
            point
        );
        let output = domain_warp(point, warp).unwrap();
        assert_eq!(output, domain_warp(point, warp).unwrap());
        assert_ne!(output, point);
        assert_ne!(
            output,
            domain_warp(point, DomainWarp { seed: 43, ..warp }).unwrap()
        );
        assert!(
            domain_warp(
                point,
                DomainWarp {
                    strength: f64::NAN,
                    ..warp
                }
            )
            .is_err()
        );
        assert!(domain_warp([f64::INFINITY, 0.0], warp).is_err());
        for value in [
            turbulence2(point, 42, Fractal::default()).unwrap(),
            ridged2(point, 42, Fractal::default()).unwrap(),
        ] {
            assert!((0.0..=1.0).contains(&value));
        }
    }

    #[test]
    fn curl_is_divergence_free_and_depends_on_both_coordinates() {
        let point = [0.37, -1.23];
        let delta = 1e-5;
        let horizontal = (curl2([point[0] + delta, point[1]], 42)[0]
            - curl2([point[0] - delta, point[1]], 42)[0])
            / (2.0 * delta);
        let vertical = (curl2([point[0], point[1] + delta], 42)[1]
            - curl2([point[0], point[1] - delta], 42)[1])
            / (2.0 * delta);
        assert!((horizontal + vertical).abs() < 1e-7);
        assert_ne!(
            curl2(point, 42)[0],
            curl2([point[0] + 0.1, point[1]], 42)[0]
        );
        assert_ne!(
            curl2(point, 42)[1],
            curl2([point[0], point[1] + 0.1], 42)[1]
        );
    }
}
