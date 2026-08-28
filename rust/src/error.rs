//! Complete-workflow error composition.
//!
//! The crate root exposes the ordinary facade error; detailed Study and Runtime
//! errors remain owned by their respective subsystem modules.

mod workflow;

pub use workflow::WorkflowError;
