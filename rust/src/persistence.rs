//! Automatic durable member recordings and program workspaces.
//!
//! Ordinary applications configure persistence only through
//! `wf_configs/study.json`.
//! Study owns an immutable effective plan, while Runtime privately constructs
//! and drives persistence. Execution unit tasks receive structured state recordings;
//! generic program and Python tasks receive config/dependency snapshots,
//! captured logs, launcher provenance, and an artifact directory. The module
//! root exposes verified state-recording readers, never write-session construction.

mod local;
mod plan;
mod session;

#[cfg(test)]
#[path = "persistence/tests/persistence_resilience.rs"]
mod persistence_resilience_tests;
#[cfg(test)]
#[path = "persistence/tests/persistence_workflow.rs"]
mod persistence_workflow_tests;
#[cfg(test)]
#[path = "persistence/tests/python_reader_conformance.rs"]
mod python_reader_conformance_tests;
pub use local::{
    JsonPayloadDecoder, JsonPayloadDecoderRegistry, PersistenceError, RecordingTiming,
    StoredStateSeriesReader,
};
pub(crate) use plan::PersistencePlan;
pub(crate) use session::{
    MemberRecordingProvenance, PersistenceSession, ProgramLaunch, ProgramPersistenceSession,
};
