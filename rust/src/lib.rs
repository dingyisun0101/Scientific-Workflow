//! Configuration-driven scientific workflow execution.
//!
//! An ordinary application defines registered [`task::basic::ScientificModel`]
//! implementations, writes `study.json`, `config/state.json`, and the central
//! `config/parameters.json`, then calls [`run`] with the project root. Workflow infers task
//! instances, phase membership, identities, output paths, progress boundaries,
//! recording lifecycle, and execution mechanics.
//!
//! # Ordinary use
//!
//! ```no_run
//! use std::path::Path;
//!
//! fn main() -> Result<(), scientific_workflow::WorkflowError> {
//!     scientific_workflow::run(Path::new("."))
//! }
//! ```
//!
//! A model carries its stable manifest key at its implementation:
//!
//! ```
//! use serde::Deserialize;
//! use scientific_workflow::prelude::basic::*;
//!
//! #[derive(Deserialize)]
//! struct Constants { initial: u64, steps: u64 }
//!
//! struct Model { state: SystemState, steps: u64 }
//!
//! #[scientific_workflow::model("example")]
//! impl ScientificModel for Model {
//!     type Constants = Constants;
//!
//!     fn initialize(constants: Constants, schema: &SystemStateSchema) -> TaskResult<Self> {
//!         let mut state = schema.create_empty_state(StateTime::from_iteration(0));
//!         state.initialize_payload("population", constants.initial)?;
//!         state.initialize_payload("cumulative_births", 0_u64)?;
//!         Ok(Self { state, steps: constants.steps })
//!     }
//!
//!     fn state(&self) -> &SystemState { &self.state }
//!     fn is_complete(&self) -> bool { self.state.time().iteration() == self.steps }
//!     fn step(&mut self) -> TaskResult {
//!         let (population, cumulative_births) = self
//!             .state
//!             .borrow_payloads_mut::<(u64, u64)>(
//!                 ("population", "cumulative_births"),
//!             )?;
//!         *population += 1;
//!         *cumulative_births += 1;
//!         self.state.advance_time(None)?;
//!         Ok(())
//!     }
//! }
//! ```
//!
//! # Ownership boundaries
//!
//! - [`config`] alone reads and parses project JSON and supplies typed model
//!   constants.
//! - [`study`] composes parsed declarations, state semantics, and compiled model
//!   registrations into immutable, output-free intent.
//! - [`runtime`] consumes a completed Study and owns active execution/output.
//! - [`task`] owns the model contract and automatic observation boundaries.
//! - [`state`] owns canonical scientific state and schema.
//! - [`observation`] owns scientific observation meaning, not persistence mechanics.
//! - [`persistence`] owns automatic durable output and verified reading.
//! - [`ui`] owns automatic terminal presentation of Runtime facts.
//! - [`error`] owns complete-workflow Study/Runtime error composition.
//!
//! The crate-level [`run`] facade performs the sole ordinary transition from
//! project root to Study to Runtime. `Study` is the ultimate coordinator of
//! declared intent; runtime is the ultimate coordinator of active execution.
//! Advanced integrations use each module's `advanced` scope. [`prelude`] only
//! aggregates those module-owned APIs and crate conveniences.
//!
//! Persistence write construction and output allocation are internal and are
//! not available to model authors.
//!
//! This crate is pre-1.0 test software and may make coordinated API changes.

#![deny(missing_docs, rustdoc::broken_intra_doc_links)]

extern crate self as scientific_workflow;

mod clock;
pub mod error;

pub mod config;
pub mod observation;
pub mod persistence;
pub mod prelude;
pub mod runtime;
pub mod state;
pub mod study;
pub mod task;
pub mod ui;

pub use error::basic::WorkflowError;
pub use scientific_workflow_macros::model;

/// Loads, preflights, and executes the Workflow project rooted at
/// `project_root`.
///
/// This is the sole ordinary application entry point. Project loading and
/// Study compilation finish before Runtime receives the validated immutable
/// Study. Successful completion returns `()`; advanced integrations can load
/// a [`study::advanced::Study`] and call [`runtime::advanced::execute`] to
/// retain a read-only run summary.
pub fn run(project_root: &std::path::Path) -> Result<(), WorkflowError> {
    let study = study::advanced::Study::load(project_root)?;
    runtime::advanced::execute(study)?;
    Ok(())
}

/// Implementation details used by Workflow's declaration macros.
///
/// This namespace is public only because procedural macro expansion occurs in
/// the application crate. It is not a supported application API.
#[doc(hidden)]
pub mod __private {
    pub use crate::task::advanced::ModelRegistration;
    pub use inventory;
}
