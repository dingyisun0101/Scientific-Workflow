//! Immutable effective persistence policy compiled by Study.

use std::num::NonZeroU64;

/// One immutable, fully defaulted persistence plan owned by Study.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct PersistencePlan {
    chunk_target_bytes: NonZeroU64,
    queue_capacity_bytes: NonZeroU64,
}

impl PersistencePlan {
    pub(crate) const fn local(
        chunk_target_bytes: NonZeroU64,
        queue_capacity_bytes: NonZeroU64,
    ) -> Self {
        Self {
            chunk_target_bytes,
            queue_capacity_bytes,
        }
    }

    pub(crate) const fn chunk_target(self) -> NonZeroU64 {
        self.chunk_target_bytes
    }

    pub(crate) const fn queue_capacity(self) -> NonZeroU64 {
        self.queue_capacity_bytes
    }
}
