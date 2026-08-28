//! Automatic durable model recordings and program workspaces.
//!
//! Ordinary applications configure persistence only through
//! `wf_configs/study.json`.
//! Study owns an immutable effective plan, while Runtime privately constructs
//! and drives persistence. Model tasks receive structured state recordings;
//! generic program and Python tasks receive config/dependency snapshots,
//! captured logs, launcher provenance, and an artifact directory. The Basic API is intentionally empty; Advanced exposes
//! verified state-recording readers, never write-session construction.

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
/// Ordinary application-facing persistence API.
///
/// This scope is intentionally empty. Applications author the optional
/// `persistence` object in `wf_configs/study.json` and call `run(&Path)`.
pub mod basic {}

/// Supported persistence API for inspection, verified reading, and Workflow peers.
pub mod advanced {
    #[allow(unused_imports)]
    pub use super::basic::*;
    pub use super::local::{
        JsonPayloadDecoder, JsonPayloadDecoderRegistry, JsonStringDecoder, JsonVecF64Decoder,
        PersistenceError, RecordingTiming, StoredStateSeriesReader,
    };
    pub(crate) use super::plan::PersistencePlan;
    pub(crate) use super::session::{
        ModelRecordingProvenance, PersistenceSession, ProgramLaunch, ProgramPersistenceSession,
    };
}
