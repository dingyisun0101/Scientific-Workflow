//! Execution boundaries for workflow runs.
//!
//! `ExecutionScope` owns the execution directory lifecycle:
//!   - creation mode (`open_or_create`, `create_generated`, `create_named`),
//!   - path derivation for per-task recordings,
//!   - and opening of previously used scopes.
//!
//! # Boundary
//!
//! This module is intentionally filesystem-only. It does not define task
//! policies, scheduling, artifact identity rules, or payload serialization. The
//! downstream caller owns those concerns and consumes the directories this module
//! provides.

mod error;
mod scope;

pub use error::ExecutionScopeError;
pub use scope::ExecutionScope;
