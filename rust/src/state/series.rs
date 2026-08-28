//! Owned in-memory collections of scientific system states.
//!
//! [`StateSeries`] is the analysis-facing growable array for complete
//! [`SystemState`] snapshots. It owns states, preserves one canonical
//! [`SystemStateSchema`], and maintains strictly increasing state iterations.
//!
//! # Scope
//!
//! This module performs no sampling, serialization, decoding, filesystem IO,
//! chunking, or queue management. Runtime sends borrowed observations to
//! persistence; a `StateSeries` is instead built when an application or reader
//! wants an owned collection for analysis.
//!
//! # Ownership and cloning
//!
//! Successful [`StateSeries::push_state`] calls move a complete state into the
//! backing `Vec`. [`StateSeries::pop_state`], [`StateSeries::into_states`], and owned
//! iteration move those owners back out. None of these paths clones a payload.
//! A rejected append returns [`StateSeriesPushError`], which retains the unchanged state.
//!
//! Explicitly cloning a `StateSeries` is intentionally expensive: every
//! populated payload is deep-cloned through [`SystemState::clone`]. Prefer a
//! borrowed `&StateSeries` for scoped immutable access or an application-owned
//! `Arc<StateSeries>` when shared ownership is required.
//!
//! # Invariants
//!
//! Every accepted state must:
//!
//! - share the exact immutable layout allocation held by the series; and
//! - have an iteration greater than the current final iteration.
//!
//! Iteration gaps are valid. Optional physical time does not determine
//! ordering. The module never exposes `&mut SystemState`, because callers could
//! otherwise change time or replace structural state behind the collection's
//! validation boundary. [`StateSeries::payload_mut_at`] permits mutation of one
//! payload while leaving those invariants inaccessible.

use std::any::Any;
use std::error::Error;
use std::fmt;

use super::error::StateSeriesError;
use super::schema::SystemStateSchema;
use super::state::SystemState;

/// A growable homogeneous array of owned, time-ordered system states.
///
/// `spec` remains present even when the collection is empty, so later appends
/// can be checked with constant-time layout identity. Each stored state carries
/// its own cheap handle to the same immutable layout allocation and therefore
/// remains independently valid after removal from the series. Like its owned
/// states, a series is `Send` but not `Sync`; shared cross-thread ownership
/// requires external synchronization.
pub struct StateSeries {
    spec: SystemStateSchema,
    states: Vec<SystemState>,
}

impl StateSeries {
    /// Creates an empty series with no state capacity reserved.
    ///
    /// This stores the supplied specification handle but allocates no state or
    /// payload storage. Use [`StateSeries::with_capacity`] when an analysis or
    /// reader already knows an approximate state count.
    pub fn new(spec: SystemStateSchema) -> Self {
        Self {
            spec,
            states: Vec::new(),
        }
    }

    /// Creates an empty series with capacity for at least `capacity` states.
    ///
    /// The reservation covers only `SystemState` owners in the backing vector.
    /// It does not create states, duplicate the shared layout, or allocate any
    /// scientific payload.
    pub fn with_capacity(spec: SystemStateSchema, capacity: usize) -> Self {
        Self {
            spec,
            states: Vec::with_capacity(capacity),
        }
    }

    /// Returns the canonical immutable specification for this collection.
    pub fn schema(&self) -> &SystemStateSchema {
        &self.spec
    }

    /// Returns the number of states currently owned by the collection.
    pub fn len(&self) -> usize {
        self.states.len()
    }

    /// Reports whether the collection currently owns no states.
    pub fn is_empty(&self) -> bool {
        self.states.is_empty()
    }

    /// Returns the backing vector's current state-owner capacity.
    pub fn capacity(&self) -> usize {
        self.states.capacity()
    }

    /// Reserves capacity for at least `additional` more state owners.
    ///
    /// Existing states and their payload allocations remain logically
    /// unchanged. As with [`Vec::reserve`], the allocator may reserve more than
    /// the exact requested amount.
    pub fn reserve(&mut self, additional: usize) {
        self.states.reserve(additional);
    }

    /// Returns one immutable state by zero-based collection position.
    ///
    /// This follows slice conventions and returns `None` when `position` is
    /// outside the collection. The position is distinct from the state's
    /// iteration because sampled iterations may contain gaps.
    pub fn state_at(&self, position: usize) -> Option<&SystemState> {
        self.states.get(position)
    }

    /// Mutably borrows one typed payload in one stored state.
    ///
    /// This is the collection's only mutable analysis boundary. It delegates
    /// concrete type validation to [`SystemState::payload_mut`] but does not expose
    /// the containing `SystemState`; callers therefore cannot change its time,
    /// clear unrelated fields, or replace it with a foreign layout.
    ///
    /// Only one payload can be borrowed mutably at a time under ordinary Rust
    /// borrowing rules. Applications requiring coupled mutation should group
    /// the coupled values into one payload type.
    ///
    /// # Errors
    ///
    /// Returns [`StateSeriesError::PositionOutOfBounds`] when no state exists at
    /// `position`. An unknown key, empty field, or concrete type mismatch is
    /// returned as [`StateSeriesError::PayloadAccess`] with the original
    /// [`StateError`](super::error::StateError) preserved as its source.
    pub fn payload_mut_at<T>(
        &mut self,
        position: usize,
        key: &str,
    ) -> Result<&mut T, StateSeriesError>
    where
        T: Any,
    {
        let len = self.states.len();
        let state = self
            .states
            .get_mut(position)
            .ok_or(StateSeriesError::PositionOutOfBounds { position, len })?;

        state
            .payload_mut::<T>(key)
            .map_err(|source| StateSeriesError::PayloadAccess { position, source })
    }

    /// Returns the earliest stored state, or `None` when the series is empty.
    pub fn first_state(&self) -> Option<&SystemState> {
        self.states.first()
    }

    /// Returns the latest stored state, or `None` when the series is empty.
    pub fn last_state(&self) -> Option<&SystemState> {
        self.states.last()
    }

    /// Returns every state as one immutable contiguous slice.
    ///
    /// No mutable slice is exposed because element replacement could bypass
    /// both shared-layout validation and increasing-iteration validation.
    pub fn as_state_slice(&self) -> &[SystemState] {
        &self.states
    }

    /// Returns an iterator over immutable states in increasing iteration order.
    pub fn iter(&self) -> std::slice::Iter<'_, SystemState> {
        self.states.iter()
    }

    /// Appends one owned state after validating collection invariants.
    ///
    /// Success moves `state` directly into the backing vector without cloning
    /// it or any payload. Failure returns [`StateSeriesPushError`] containing the complete
    /// unchanged state, allowing the caller to recover expensive data without
    /// cloning before the operation.
    ///
    /// # Errors
    ///
    /// - [`StateSeriesError::SchemaMismatch`] if `state` does not share the exact
    ///   canonical layout allocation;
    /// - [`StateSeriesError::NonIncreasingIteration`] if its iteration is not
    ///   greater than the current final iteration.
    pub fn push_state(&mut self, state: SystemState) -> Result<(), StateSeriesPushError> {
        if !self.spec.shares_schema_instance(state.schema()) {
            return Err(StateSeriesPushError::new(
                StateSeriesError::SchemaMismatch {
                    iteration: state.time().iteration(),
                },
                state,
            ));
        }

        if let Some(previous) = self.last_state().map(|state| state.time().iteration()) {
            let next = state.time().iteration();
            if next <= previous {
                return Err(StateSeriesPushError::new(
                    StateSeriesError::NonIncreasingIteration { previous, next },
                    state,
                ));
            }
        }

        self.states.push(state);
        Ok(())
    }

    /// Removes and returns the latest state without cloning its payloads.
    ///
    /// A later append is compared with the new final state. Once empty, the
    /// series accepts any iteration from a state sharing its layout.
    pub fn pop_state(&mut self) -> Option<SystemState> {
        self.states.pop()
    }

    /// Drops every state while retaining specification and vector capacity.
    ///
    /// Stored payloads are dropped with their owning states. This method is an
    /// explicit analysis working-set operation and has no relationship to
    /// writer rollover or persistent chunks.
    pub fn clear_states(&mut self) {
        self.states.clear();
    }

    /// Consumes the series and returns its complete state vector.
    ///
    /// The vector allocation, states, and payload allocations move unchanged.
    /// Dropping the separate canonical specification handle is safe because
    /// every returned state retains its own shared handle.
    pub fn into_states(self) -> Vec<SystemState> {
        self.states
    }
}

impl Clone for StateSeries {
    /// Creates a fully independent deep copy of all states and payloads.
    ///
    /// # Performance warning
    ///
    /// Cost scales with the complete populated payload volume and may involve
    /// gigabytes of allocation and copying. This method is appropriate only
    /// when analysis requires independent mutable payload ownership. Use
    /// borrowing `&StateSeries` or an `Arc<StateSeries>` for lightweight sharing.
    /// The immutable specification allocation remains shared.
    fn clone(&self) -> Self {
        Self {
            spec: self.spec.clone(),
            states: self.states.clone(),
        }
    }
}

impl fmt::Debug for StateSeries {
    /// Formats bounded structural context without traversing payload values.
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("StateSeries")
            .field("source", &self.spec.template_path())
            .field("states", &self.len())
            .field(
                "first_iteration",
                &self.first_state().map(|state| state.time().iteration()),
            )
            .field(
                "last_iteration",
                &self.last_state().map(|state| state.time().iteration()),
            )
            .finish_non_exhaustive()
    }
}

impl<'a> IntoIterator for &'a StateSeries {
    type Item = &'a SystemState;
    type IntoIter = std::slice::Iter<'a, SystemState>;

    /// Iterates over borrowed states without exposing mutable replacement.
    fn into_iter(self) -> Self::IntoIter {
        self.iter()
    }
}

impl IntoIterator for StateSeries {
    type Item = SystemState;
    type IntoIter = std::vec::IntoIter<SystemState>;

    /// Consumes the series and moves each owned state out in iteration order.
    fn into_iter(self) -> Self::IntoIter {
        self.states.into_iter()
    }
}

/// An append failure that preserves ownership of the rejected state.
///
/// This follows the ownership behavior of standard-library channel send
/// errors: callers never need to clone expensive scientific data before an
/// operation that may reject it. The state is boxed internally so
/// `Result<(), StateSeriesPushError>` remains small on the successful hot path. The box is
/// allocated only after validation fails.
#[must_use = "the rejected SystemState remains owned by this error until recovered or dropped"]
pub struct StateSeriesPushError {
    error: StateSeriesError,
    state: Box<SystemState>,
}

impl StateSeriesPushError {
    /// Creates a failure-path owner for one unchanged rejected state.
    fn new(error: StateSeriesError, state: SystemState) -> Self {
        Self {
            error,
            state: Box::new(state),
        }
    }

    /// Returns the collection invariant that rejected the state.
    pub fn error(&self) -> &StateSeriesError {
        &self.error
    }

    /// Returns the unchanged rejected state by shared reference.
    pub fn state(&self) -> &SystemState {
        &self.state
    }

    /// Consumes the error and returns its reason and original state.
    ///
    /// Moving the state out of its failure-only outer box does not clone the
    /// state or any scientific payload allocation. The tuple follows borrowed
    /// inspection order: error first, then state.
    pub fn into_parts(self) -> (StateSeriesError, SystemState) {
        (self.error, *self.state)
    }
}

impl fmt::Debug for StateSeriesPushError {
    /// Formats the reason and structural state context without payload values.
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("StateSeriesPushError")
            .field("error", &self.error)
            .field("state", &self.state)
            .finish_non_exhaustive()
    }
}

impl fmt::Display for StateSeriesPushError {
    /// Delegates user-facing formatting to the collection invariant failure.
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        fmt::Display::fmt(&self.error, formatter)
    }
}

impl Error for StateSeriesPushError {
    /// Exposes the underlying collection error for standard source traversal.
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        Some(&self.error)
    }
}
