//! Process and filesystem execution boundaries for workflow runs.
//!
//! [`ReplicateExecutor`] starts the current executable once per replicate and
//! gives every worker a [`ReplicateContext`] containing its isolated scope and
//! lazy seed deriver. [`ExecutionScope`] owns the directory lifecycle:
//!   - creation mode (`open_or_create`, `create_generated`, `create_named`),
//!   - path derivation for per-task recordings,
//!   - and opening of previously used scopes.
//!
//! # Boundary
//!
//! This module owns only process-level replicate isolation and filesystem
//! scopes. It does not define task policies, phase scheduling, artifact
//! identity rules, or payload serialization. The downstream caller owns those
//! concerns and consumes the contexts and directories this module provides.

mod error;
mod replicate;
mod scope;

pub use error::ExecutionScopeError;
pub use replicate::{ReplicateContext, ReplicateExecutionError, ReplicateExecutor};
pub use scope::ExecutionScope;
