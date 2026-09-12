//! Crate-owned composition of active Runtime mechanics and the required dashboard.

use crate::runtime::{RunSummary, RuntimeError};
use crate::study::Study;
use crate::ui::UiSession;

/// Executes a validated Study with Workflow's required terminal dashboard.
/// Noninteractive launches fail before output creation or cleanup; use screen/tmux.
pub fn execute(study: Study) -> Result<RunSummary, RuntimeError> {
    execute_options(study, false)
}

pub(crate) fn execute_options(study: Study, clean: bool) -> Result<RunSummary, RuntimeError> {
    UiSession::require_terminal()
        .map_err(|error| RuntimeError::presentation_boxed(Box::new(error)))?;
    crate::runtime::execute_with_observer_options(
        study,
        || UiSession::automatic().map_err(|source| Box::new(source) as _),
        clean,
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::IsTerminal;

    #[test]
    fn noninteractive_launch_rejects_before_clean_or_output_creation() {
        if std::io::stdin().is_terminal() && std::io::stderr().is_terminal() {
            return; // The real-terminal contract is qualified by the PTY suite.
        }
        let root = std::env::temp_dir().join(format!(
            "workflow-dashboard-preflight-{}",
            std::process::id()
        ));
        std::fs::create_dir_all(root.join("wf_configs")).unwrap();
        std::fs::write(root.join("wf_configs/study.json"), r#"{"workflow_schema":1,"threads":1,"compute":{"mode":"auto"},"phases":{"run":{"tasks":[{"program":"/bin/true"}]}}}"#).unwrap();
        std::fs::write(root.join("wf_configs/parameters.json"), "{}").unwrap();
        let error = execute(Study::load(&root).unwrap()).unwrap_err();
        assert!(error.to_string().contains("screen or tmux"));
        assert!(!root.join("output").exists());
        std::fs::create_dir(root.join("output")).unwrap();
        std::fs::write(root.join("output/keep"), "complete result").unwrap();
        let error = execute_options(Study::load(&root).unwrap(), true).unwrap_err();
        assert!(matches!(error, RuntimeError::Presentation { .. }));
        assert_eq!(
            std::fs::read_to_string(root.join("output/keep")).unwrap(),
            "complete result"
        );
        std::fs::remove_dir_all(root).unwrap();
    }
}
