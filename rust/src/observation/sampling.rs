//! Private validated observation sampling policy.
use std::num::NonZeroU64;
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) enum IterationSampling {
    Iterations(NonZeroU64),
    InitialAndFinal,
}
impl IterationSampling {
    pub(super) const EVERY: Self = Self::Iterations(NonZeroU64::MIN);
    pub(super) const fn new(iterations: NonZeroU64) -> Self {
        Self::Iterations(iterations)
    }
    pub(super) const fn get(self) -> Option<u64> {
        match self {
            Self::Iterations(n) => Some(n.get()),
            Self::InitialAndFinal => None,
        }
    }
    pub(super) const fn includes(self, iteration: u64) -> bool {
        match self {
            Self::Iterations(n) => iteration.is_multiple_of(n.get()),
            Self::InitialAndFinal => false,
        }
    }
}
