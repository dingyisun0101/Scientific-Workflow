//! Errors produced by the in-memory state-series collection.
//!
//! This module describes only failures that arise while organizing owned
//! [`SystemState`](crate::system_state::SystemState) values for analysis. JSON
//! encoding, payload reconstruction, metadata validation, filesystem access,
//! chunk management, and writer lifecycles belong to the separate storage
//! module and deliberately do not appear in [`StateSeriesError`].
//!
//! # Invariant failures
//!
//! A [`StateSeries`](super::state_series::StateSeries) accepts a state only when it
//! shares the series' exact immutable layout allocation and has a simulation
//! iteration greater than the current final iteration. These requirements make layout
//! checks constant-time and preserve one unambiguous iteration order while
//! still allowing gaps between sampled indices.
//!
//! # Analysis access failures
//!
//! Immutable state lookup follows ordinary slice conventions and returns an
//! `Option`. The narrow mutable analysis boundary needs richer diagnostics
//! because it validates both a series position and a typed state field.
//! [`StateSeriesError::PositionOutOfBounds`] identifies the former, while
//! [`StateSeriesError::PayloadAccess`] adds the series position to the original
//! [`StateError`] without discarding its source-chain information.

use thiserror::Error;

use crate::system_state::StateError;

/// A failure produced while maintaining or mutating an in-memory state series.
///
/// The enum is intentionally small and independent of persistence. Every
/// variant is either a collection invariant violation or contextualized typed
/// access into one already-stored state.
///
/// `StateSeriesError` is non-exhaustive so additional analysis invariants can be
/// introduced without forcing downstream crates to use exhaustive matches.
/// Callers should therefore retain a fallback match arm.
#[derive(Debug, Error)]
#[non_exhaustive]
pub enum StateSeriesError {
    /// The rejected state does not share the series' canonical layout.
    ///
    /// Structural equality is insufficient: accepted states must derive from
    /// the same [`SystemStateSchema`](crate::system_state::SystemStateSchema) allocation. The
    /// rejected state's iteration is retained for diagnostics without
    /// inspecting or formatting any scientific payload.
    #[error("state at iteration {iteration} does not share the series specification")]
    SchemaMismatch {
        /// Iteration carried by the rejected state.
        iteration: u64,
    },

    /// The rejected state would violate strictly increasing simulation order.
    ///
    /// Iteration gaps are permitted, but an equal or decreasing value would make
    /// ordered iteration ambiguous. Physical time is not used for ordering
    /// because it is optional and may follow an application-specific scale.
    #[error("state iteration {next} must be greater than the previous iteration {previous}")]
    NonIncreasingIteration {
        /// Iteration of the series' current final state.
        previous: u64,
        /// Iteration carried by the rejected state.
        next: u64,
    },

    /// A mutable analysis request selected no stored state.
    ///
    /// `position` is a zero-based position in the series rather than a
    /// simulation iteration. Both the attempted position and current length
    /// are recorded so callers can diagnose stale analysis selections.
    #[error("state-series position {position} is out of bounds for length {len}")]
    PositionOutOfBounds {
        /// Zero-based series position requested by the caller.
        position: usize,
        /// Number of states stored when the request was evaluated.
        len: usize,
    },

    /// Typed mutable access failed inside the selected state.
    ///
    /// The source distinguishes an undeclared key, an empty field, and a
    /// concrete payload type mismatch. Wrapping it here adds the series
    /// position while preserving [`std::error::Error::source`] traversal.
    #[error("cannot access state-series position {position}: {source}")]
    PayloadAccess {
        /// Zero-based position of the state containing the requested field.
        position: usize,
        /// Original typed access failure reported by `SystemState::payload_mut`.
        #[source]
        source: StateError,
    },
}
