//! Key-derived deterministic random streams without global or frame-local state.

use std::hash::{Hash, Hasher};
use std::ops::Range;

use crate::Vector2;

/// A reproducible local random stream derived entirely from a hashed key.
///
/// Key bytes use FNV-1a, followed by SplitMix64 for each sample. Integer hashing
/// uses little-endian bytes and widens usize/isize to 64 bits. Custom Hash
/// implementations must themselves be deterministic. Rust's Hash encodings are
/// not a serialization format; cross-version key encoding stability is not
/// promised. This generator is not cryptographic and has no shared mutable state.
///
/// ```
/// use motion_core::Random;
/// let mut first = Random::from_key(("spark", 42_u64));
/// let mut second = Random::from_key(("spark", 42_u64));
/// assert_eq!(first.next_float(), second.next_float());
/// assert!((10.0..20.0).contains(&first.range(10.0..20.0)));
/// assert_eq!(first.choose(&[7]), Some(&7));
/// ```
#[derive(Clone, Debug)]
pub struct Random {
    state: u64,
}

impl Random {
    /// Starts a fresh stream; equal keys reproduce the same sequence.
    pub fn from_key(key: impl Hash) -> Self {
        let mut hasher = KeyHasher(0xcbf29ce484222325);
        key.hash(&mut hasher);
        Self {
            state: hasher.finish(),
        }
    }

    /// Starts a counter-keyed particle stream without Hash encoding or shared state.
    /// Equal seed/index/step values are independent of traversal and worker count.
    pub fn for_particle(seed: u64, index: u64, step: u64) -> Self {
        fn mix(mut value: u64) -> u64 {
            value = (value ^ (value >> 30)).wrapping_mul(0xbf58476d1ce4e5b9);
            value = (value ^ (value >> 27)).wrapping_mul(0x94d049bb133111eb);
            value ^ (value >> 31)
        }
        Self {
            state: mix(seed
                ^ mix(index.wrapping_add(0x9e3779b97f4a7c15))
                ^ mix(step.wrapping_add(0xd1b54a32d192ed03))),
        }
    }

    fn next_bits(&mut self) -> u64 {
        self.state = self.state.wrapping_add(0x9e3779b97f4a7c15);
        let mut value = self.state;
        value = (value ^ (value >> 30)).wrapping_mul(0xbf58476d1ce4e5b9);
        value = (value ^ (value >> 27)).wrapping_mul(0x94d049bb133111eb);
        value ^ (value >> 31)
    }

    /// Returns one of 2^53 equally spaced floating-point values in [0, 1).
    pub fn next_float(&mut self) -> f64 {
        (self.next_bits() >> 11) as f64 / (1_u64 << 53) as f64
    }

    /// Samples a finite half-open interval, including adjacent float endpoints.
    ///
    /// Rounding is clamped below the upper endpoint; no overflowing endpoint
    /// subtraction is performed.
    ///
    /// # Panics
    /// Panics for an empty, reversed or nonfinite range; see [`Self::try_range`].
    pub fn range(&mut self, range: Range<f64>) -> f64 {
        self.try_range(range)
            .unwrap_or_else(|error| panic!("{}", error.message))
    }

    /// Samples like [`Self::range`], reporting an invalid range without
    /// advancing the stream.
    pub fn try_range(&mut self, range: Range<f64>) -> Result<f64, crate::ValidationError> {
        if !(range.start.is_finite() && range.end.is_finite() && range.start < range.end) {
            return Err(crate::ValidationError::invalid(
                "random",
                "range",
                "random range must have finite, strictly increasing endpoints",
            ));
        }
        let progress = self.next_float();
        let value = range.start * (1.0 - progress) + range.end * progress;
        Ok(value.max(range.start).min(range.end.next_down()))
    }

    /// Samples a uniform angle using deterministic libm trigonometry.
    pub fn unit_vector(&mut self) -> Vector2 {
        let angle = self.next_float() * std::f64::consts::TAU;
        Vector2::new(libm::cos(angle), libm::sin(angle))
    }

    /// Selects an element without modulo bias; an empty slice returns None.
    pub fn choose<'a, Value>(&mut self, values: &'a [Value]) -> Option<&'a Value> {
        if values.is_empty() {
            return None;
        }
        let count = u64::try_from(values.len()).expect("slice length fits u64");
        let threshold = count.wrapping_neg() % count;
        loop {
            let bits = self.next_bits();
            if bits >= threshold {
                return Some(&values[(bits % count) as usize]);
            }
        }
    }
}

struct KeyHasher(u64);

impl Hasher for KeyHasher {
    fn finish(&self) -> u64 {
        self.0
    }

    fn write(&mut self, bytes: &[u8]) {
        for byte in bytes {
            self.0 = (self.0 ^ u64::from(*byte)).wrapping_mul(0x100000001b3);
        }
    }

    fn write_u8(&mut self, value: u8) {
        self.write(&value.to_le_bytes());
    }

    fn write_u16(&mut self, value: u16) {
        self.write(&value.to_le_bytes());
    }

    fn write_u32(&mut self, value: u32) {
        self.write(&value.to_le_bytes());
    }

    fn write_u64(&mut self, value: u64) {
        self.write(&value.to_le_bytes());
    }

    fn write_u128(&mut self, value: u128) {
        self.write(&value.to_le_bytes());
    }

    fn write_usize(&mut self, value: usize) {
        self.write_u64(value as u64);
    }

    fn write_i8(&mut self, value: i8) {
        self.write(&value.to_le_bytes());
    }

    fn write_i16(&mut self, value: i16) {
        self.write(&value.to_le_bytes());
    }

    fn write_i32(&mut self, value: i32) {
        self.write(&value.to_le_bytes());
    }

    fn write_i64(&mut self, value: i64) {
        self.write(&value.to_le_bytes());
    }

    fn write_i128(&mut self, value: i128) {
        self.write(&value.to_le_bytes());
    }

    fn write_isize(&mut self, value: isize) {
        self.write_i64(value as i64);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fixed_sequence_matches_independent_integer_reference() {
        let mut random = Random::from_key(42_u64);
        assert_eq!(random.state, 0xff3add6b3789daef);
        for expected in [
            0xa4e6579fd9ba8f6d,
            0x14e9a1cc21726518,
            0x29700e43bfeb9b12,
            0xebc4f50c4cfbc8c4,
        ] {
            assert_eq!(random.next_bits(), expected);
        }
        let mut random = Random::from_key(42_u64);
        assert_eq!(random.next_float(), 0.6441397443065658);
        assert_eq!(
            Random::from_key(42_usize).state,
            Random::from_key(42_u64).state
        );
        assert_eq!(
            Random::from_key(-42_isize).state,
            Random::from_key(-42_i64).state
        );
    }

    #[test]
    fn equal_keys_and_cloned_streams_are_repeatable_without_global_state() {
        let mut first = Random::from_key(("spark", 27_u32));
        let mut same = first.clone();
        let mut different = Random::from_key(("spark", 28_u32));
        for _ in 0..1024 {
            let value = first.next_float();
            assert!((0.0..1.0).contains(&value));
            assert_eq!(value, same.next_float());
        }
        assert_ne!(same.next_float(), different.next_float());
        let expected = Random::from_key("parallel").next_float();
        std::thread::scope(|scope| {
            let handles: Vec<_> = (0..8)
                .map(|_| scope.spawn(|| Random::from_key("parallel").next_float()))
                .collect();
            for handle in handles {
                assert_eq!(handle.join().unwrap(), expected);
            }
        });
    }

    #[test]
    fn ranges_vectors_and_choices_obey_their_domains() {
        let mut random = Random::from_key("domains");
        let ranges = [
            -f64::MAX..f64::MAX,
            f64::MAX.next_down()..f64::MAX,
            -f64::MAX..(-f64::MAX).next_up(),
            0.0..f64::from_bits(1),
            -f64::from_bits(1)..0.0,
            1.0..1.0_f64.next_up(),
            -3.0..7.0,
        ];
        let choices = [2, 5, 9];
        let mut seen = [false; 3];
        for _ in 0..2048 {
            for range in &ranges {
                let value = random.range(range.clone());
                assert!(range.contains(&value), "{range:?}: {value:?}");
            }
            let vector = random.unit_vector();
            assert!((vector.x * vector.x + vector.y * vector.y - 1.0).abs() <= 4.0 * f64::EPSILON);
            let choice = random.choose(&choices).unwrap();
            seen[choices.iter().position(|value| value == choice).unwrap()] = true;
        }
        assert_eq!(seen, [true; 3]);
        assert_eq!(random.choose::<u32>(&[]), None);
        assert_eq!(random.choose(&[7]), Some(&7));
    }

    #[test]
    fn invalid_ranges_fail_explicitly() {
        for range in [
            0.0..0.0,
            2.0..1.0,
            f64::NAN..1.0,
            0.0..f64::INFINITY,
            f64::NEG_INFINITY..0.0,
        ] {
            assert!(std::panic::catch_unwind(|| Random::from_key(0_u32).range(range)).is_err());
        }
    }
}
