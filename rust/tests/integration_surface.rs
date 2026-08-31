//! Downstream checks for the crate facade, one prelude, and module-owned APIs.

use std::path::Path;

use scientific_workflow::WorkflowError;
use scientific_workflow::runtime::RuntimeError;
use scientific_workflow::study::StudyError;

#[test]
fn ordinary_facade_and_error_are_available_from_root_and_prelude() {
    let _run: fn(&Path) -> Result<(), WorkflowError> = scientific_workflow::run;

    fn accepts_root(_: Option<scientific_workflow::WorkflowError>) {}
    fn accepts_prelude(_: Option<scientific_workflow::prelude::WorkflowError>) {}

    accepts_root(None);
    accepts_prelude(None);
}

#[test]
fn prelude_exposes_the_complete_ordinary_authoring_inventory() {
    fn assert_send_sync<T: Send + Sync>() {}
    assert_send_sync::<scientific_workflow::InitializationContext>();
    assert_send_sync::<scientific_workflow::SeedError>();

    #[allow(unused_imports)]
    use scientific_workflow::prelude::{
        ExecutionUnit, InitializationContext, MemberCompletion, MemberView, ObservationError,
        ObservationPlan, ObservationStream, PayloadInsertError, SeedError, StateError, StateSeries,
        StateSeriesError, StateSeriesPushError, StateTime, SystemState, SystemStateSchema,
        UnitResult, WorkflowError, execution_unit, run,
    };
}

#[test]
fn specialized_capabilities_live_at_their_owning_module_roots() {
    #[allow(unused_imports)]
    use scientific_workflow::config::ConfigError;
    #[allow(unused_imports)]
    use scientific_workflow::persistence::{
        JsonPayloadDecoder, JsonPayloadDecoderRegistry, PersistenceError, RecordingTiming,
        StoredStateSeriesReader,
    };
    #[allow(unused_imports)]
    use scientific_workflow::runtime::{
        MemberRunSummary, PhaseRunSummary, ReplicateRunSummary, RunSummary, RuntimeError,
        TaskRunKind, TaskRunSummary, execute,
    };
    #[allow(unused_imports)]
    use scientific_workflow::state::{StateFieldSchema, StateSchemaProvider};
    #[allow(unused_imports)]
    use scientific_workflow::study::{
        PhasePlanSummary, PlanFailurePolicy, PlanReplicateScheduling, PlanSummary, PlannedTaskKind,
        Study, StudyError, TaskPlanSummary,
    };
}

#[test]
fn workflow_error_preserves_stage_conversion_and_thread_safety() {
    fn assert_send_sync<T: Send + Sync>() {}
    assert_send_sync::<WorkflowError>();

    let study = WorkflowError::from(StudyError::TaskIdentityOverflow);
    assert!(matches!(
        study,
        WorkflowError::Study(StudyError::TaskIdentityOverflow)
    ));

    let runtime = WorkflowError::from(RuntimeError::ExecutionCancelled);
    assert!(matches!(
        runtime,
        WorkflowError::Runtime(RuntimeError::ExecutionCancelled)
    ));
}

#[test]
fn workflow_error_transparently_preserves_display_and_source_chain() {
    use std::error::Error as _;

    let runtime = RuntimeError::OutputScope {
        path: Path::new("unavailable-output").to_path_buf(),
        source: std::io::Error::other("storage unavailable"),
    };
    let expected_display = runtime.to_string();
    let workflow = WorkflowError::from(runtime);

    assert_eq!(workflow.to_string(), expected_display);
    assert_eq!(
        workflow.source().map(ToString::to_string).as_deref(),
        Some("storage unavailable")
    );
}

#[test]
fn runtime_accepts_only_a_completed_study() {
    let _execute: fn(
        scientific_workflow::study::Study,
    ) -> Result<
        scientific_workflow::runtime::RunSummary,
        scientific_workflow::runtime::RuntimeError,
    > = scientific_workflow::runtime::execute;
}
