//! Central parsing and resolution of declarative Workflow project parameters.
//!
//! Ordinary applications do not call this subsystem: they write a study
//! manifest, named state-schema documents, and one arbitrary `parameters.json`
//! beneath the required `wf_configs` project directory. Config parses them once
//! into one immutable namespaced graph. Study retains that graph, uses typed
//! reserved views for workflow procedure, and binds execution-unit, generic program, or
//! environment-managed Python tasks from it.

mod document;
mod error;
mod expansion;
mod manifest;
mod parameters;
mod program;
mod python;
mod specification;
mod store;

#[cfg(test)]
#[path = "config/tests/config_workflow.rs"]
mod config_workflow_tests;

pub(crate) use document::StateSchemaDocument;
pub use error::ConfigError;
pub(crate) use manifest::{
    FailurePolicy, PersistenceSpecification, PhaseSpecification, ReplicatePolicy,
    ReplicateScheduling, StudyManifest,
};
pub(crate) use parameters::{ResolvedExecutionUnitParameters, ResolvedTask};
pub(crate) use program::ResolvedProgramTask;
pub(crate) use specification::ProjectSpecification;
pub(crate) use store::{Config, ConfigSnapshot};
