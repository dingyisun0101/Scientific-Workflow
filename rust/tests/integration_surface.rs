//! Downstream checks for the crate facade and centrally aggregated API tiers.

use std::path::Path;

use scientific_workflow::error::basic::WorkflowError;
use scientific_workflow::runtime::advanced::RuntimeError;
use scientific_workflow::study::advanced::StudyError;

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
fn preludes_expose_the_complete_supported_inventories() {
    fn assert_send_sync<T: Send + Sync>() {}
    assert_send_sync::<scientific_workflow::task::basic::InitializationContext>();
    assert_send_sync::<scientific_workflow::task::basic::SeedError>();

    {
        #[allow(unused_imports)]
        use scientific_workflow::prelude::basic::{
            ExecutionUnit, InitializationContext, ModelView, ObservationError, ObservationPlan,
            ObservationStream, PayloadInsertError, SeedError, StateError, StateSeries,
            StateSeriesError, StateSeriesPushError, StateTime, SystemState, SystemStateSchema,
            TaskResult, WorkflowError, execution_unit, model, run,
        };
    }

    {
        #[allow(unused_imports)]
        use scientific_workflow::prelude::advanced::{
            ConfigError, ExecutionUnit, InitializationContext, JsonPayloadDecoder,
            JsonPayloadDecoderRegistry, JsonStringDecoder, JsonVecF64Decoder, ModelView,
            ObservationError, ObservationPlan, ObservationStream, PayloadInsertError,
            PersistenceError, PhaseRunSummary, RecordingTiming, ReplicateRunSummary, RunSummary,
            RuntimeError, SeedError, StateError, StateFieldSchema, StateMaintenance,
            StateSchemaAccess, StateSeries, StateSeriesError, StateSeriesPushError, StateTime,
            StoredStateSeriesReader, Study, StudyError, SystemState, SystemStateSchema, TaskResult,
            TaskRunKind, TaskRunSummary, WorkflowError, execute, execution_unit, model, run,
        };
    }
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
fn runtime_advanced_accepts_only_a_completed_study() {
    let _execute: fn(
        scientific_workflow::study::advanced::Study,
    ) -> Result<
        scientific_workflow::runtime::advanced::RunSummary,
        scientific_workflow::runtime::advanced::RuntimeError,
    > = scientific_workflow::runtime::advanced::execute;

    #[allow(unused_imports)]
    use scientific_workflow::runtime::basic::*;

    let kind = scientific_workflow::prelude::advanced::TaskRunKind::Model;
    assert_eq!(
        kind,
        scientific_workflow::runtime::advanced::TaskRunKind::Model
    );
}
