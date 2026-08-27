//! Immutable UI policy inferred and owned by Study.

use std::time::Duration;

const DEFAULT_REFRESH_INTERVAL: Duration = Duration::from_millis(100);

/// Complete effective UI policy for one Study.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct UiPlan {
    refresh_interval: Duration,
}

impl UiPlan {
    /// Infers the zero-configuration UI policy.
    pub(crate) const fn automatic() -> Self {
        Self {
            refresh_interval: DEFAULT_REFRESH_INTERVAL,
        }
    }

    pub(crate) const fn refresh_interval(self) -> Duration {
        self.refresh_interval
    }
}
