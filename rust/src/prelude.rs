//! Narrow end-user imports grouped by responsibility.
//!
//! Scientific configuration, state, storage, and artifact APIs live in
//! [`basics`]. Task, phase, study, display, and cancellation APIs live in
//! [`study`]. Import study management only at orchestration boundaries.
//!
//! # Boundary
//! 
//! The prelude modules define import convenience only. They do not add new API
//! behavior; they preserve stable ownership boundaries by limiting wildcard usage.

pub mod basics;
#[path = "study.rs"]
pub mod study;
