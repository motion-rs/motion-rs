//! Shared logical resource admission for bounded geometry and rendering.
use crate::{PixelSize, ValidationError, ValidationLimits};
use std::mem::size_of;

/// Logical preparation/composition budgets, not a process memory cap.
#[derive(Clone, Copy, Debug)]
pub struct RasterLimits {
    /// Maximum raster pixels, including filter guard bands and empty frames.
    pub max_pixels: u64,
    /// Maximum live logical bytes, including scratch; capacity/overhead is excluded.
    pub max_bytes: u64,
    /// Maximum admitted work units; `u64::MAX` disables the authoring work cap.
    pub max_work: u64,
}

impl Default for RasterLimits {
    fn default() -> Self {
        Self::authoring()
    }
}

impl RasterLimits {
    /// Default trusted-authoring admission, bounded by pixels and logical bytes.
    pub const fn authoring() -> Self {
        Self {
            max_pixels: 16_777_216,
            max_bytes: 256 * 1024 * 1024,
            max_work: u64::MAX,
        }
    }

    /// Opt-in bounded raster work; arbitrary scene callbacks are still trusted Rust.
    pub const fn sandbox() -> Self {
        Self {
            max_work: 200_000_000,
            ..Self::authoring()
        }
    }

    /// Checks positive allowances and whether the raster fits the pixel limit.
    /// Does not allocate pixels or reserve the eventual frame's byte/work cost.
    pub fn validate_size(self, size: PixelSize) -> Result<(), ValidationError> {
        Budget::new(size, self).map(|_| ())
    }
}

/// Structural and raster allowances for APIs that check both.
#[derive(Clone, Copy, Debug, Default)]
pub struct SceneLimits {
    /// Layer count and depth allowances.
    pub structure: ValidationLimits,
    /// Pixel, byte and work allowances.
    pub raster: RasterLimits,
}

/// Mutable admission counters for one bounded geometry or rendering operation.
/// Callers release transient payloads from `live_bytes`; work is never refunded.
pub struct Budget {
    /// Immutable allowances used by subsequent admission checks.
    pub limits: RasterLimits,
    live_bytes: u64,
    peak_bytes: u64,
    work: u64,
}

impl Budget {
    /// Creates empty counters after checking positive allowances and pixel count.
    pub fn new(size: PixelSize, limits: RasterLimits) -> Result<Self, ValidationError> {
        for (field, value) in [
            ("max_pixels", limits.max_pixels),
            ("max_bytes", limits.max_bytes),
            ("max_work", limits.max_work),
        ] {
            if value == 0 {
                return Err(ValidationError::invalid(
                    "prepared",
                    field,
                    "Raster limits must be positive",
                ));
            }
        }
        if u64::from(size.width) * u64::from(size.height) > limits.max_pixels {
            return Err(ValidationError::limit(
                "prepared",
                "max_pixels",
                "Output dimensions exceed the pixel budget",
            ));
        }
        Ok(Self {
            limits,
            live_bytes: 0,
            peak_bytes: 0,
            work: 0,
        })
    }

    /// Resumes counters recorded by an earlier planning pass over the same limits.
    ///
    /// The caller vouches that `live_bytes`, `peak_bytes` and `work` already passed
    /// admission against `limits`; nothing is re-checked.
    pub fn resume(limits: RasterLimits, live_bytes: u64, peak_bytes: u64, work: u64) -> Self {
        Self {
            limits,
            live_bytes,
            peak_bytes: peak_bytes.max(live_bytes),
            work,
        }
    }

    /// Current logical payload bytes, excluding allocation capacity and overhead.
    pub fn live_bytes(&self) -> u64 {
        self.live_bytes
    }

    /// Highest admitted live-byte total observed by this operation.
    pub fn peak_bytes(&self) -> u64 {
        self.peak_bytes
    }

    /// Cumulative admitted algorithm work units, not elapsed time.
    pub fn work_done(&self) -> u64 {
        self.work
    }

    /// Returns transient payload previously admitted with [`Self::bytes`]; the peak is kept.
    ///
    /// Releasing more than is live is a caller bug and saturates at zero rather than wrapping.
    pub fn release(&mut self, amount: u64) {
        self.live_bytes = self.live_bytes.saturating_sub(amount);
    }

    /// Restarts peak tracking at the current live bytes and returns the previous peak.
    ///
    /// Use it to measure one nested operation, then call [`Self::merge_peak`] with the
    /// returned value so the enclosing operation's peak is not lost.
    pub fn reset_peak(&mut self) -> u64 {
        std::mem::replace(&mut self.peak_bytes, self.live_bytes)
    }

    /// Raises the peak to at least `peak`, for example after [`Self::reset_peak`].
    pub fn merge_peak(&mut self, peak: u64) {
        self.peak_bytes = self.peak_bytes.max(peak);
    }

    /// Replaces the live-byte count with an externally measured total, raising the peak if needed.
    pub fn set_live_bytes(&mut self, live_bytes: u64) {
        self.live_bytes = live_bytes;
        self.peak_bytes = self.peak_bytes.max(live_bytes);
    }

    /// Admits additional live payload and updates the peak; failure leaves counters unchanged.
    pub fn bytes(&mut self, amount: u64, path: &str) -> Result<(), ValidationError> {
        let next = self
            .live_bytes
            .checked_add(amount)
            .filter(|value| *value <= self.limits.max_bytes)
            .ok_or_else(|| {
                ValidationError::limit(path, "max_bytes", "storage exceeds the logical byte budget")
            })?;
        self.live_bytes = next;
        self.peak_bytes = self.peak_bytes.max(next);
        Ok(())
    }

    /// Admits cumulative work before execution; failure leaves the counter unchanged.
    pub fn work(&mut self, amount: u64, path: &str) -> Result<(), ValidationError> {
        let next = self
            .work
            .checked_add(amount)
            .filter(|value| *value <= self.limits.max_work)
            .ok_or_else(|| ValidationError::limit(path, "max_work", "work exceeds the budget"))?;
        self.work = next;
        Ok(())
    }

    /// Charges one element's logical bytes before fallibly reserving and appending.
    /// An allocation failure retains the admission charge; discard the partial operation.
    pub fn push<Value>(
        &mut self,
        values: &mut Vec<Value>,
        value: Value,
        path: &str,
    ) -> Result<(), ValidationError> {
        self.bytes(size_of::<Value>() as u64, path)?;
        values.try_reserve(1).map_err(|_| {
            ValidationError::limit(path, "max_bytes", "Could not reserve metadata storage")
        })?;
        values.push(value);
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn authoring_admits_large_work_but_retains_storage_and_pixel_limits() {
        let limits = RasterLimits::default();
        let mut budget = Budget::new(PixelSize::new(1920, 1080), limits).unwrap();
        budget.work(4_000_000_000, "particles").unwrap();
        assert_eq!(budget.work, 4_000_000_000);
        budget.bytes(limits.max_bytes, "pixels").unwrap();
        assert!(budget.bytes(1, "pixels").is_err());
        assert_eq!(budget.live_bytes, limits.max_bytes);
        assert!(limits.validate_size(PixelSize::new(8192, 8192)).is_err());
    }

    #[test]
    fn sandbox_and_explicit_work_caps_reject_without_mutating_counters() {
        for limits in [
            RasterLimits::sandbox(),
            RasterLimits {
                max_work: 17,
                ..RasterLimits::default()
            },
        ] {
            let mut budget = Budget::new(PixelSize::new(1, 1), limits).unwrap();
            budget.work(limits.max_work, "particles").unwrap();
            assert!(budget.work(1, "particles").is_err());
            assert_eq!(budget.work, limits.max_work);
        }
    }
}
