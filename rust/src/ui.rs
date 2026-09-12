//! Sole automatic presentation interface for runtime-owned execution progress.
//!
//! Applications configure and drive no UI objects. Runtime owns the observer
//! and lifecycle-fact contracts; crate composition attaches UI's inferred
//! required interactive dashboard.

mod command;
mod live_log;
mod session;
mod state;
mod terminal;
mod usage;

pub(crate) use session::UiSession;
