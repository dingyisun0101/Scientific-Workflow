//! Automatic presentation of runtime-owned execution progress.
//!
//! Applications configure and drive no UI objects. Study owns an inferred
//! immutable plan, Runtime publishes lifecycle facts, and UI decides whether
//! the current process has an interactive terminal worth rendering to.

mod command;
mod event;
mod plan;
mod session;
mod state;
mod terminal;

/// Ordinary application-facing UI API.
///
/// This scope is intentionally empty. Interactive progress is automatic.
pub mod basic {}

/// Supported UI API for advanced users and Workflow peer subsystems.
///
/// The public surface currently adds nothing to [`basic`]. Runtime reaches the
/// private event/session boundary through crate-visible exports.
pub mod advanced {
    #[allow(unused_imports)]
    pub use super::basic::*;
    pub(crate) use super::event::UiEvent;
    pub(crate) use super::plan::UiPlan;
    pub(crate) use super::session::{TaskUi, UiSession};
}
