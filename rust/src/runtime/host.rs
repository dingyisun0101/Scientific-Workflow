//! Runtime adapter from task observation boundaries to automatic persistence.

use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use serde_json::{Map, Value};

use crate::observation::advanced::BoundObservationPlan;
use crate::persistence::advanced::{PersistencePlan, PersistenceSession};
use crate::state::advanced::{SystemState, SystemStateSchema};
use crate::task::advanced::{TaskExecutionHost, TaskResult};

pub(crate) struct RuntimeTaskHost {
    schema: SystemStateSchema,
    persistence_plan: PersistencePlan,
    cancellation: Arc<AtomicBool>,
    recording_directory: PathBuf,
    metadata: Map<String, Value>,
    persistence: Option<PersistenceSession>,
    final_iteration: Option<u64>,
}

impl RuntimeTaskHost {
    pub(crate) fn new(
        schema: SystemStateSchema,
        persistence_plan: PersistencePlan,
        cancellation: Arc<AtomicBool>,
        recording_directory: PathBuf,
        metadata: Map<String, Value>,
    ) -> Self {
        Self {
            schema,
            persistence_plan,
            cancellation,
            recording_directory,
            metadata,
            persistence: None,
            final_iteration: None,
        }
    }

    pub(crate) fn recording_directory(&self) -> &std::path::Path {
        &self.recording_directory
    }

    pub(crate) fn final_iteration(&self) -> Option<u64> {
        self.final_iteration
    }

    pub(crate) fn cancellation_requested(&self) -> bool {
        self.cancellation.load(Ordering::Acquire)
    }

    pub(crate) fn fail(&mut self, reason: &str) {
        if let Some(mut persistence) = self.persistence.take() {
            persistence.fail(reason);
        }
    }
}

impl TaskExecutionHost for RuntimeTaskHost {
    fn state_schema(&self) -> TaskResult<&SystemStateSchema> {
        Ok(&self.schema)
    }

    fn cancellation_requested(&self) -> bool {
        self.cancellation_requested()
    }

    fn begin_model(
        &mut self,
        plan: BoundObservationPlan,
        state: &SystemState,
        _target_iteration: Option<u64>,
    ) -> TaskResult {
        let persistence = PersistenceSession::start(
            self.recording_directory.clone(),
            plan,
            self.persistence_plan,
            self.metadata.clone(),
            state,
        )?;
        self.final_iteration = Some(state.time().iteration());
        self.persistence = Some(persistence);
        Ok(())
    }

    fn observe_model_step(
        &mut self,
        state: &SystemState,
        _target_iteration: Option<u64>,
    ) -> TaskResult {
        self.persistence
            .as_mut()
            .expect("begin_model precedes step observation")
            .observe(state)?;
        self.final_iteration = Some(state.time().iteration());
        Ok(())
    }

    fn observe_model_final(
        &mut self,
        state: &SystemState,
        _target_iteration: Option<u64>,
    ) -> TaskResult {
        self.persistence
            .as_mut()
            .expect("begin_model precedes final observation")
            .complete(state)?;
        self.persistence = None;
        self.final_iteration = Some(state.time().iteration());
        Ok(())
    }
}
