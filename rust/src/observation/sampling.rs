//! Private validated observation sampling policy.

use std::num::NonZeroU64;

/// Iteration cadence for one validated stream.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) struct IterationSampling(NonZeroU64);

impl IterationSampling {
    /// Every iteration is the inference-first default.
    pub(super) const EVERY: Self = Self(NonZeroU64::MIN);

    /// Creates a positive cadence.
    pub(super) const fn new(iterations: NonZeroU64) -> Self {
        Self(iterations)
    }

    /// Returns the positive interval.
    pub(super) const fn get(self) -> u64 {
        self.0.get()
    }

    /// Reports whether an iteration is due.
    pub(super) const fn includes(self, iteration: u64) -> bool {
        iteration.is_multiple_of(self.get())
    }
}
