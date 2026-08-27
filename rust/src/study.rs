//! Effect-free compilation of project declarations into executable scientific intent.
//!
//! Config remains the sole parser. Study composes its immutable output with
//! compiled model registrations, state semantics, deterministic identities,
//! phase organization, and complete preflight validation. Runtime consumes a
//! finished [`advanced::Study`] and never reinterprets project JSON.

mod compilation;
mod error;
mod plan;

/// Ordinary application-facing study API.
///
/// This scope is intentionally empty: ordinary applications write `study.json`
/// and call the crate-level `run(&Path)` entry point.
pub mod basic {}

/// Supported study API for inspection, embedding, and Workflow peer modules.
pub mod advanced {
    #[allow(unused_imports)]
    pub use super::basic::*;
    pub use super::error::StudyError;
    pub use super::plan::{Study, StudyPhase, StudyTask};
}
