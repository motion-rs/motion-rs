//! Display-film grain: variance-compensated Gaussian values on the shared N0 lattice hash.

use super::hash2;

/// Variance-compensated bilinear film grain at one display pixel position.
/// Intended for finite nonnegative display pixel coordinates; folds both seed words.
pub fn film_grain(position: [f32; 2], seed: u64) -> f32 {
    let [value] = film_grain_multi(position, [seed]);
    value
}

/// Evaluates [`film_grain`] for several seeds at one position. Cell, fraction and
/// variance normalisation are shared across the seeds.
pub fn film_grain_multi<const N: usize>(position: [f32; 2], seeds: [u64; N]) -> [f32; N] {
    let cell = position.map(|value| value.floor() as u32);
    let fraction = position.map(|value| value - value.floor());
    let variance = fraction.map(|value| (1.0 - value) * (1.0 - value) + value * value);
    let normalisation = (variance[0] * variance[1]).sqrt();
    let next = [cell[0].wrapping_add(1), cell[1].wrapping_add(1)];
    // Unsigned cells reinterpret as the signed lattice words of `hash2`, matching WGSL bitcast.
    let uniform =
        |x: u32, y: u32, seed: u32| (hash2(x as i32, y as i32, seed) >> 8) as f32 / 16777216.0;
    seeds.map(|seed| {
        let folded = seed as u32 ^ (seed >> 32) as u32;
        let sample = |x: u32, y: u32| {
            (uniform(x, y, folded) + uniform(x, y, folded ^ 0xa511e9b3) - 1.0) * 2.4494898
        };
        let top =
            sample(cell[0], cell[1]) * (1.0 - fraction[0]) + sample(next[0], cell[1]) * fraction[0];
        let bottom =
            sample(cell[0], next[1]) * (1.0 - fraction[0]) + sample(next[0], next[1]) * fraction[0];
        (top * (1.0 - fraction[1]) + bottom * fraction[1]) / normalisation
    })
}
