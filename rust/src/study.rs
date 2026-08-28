//! Effect-free compilation of project declarations into executable scientific intent.
//!
//! Config remains the sole parser. Study composes its immutable output with
//! the retained central Config snapshot, compiled execution unit registrations,
//! already-resolved program/Python tasks, state semantics, deterministic
//! identities, phase organization, inferred operational plans, and complete
//! preflight. Runtime consumes a finished [`Study`] and never
//! reinterprets project JSON.

mod compilation;
mod error;
mod plan;

#[cfg(test)]
#[path = "study/tests/study_workflow.rs"]
mod study_workflow_tests;

pub use error::StudyError;
pub use plan::Study;
pub(crate) use plan::{StudyPhase, StudyTask};
