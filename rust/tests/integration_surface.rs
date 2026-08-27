//! Downstream checks for the crate facade and centrally aggregated API tiers.

use std::path::Path;

#[test]
fn ordinary_facade_and_error_are_available_through_every_supported_scope() {
    let _run: fn(&Path) -> Result<(), scientific_workflow::WorkflowError> =
        scientific_workflow::run;

    fn accepts_root(_: Option<scientific_workflow::WorkflowError>) {}
    fn accepts_module_basic(_: Option<scientific_workflow::error::basic::WorkflowError>) {}
    fn accepts_module_advanced(_: Option<scientific_workflow::error::advanced::WorkflowError>) {}
    fn accepts_prelude_basic(_: Option<scientific_workflow::prelude::basic::WorkflowError>) {}
    fn accepts_prelude_advanced(_: Option<scientific_workflow::prelude::advanced::WorkflowError>) {}

    accepts_root(None);
    accepts_module_basic(None);
    accepts_module_advanced(None);
    accepts_prelude_basic(None);
    accepts_prelude_advanced(None);
}

#[test]
fn runtime_advanced_accepts_only_a_completed_study() {
    let _execute: fn(
        scientific_workflow::study::advanced::Study,
    ) -> Result<
        scientific_workflow::runtime::advanced::RunSummary,
        scientific_workflow::runtime::advanced::RuntimeError,
    > = scientific_workflow::runtime::advanced::execute;

    #[allow(unused_imports)]
    use scientific_workflow::runtime::basic::*;
}
