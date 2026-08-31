//! Crate-owned composition of active Runtime mechanics and automatic UI.

use crate::runtime::{RunSummary, RuntimeError};
use crate::study::Study;
#[cfg(feature = "terminal-ui")]
use crate::ui::UiSession;

#[cfg(not(feature = "terminal-ui"))]
use crate::runtime::{PresentationFailure, RuntimeEvent, RuntimeObserver};

/// Executes a validated Study with Workflow's automatic presentation adapter.
pub fn execute(study: Study) -> Result<RunSummary, RuntimeError> {
    #[cfg(feature = "terminal-ui")]
    {
        crate::runtime::execute_with_observer(study, || {
            UiSession::automatic().map_err(|source| Box::new(source) as _)
        })
    }

    #[cfg(not(feature = "terminal-ui"))]
    {
        crate::runtime::execute_with_observer(study, || Ok(SilentObserver))
    }
}

/// Observer used only by explicitly headless, no-default-feature builds.
#[cfg(not(feature = "terminal-ui"))]
struct SilentObserver;

#[cfg(not(feature = "terminal-ui"))]
impl RuntimeObserver for SilentObserver {
    fn publish(&self, _event: RuntimeEvent<'_>) -> Result<(), PresentationFailure> {
        Ok(())
    }

    fn cancellation_requested(&self) -> Result<bool, PresentationFailure> {
        Ok(false)
    }

    fn finish(&self) -> Result<(), PresentationFailure> {
        Ok(())
    }
}
