//! Runtime adapter from task observation boundaries to durable storage.

use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use serde_json::{Map, Value};

use crate::state::advanced::{SystemState, SystemStateSchema};
use crate::storage::SystemStateWriter;
use crate::task::advanced::{TaskExecutionHost, TaskResult};
use crate::writer::advanced::Writer;

pub(crate) struct RuntimeTaskHost {
    schema: SystemStateSchema,
    cancellation: Arc<AtomicBool>,
    recording_directory: PathBuf,
    metadata: Map<String, Value>,
    writer: Option<SystemStateWriter>,
    final_iteration: Option<u64>,
}

impl RuntimeTaskHost {
    pub(crate) fn new(
        schema: SystemStateSchema,
        cancellation: Arc<AtomicBool>,
        recording_directory: PathBuf,
        metadata: Map<String, Value>,
    ) -> Self {
        Self {
            schema,
            cancellation,
            recording_directory,
            metadata,
            writer: None,
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
        if let Some(writer) = self.writer.take() {
            let _ = writer.mark_recording_failed(reason.to_owned());
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
        writer: Writer,
        state: &SystemState,
        _target_iteration: Option<u64>,
    ) -> TaskResult {
        let mut storage =
            SystemStateWriter::builder(self.recording_directory.clone(), &self.schema)
                .with_writer(writer)
                .with_user_metadata(self.metadata.clone())
                .create_new_recording()?;
        storage.observe_state(state)?;
        self.final_iteration = Some(state.time().iteration());
        self.writer = Some(storage);
        Ok(())
    }

    fn observe_model_step(
        &mut self,
        state: &SystemState,
        _target_iteration: Option<u64>,
    ) -> TaskResult {
        self.writer
            .as_mut()
            .expect("begin_model precedes step observation")
            .observe_state(state)?;
        self.final_iteration = Some(state.time().iteration());
        Ok(())
    }

    fn observe_model_final(
        &mut self,
        state: &SystemState,
        _target_iteration: Option<u64>,
    ) -> TaskResult {
        let writer = self
            .writer
            .take()
            .expect("begin_model precedes final observation");
        writer.complete_recording_with_final_state(state)?;
        self.final_iteration = Some(state.time().iteration());
        Ok(())
    }
}
