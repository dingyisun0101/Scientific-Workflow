//! Sole automatic presentation interface for runtime-owned execution progress.
//!
//! Applications configure and drive no UI objects. Study owns an inferred
//! immutable plan, Runtime publishes lifecycle facts, and UI selects either
//! its interactive dashboard or its noninteractive plain renderer.

mod command;
mod event;
mod plan;
mod session;
mod state;
mod terminal;

pub(crate) use event::UiEvent;
pub(crate) use plan::UiPlan;
pub(crate) use session::{TaskUi, UiSession};
