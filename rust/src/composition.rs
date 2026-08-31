//! Crate-owned composition of active Runtime mechanics and automatic UI.

use crate::runtime::{RunSummary, RuntimeError};
use crate::study::Study;
use crate::ui::UiSession;

/// Executes a validated Study with Workflow's automatic presentation adapter.
pub fn execute(study: Study) -> Result<RunSummary, RuntimeError> {
    crate::runtime::execute_with_observer(study, || {
        UiSession::automatic().map_err(|source| Box::new(source) as _)
    })
}
