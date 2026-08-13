//! Narrow end-user imports grouped by responsibility.
//!
//! Scientific configuration, state, storage, and artifact APIs live in
//! [`basics`]. Task, phase, scheduling, display, and cancellation APIs live in
//! [`runtime`]. Import runtime management only at orchestration boundaries.

pub mod basics;
pub mod runtime;
