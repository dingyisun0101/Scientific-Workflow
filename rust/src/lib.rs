//! Configuration-driven scientific workflow execution.
//!
//! An ordinary application defines a registered [`ExecutionUnit`]
//! implementations, writes `wf_configs/study.json`, its declared named
//! state-schema documents, and the central `wf_configs/parameters.json`, then
//! calls [`run`] with the project root. Workflow infers task
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
//! An execution unit carries its stable manifest key at its implementation:
//!
//! ```
//! use serde::Deserialize;
//! use scientific_workflow::prelude::*;
//!
//! #[derive(Deserialize)]
//! struct Constants { initial: u64, steps: u64 }
//!
//! struct ExampleUnit { state: SystemState, target_iteration: u64 }
//!
//! #[scientific_workflow::execution_unit("example")]
//! impl ExecutionUnit for ExampleUnit {
//!     type Constants = Constants;
//!
//!     fn initialize(
//!         constants: Constants,
//!         schema: &SystemStateSchema,
//!         _context: &InitializationContext,
//!     ) -> UnitResult<Self> {
//!         let mut state = schema.create_empty_state(StateTime::from_iteration(0));
//!         state.initialize_payload("population", constants.initial)?;
//!         state.initialize_payload("cumulative_births", 0_u64)?;
//!         Ok(Self {
//!             state,
//!             target_iteration: constants.steps,
//!         })
//!     }
//!
//!     fn member_count(&self) -> usize { 1 }
//!     fn member(&self, index: usize) -> Option<MemberView<'_>> {
//!         (index == 0).then(|| MemberView::new(
//!             "example",
//!             &self.state,
//!             (self.state.time().iteration() >= self.target_iteration)
//!                 .then_some(MemberCompletion::without_reason()),
//!             Some(self.target_iteration),
//!         ))
//!     }
//!     fn step(&mut self) -> UnitResult {
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
//! - [`config`] alone reads and parses project JSON and supplies typed execution-unit
//!   constants.
//! - [`study`] composes parsed declarations, state semantics, and compiled execution-unit
//!   registrations into immutable, output-free intent.
//! - [`runtime`] consumes a completed Study and owns active execution/output.
//! - The crate root owns the execution-unit contract and automatic observation boundaries.
//! - [`state`] owns canonical scientific state and schema.
//! - [`observation`] owns scientific observation meaning, not persistence mechanics.
//! - [`persistence`] owns automatic durable output and verified reading.
//! - The private UI subsystem owns automatic terminal presentation of Runtime facts.
//! - [`WorkflowError`] composes complete-workflow Study/Runtime failures.
//!
//! The crate-level [`run`] facade performs the sole ordinary transition from
//! project root to Study to Runtime. `Study` is the ultimate coordinator of
//! declared intent; runtime is the ultimate coordinator of active execution.
//! Inspection and embedding integrations import APIs directly from their
//! owning module roots. [`prelude`] contains only ordinary unit-authoring APIs
//! and crate conveniences.
//!
//! Persistence write construction and output allocation are internal and are
//! not available to execution-unit authors.
//!
//! This crate is pre-1.0 test software and may make coordinated API changes.

#![deny(missing_docs, rustdoc::broken_intra_doc_links)]

extern crate self as scientific_workflow;

mod clock;
mod error;

pub mod config;
pub mod observation;
pub mod persistence;
pub mod prelude;
pub mod runtime;
pub mod state;
pub mod study;
mod task;
mod ui;

pub use error::WorkflowError;
pub use scientific_workflow_macros::execution_unit;
pub use task::{
    ExecutionUnit, InitializationContext, MemberCompletion, MemberView, SeedError, UnitResult,
};

/// Loads, preflights, and executes the Workflow project rooted at
/// `project_root`.
///
/// This is the sole ordinary application entry point. Project loading and
/// Study compilation finish before Runtime receives the validated immutable
/// Study. Successful completion returns `()`; advanced integrations can load
/// an embedding integration can load a [`study::Study`] and call
/// [`runtime::execute`] to
/// retain a read-only run summary.
pub fn run(project_root: &std::path::Path) -> Result<(), WorkflowError> {
    let study = study::Study::load(project_root)?;
    runtime::execute(study)?;
    Ok(())
}

/// Implementation details used by Workflow's declaration macros.
///
/// This namespace is public only because procedural macro expansion occurs in
/// the application crate. It is not a supported application API.
#[doc(hidden)]
pub mod __private {
    pub use crate::task::ExecutionUnitRegistration;
    pub use inventory;
}
