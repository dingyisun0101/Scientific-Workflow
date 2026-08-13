//! Execution scope management for scientific workflows.
//!
//! `ExecutionScope` centralizes filesystem scope creation for one project
//! execution and all task recordings.

mod error;
mod scope;

pub use error::ExecutionScopeError;
pub use scope::ExecutionScope;
