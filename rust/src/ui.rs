//! Sole automatic presentation interface for runtime-owned execution progress.
//!
//! Applications configure and drive no UI objects. Runtime owns the observer
//! and lifecycle-fact contracts; crate composition attaches UI's inferred
//! automatic dashboard or noninteractive plain renderer.

mod command;
mod plan;
mod session;
mod state;
mod terminal;

pub(crate) use session::UiSession;
