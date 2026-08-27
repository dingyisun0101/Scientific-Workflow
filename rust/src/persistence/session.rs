//! Private automatic persistence session used by Runtime.

use std::path::PathBuf;

use serde_json::{Map, Value};

use crate::observation::advanced::BoundObservationPlan;
use crate::state::advanced::SystemState;

use super::local::{PersistenceError, StateStreamStorage, SystemStateWriter};
use super::plan::PersistencePlan;

pub(crate) struct PersistenceSession {
    writer: Option<SystemStateWriter>,
}

impl PersistenceSession {
    pub(crate) fn start(
        directory: PathBuf,
        observation_plan: BoundObservationPlan,
        persistence_plan: PersistencePlan,
        metadata: Map<String, Value>,
        initial_state: &SystemState,
    ) -> Result<Self, PersistenceError> {
        let storage = StateStreamStorage::chunked(
            persistence_plan.chunk_target(),
            persistence_plan.queue_capacity(),
        );
        let mut writer = SystemStateWriter::create(directory, observation_plan, metadata, storage)?;
        writer.observe_state(initial_state)?;
        Ok(Self {
            writer: Some(writer),
        })
    }

    pub(crate) fn observe(&mut self, state: &SystemState) -> Result<(), PersistenceError> {
        self.writer
            .as_mut()
            .expect("active persistence session owns its backend")
            .observe_state(state)
    }

    pub(crate) fn complete(&mut self, state: &SystemState) -> Result<(), PersistenceError> {
        self.writer
            .take()
            .expect("active persistence session owns its backend")
            .complete_recording_with_final_state(state)?;
        Ok(())
    }

    pub(crate) fn fail(&mut self, reason: &str) {
        if let Some(writer) = self.writer.take() {
            let _ = writer.mark_recording_failed(reason.to_owned());
        }
    }
}
