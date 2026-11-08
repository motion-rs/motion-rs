//! Bit-exact float keys for pure authoring caches.

/// An f64 key distinguishing signed zero and all NaN payload bit patterns.
#[derive(Clone, Copy, Debug)]
pub struct F64(pub f64);

impl PartialEq for F64 {
    fn eq(&self, other: &Self) -> bool {
        self.0.to_bits() == other.0.to_bits()
    }
}

impl Eq for F64 {}

impl std::hash::Hash for F64 {
    fn hash<Hasher: std::hash::Hasher>(&self, state: &mut Hasher) {
        state.write_u64(self.0.to_bits());
    }
}

impl PartialOrd for F64 {
    fn partial_cmp(&self, other: &Self) -> Option<std::cmp::Ordering> {
        Some(self.cmp(other))
    }
}

impl Ord for F64 {
    fn cmp(&self, other: &Self) -> std::cmp::Ordering {
        self.0.total_cmp(&other.0)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn keys_distinguish_signed_zero_and_nan_payloads() {
        let values = [
            F64(0.0),
            F64(-0.0),
            F64(f64::NAN),
            F64(f64::from_bits(f64::NAN.to_bits() + 1)),
        ];
        assert_eq!(
            values
                .into_iter()
                .collect::<std::collections::HashSet<_>>()
                .len(),
            4
        );
        assert_eq!(
            values
                .into_iter()
                .collect::<std::collections::BTreeSet<_>>()
                .len(),
            4
        );
    }
}
