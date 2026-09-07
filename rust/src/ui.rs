//! Sole automatic presentation interface for runtime-owned execution progress.
//!
//! Applications configure and drive no UI objects. Runtime owns the observer
//! and lifecycle-fact contracts; crate composition attaches UI's inferred
//! automatic dashboard or noninteractive plain renderer.

mod command;
mod live_log;
mod plan;
mod session;
mod state;
mod terminal;
mod usage;

pub(crate) use session::UiSession;
