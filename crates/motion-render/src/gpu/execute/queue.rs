//! Bounded ordered queues for asynchronous GPU frames.

use super::*;

impl FrameQueue {
    /// Creates an empty three-slot queue with a positive aggregate byte allowance.
    pub fn new(context: GpuContext, maximum: u64) -> motion_core::Result<Self> {
        if maximum == 0 {
            return Err(ValidationError::invalid(
                "gpu.queue",
                "max_bytes",
                "Queue allowance must be positive",
            )
            .into());
        }
        Ok(Self {
            renderer: Gpu::from_context(context),
            pending: std::collections::VecDeque::new(),
            packed: std::collections::VecDeque::new(),
            bytes: 0,
            maximum,
        })
    }

    /// Number of already submitted frames awaiting ordered completion.
    pub fn len(&self) -> usize {
        self.pending.len() + self.packed.len()
    }

    /// Whether there are no frames awaiting completion.
    pub fn is_empty(&self) -> bool {
        self.pending.is_empty() && self.packed.is_empty()
    }

    /// Sum of logical per-plan peaks reserved in flight, not configuration caps or RSS.
    pub fn in_flight_bytes(&self) -> u64 {
        self.bytes
    }

    /// Completed reusable readback storage, separate from in-flight plan reservations.
    pub fn pooled_readback_bytes(&self) -> u64 {
        self.renderer.pooled_readback_bytes()
    }

    /// Submits only when both the three-frame cap and aggregate allowance admit it.
    /// A rejected plan is not executed and existing queued frames are unchanged.
    pub fn submit(&mut self, frame: &PreparedFrame, plan: Plan) -> motion_core::Result<()> {
        if !self.packed.is_empty() {
            return Err(ValidationError::invalid(
                "gpu.queue",
                "format",
                "Cannot mix linear and packed pending frames",
            )
            .into());
        }
        if self.pending.len() == 3 {
            return Err(ValidationError::limit(
                "gpu.queue",
                "frames",
                "Three frames are already in flight",
            )
            .into());
        }
        let charge = plan.stats.peak_bytes;
        let next = self
            .bytes
            .checked_add(charge)
            .filter(|bytes| *bytes <= self.maximum)
            .ok_or_else(|| {
                ValidationError::limit(
                    "gpu.queue",
                    "max_bytes",
                    "In-flight GPU plans exceed queue allowance",
                )
            })?;
        self.pending.try_reserve(1).map_err(|_| {
            ValidationError::limit(
                "gpu.queue",
                "max_bytes",
                "Could not reserve GPU queue metadata",
            )
        })?;
        let pending = self.renderer.submit_linear(frame, plan)?;
        self.pending.push_back((charge, pending));
        self.bytes = next;
        Ok(())
    }

    /// Completes the oldest frame. Later submissions may already be executing.
    pub fn finish_next(&mut self) -> motion_core::Result<Option<LinearFrame>> {
        let Some((charge, pending)) = self.pending.pop_front() else {
            return Ok(None);
        };
        self.bytes -= charge;
        match pending.finish_reusable() {
            Ok((frame, bytes, pixels, status)) => {
                if bytes <= self.maximum / 4 && bytes <= 64 * 1024 * 1024 {
                    self.renderer.readback = Some((bytes, pixels, status));
                }
                Ok(Some(frame))
            }
            Err(error) => {
                self.pending.clear();
                self.bytes = 0;
                Err(error)
            }
        }
    }

    /// Submits packed output under the three-slot and aggregate logical-peak policy.
    pub fn submit_output(
        &mut self,
        frame: &PreparedFrame,
        plan: Plan,
        format: crate::PixelFormat,
    ) -> motion_core::Result<()> {
        if !self.pending.is_empty() {
            return Err(ValidationError::invalid(
                "gpu.queue",
                "format",
                "Cannot mix packed and linear pending frames",
            )
            .into());
        }
        if self.packed.len() == 3 {
            return Err(ValidationError::limit(
                "gpu.queue",
                "frames",
                "Three frames are already in flight",
            )
            .into());
        }
        let charge = plan.packed_stats(frame.size(), format)?.peak_bytes;
        let next = self
            .bytes
            .checked_add(charge)
            .filter(|bytes| *bytes <= self.maximum)
            .ok_or_else(|| {
                ValidationError::limit(
                    "gpu.queue",
                    "max_bytes",
                    "In-flight GPU plans exceed queue allowance",
                )
            })?;
        self.packed.try_reserve(1).map_err(|_| {
            ValidationError::limit(
                "gpu.queue",
                "max_bytes",
                "Could not reserve GPU queue metadata",
            )
        })?;
        let pending = self.renderer.submit_output(frame, plan, format)?;
        self.packed.push_back((charge, pending));
        self.bytes = next;
        Ok(())
    }

    /// Completes and recycles the oldest packed output, never a later frame first.
    pub fn finish_next_output(&mut self) -> motion_core::Result<Option<crate::FrameBuffer>> {
        let Some((charge, pending)) = self.packed.pop_front() else {
            return Ok(None);
        };
        self.bytes -= charge;
        match pending.finish_reusable() {
            Ok((frame, bytes, pixels, status, packed)) => {
                if bytes <= self.maximum / 4 && bytes <= 64 * 1024 * 1024 {
                    self.renderer.readback = Some((bytes, pixels, status));
                    self.renderer.packed_output = Some((bytes, packed));
                }
                Ok(Some(frame))
            }
            Err(error) => {
                self.packed.clear();
                self.bytes = 0;
                Err(error)
            }
        }
    }
}
