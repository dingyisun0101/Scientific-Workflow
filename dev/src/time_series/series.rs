//! Owned in-memory collections of ordered scientific system states.
//!
//! [`StateSeries`] is the growable-array layer of time-series support. It owns
//! complete [`SystemState`] values, enforces one shared [`StateSpec`], and
//! maintains strictly increasing integer time indices. [`StateChunk`] adds the
//! small amount of rollover context needed to transfer a completed series to a
//! writer without copying its state vector or payloads.
//!
//! # Ownership
//!
//! Appending consumes a `SystemState`. A successful append moves that owner
//! directly into the underlying `Vec`; it does not clone the state or any
//! payload. A rejected append returns [`PushError`], which retains ownership of
//! the unchanged state so validation failures cannot discard scientific data.
//!
//! Converting a series into a chunk and converting a chunk back into a series
//! likewise move the original vector. Explicitly cloning a `StateSeries` is
//! different: it invokes [`SystemState::clone`] for every state and therefore
//! deep-clones populated payloads.
//!
//! **Cloning a series is intentionally expensive.** It must not be used as a
//! reference mechanism or in ordinary IO and rollover paths. Borrow the series,
//! call [`StateSeries::view`], or share an immutable owner through
//! [`Arc`](std::sync::Arc) when payload independence is not required.
//!
//! # Invariants
//!
//! Every accepted state must:
//!
//! - share the exact immutable layout allocation owned by the series, as
//!   reported by [`StateSpec::shares_layout`];
//! - have an integer time index greater than the current final index.
//!
//! Time-index gaps are valid. Physical coordinates do not define ordering and
//! may be absent. The module exposes no mutable reference to a stored complete
//! state because replacing through such a reference could bypass both
//! invariants; callers should finish constructing a state before appending it.
//!
//! # Scope
//!
//! This module performs no serialization, filesystem IO, or chunk-policy
//! evaluation. Those responsibilities belong to the format and writer layers.

use std::error::Error;
use std::fmt;

use crate::system_state::{StateSpec, SystemState};

use super::error::SeriesError;

/// A growable, homogeneous, time-ordered array of owned system states.
///
/// The series retains one canonical `StateSpec` handle even while empty. Every
/// stored state carries another cheap handle to that same immutable allocation,
/// allowing states returned by [`StateSeries::pop`] or
/// [`StateSeries::into_states`] to remain independently valid.
///
/// # Cloning cost
///
/// [`Clone::clone`] creates an independent vector and deep-clones every
/// populated payload. For a lightweight read-only reference, use
/// [`StateSeries::view`] instead.
pub struct StateSeries {
    spec: StateSpec,
    states: Vec<SystemState>,
}

impl StateSeries {
    /// Creates an empty series using `spec` as its canonical state layout.
    ///
    /// This allocates no state-vector storage until the first append. Use
    /// [`StateSeries::with_capacity`] when a chunk policy already provides a
    /// useful state-count estimate.
    pub fn new(spec: StateSpec) -> Self {
        Self {
            spec,
            states: Vec::new(),
        }
    }

    /// Creates an empty series with capacity for at least `capacity` states.
    ///
    /// The allocation reserves only `SystemState` owners. It does not create
    /// states, clone a specification, or allocate any scientific payload.
    pub fn with_capacity(spec: StateSpec, capacity: usize) -> Self {
        Self {
            spec,
            states: Vec::with_capacity(capacity),
        }
    }

    /// Returns the canonical immutable specification shared by accepted states.
    pub const fn spec(&self) -> &StateSpec {
        &self.spec
    }

    /// Creates a lightweight read-only view of this series.
    ///
    /// The returned SeriesRef borrows the canonical specification and state
    /// slice. Creating or copying the view does not clone a SystemState,
    /// scientific payload, field specification, or vector allocation.
    ///
    /// Prefer this operation over StateSeries::clone whenever a consumer only
    /// needs to inspect a series. The view cannot outlive this owner and cannot
    /// mutate, append, remove, or replace states.
    pub fn view(&self) -> SeriesRef<'_> {
        SeriesRef::new(&self.spec, &self.states)
    }

    /// Returns the number of states currently owned by the series.
    pub const fn len(&self) -> usize {
        self.states.len()
    }

    /// Reports whether the series currently owns no states.
    pub const fn is_empty(&self) -> bool {
        self.states.is_empty()
    }

    /// Returns the number of states the current vector allocation can hold
    /// without reallocating.
    pub const fn capacity(&self) -> usize {
        self.states.capacity()
    }

    /// Reserves capacity for at least `additional` more states.
    ///
    /// This delegates to `Vec::reserve` and therefore may reserve more than the
    /// exact request. Existing states and payload buffers are not cloned or
    /// moved out of their state owners.
    pub fn reserve(&mut self, additional: usize) {
        self.states.reserve(additional);
    }

    /// Returns a state by zero-based series position.
    pub fn get(&self, position: usize) -> Option<&SystemState> {
        self.states.get(position)
    }

    /// Returns the earliest stored state, or `None` when the series is empty.
    pub fn first(&self) -> Option<&SystemState> {
        self.states.first()
    }

    /// Returns the latest stored state, or `None` when the series is empty.
    pub fn last(&self) -> Option<&SystemState> {
        self.states.last()
    }

    /// Returns all states as one immutable contiguous slice.
    ///
    /// No mutable slice is exposed because replacing an element could insert a
    /// foreign specification or invalidate time ordering.
    pub fn states(&self) -> &[SystemState] {
        &self.states
    }

    /// Returns an iterator over states in increasing time-index order.
    pub fn iter(&self) -> std::slice::Iter<'_, SystemState> {
        self.states.iter()
    }

    /// Appends one owned state after validating specification identity and time
    /// ordering.
    ///
    /// On success, the state moves directly into the backing vector. On
    /// failure, [`PushError`] returns both the reason and the original unchanged
    /// state.
    ///
    /// # Errors
    ///
    /// Returns a rejection containing:
    ///
    /// - [`SeriesError::SpecMismatch`] when the state does not share the exact
    ///   canonical layout allocation;
    /// - [`SeriesError::NonIncreasingTime`] when its integer index is equal to
    ///   or less than the current final index.
    pub fn push(&mut self, state: SystemState) -> Result<(), PushError> {
        if !self.spec.shares_layout(state.spec()) {
            return Err(PushError::new(
                SeriesError::SpecMismatch {
                    index: state.time().index(),
                },
                state,
            ));
        }

        if let Some(previous) = self.last().map(|state| state.time().index()) {
            let next = state.time().index();
            if next <= previous {
                return Err(PushError::new(
                    SeriesError::NonIncreasingTime { previous, next },
                    state,
                ));
            }
        }

        self.states.push(state);
        Ok(())
    }

    /// Removes and returns the latest state, preserving all payload ownership.
    ///
    /// After a pop, a later append is compared with the new final state. An
    /// empty series accepts any time index from a state sharing its layout.
    pub fn pop(&mut self) -> Option<SystemState> {
        self.states.pop()
    }

    /// Drops all states while retaining the canonical specification and vector
    /// allocation for reuse.
    ///
    /// This operation drops owned payloads. Automatic writer rollover does not
    /// use `clear`; it moves the completed vector into a `StateChunk` and drops
    /// that chunk only after a successful durable commit.
    pub fn clear(&mut self) {
        self.states.clear();
    }

    /// Consumes the series and returns its complete owned state vector.
    ///
    /// The vector allocation and every state payload move unchanged. The
    /// separate canonical specification handle is dropped, while each returned
    /// state continues to own its shared `StateSpec` handle.
    pub fn into_states(self) -> Vec<SystemState> {
        self.states
    }

    /// Consumes a completed series and attaches writer rollover context.
    ///
    /// This crate-private conversion is the zero-copy boundary used after a
    /// writer replaces its active series with a new empty one.
    pub(crate) fn into_chunk(self, ordinal: u64, estimated_bytes: usize) -> StateChunk {
        StateChunk::new(ordinal, self, estimated_bytes)
    }
}

impl Clone for StateSeries {
    /// Creates a fully independent deep clone of this complete series.
    ///
    /// # Performance warning
    ///
    /// This operation clones every SystemState and therefore every populated
    /// scientific payload. Its time and memory cost scale with the complete
    /// payload volume, which may be many gigabytes. It is not a lightweight
    /// reference operation and should be avoided in calculation hot paths,
    /// writer rollover, reader delivery, logging, and temporary inspection.
    ///
    /// Use StateSeries::view for a scoped, allocation-free read-only view. When
    /// independently owned shared access is required, wrap the series in
    /// std::sync::Arc and clone the Arc; that increments only its reference
    /// count.
    ///
    /// The immutable StateSpec allocation remains shared between the two
    /// series. The state vector and populated payloads do not: they are deeply
    /// cloned so mutation of either series cannot affect the other.
    fn clone(&self) -> Self {
        Self {
            spec: self.spec.clone(),
            states: self.states.clone(),
        }
    }
}

impl fmt::Debug for StateSeries {
    /// Formats bounded structural information without traversing payloads.
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("StateSeries")
            .field("source", &self.spec.source())
            .field("states", &self.len())
            .field(
                "first_index",
                &self.first().map(|state| state.time().index()),
            )
            .field("last_index", &self.last().map(|state| state.time().index()))
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

    /// Consumes the series and iterates by moving out its owned states.
    fn into_iter(self) -> Self::IntoIter {
        self.states.into_iter()
    }
}

/// A lightweight immutable view over a canonical state specification and an
/// ordered state slice.
///
/// SeriesRef contains only two borrowed references. Both Copy and Clone copy
/// those references; neither operation clones a state, payload, specification
/// allocation, or vector. A view is therefore the preferred function-argument
/// and inspection boundary for large series.
///
/// The constructor is private so every view originates from an owner that has
/// already enforced exact shared-layout identity and increasing time indices.
/// It initially represents a complete StateSeries or StateChunk, while its
/// borrowed representation can later support validated subranges without
/// changing read-only consumers.
#[must_use = "a series view has no effect unless it is inspected"]
#[derive(Clone, Copy)]
pub struct SeriesRef<'a> {
    spec: &'a StateSpec,
    states: &'a [SystemState],
}

impl<'a> SeriesRef<'a> {
    /// Creates one invariant-preserving borrowed view.
    const fn new(spec: &'a StateSpec, states: &'a [SystemState]) -> Self {
        Self { spec, states }
    }

    /// Returns the canonical immutable specification shared by all states.
    pub const fn spec(self) -> &'a StateSpec {
        self.spec
    }

    /// Returns the number of states in the borrowed slice.
    pub const fn len(self) -> usize {
        self.states.len()
    }

    /// Reports whether the borrowed slice contains no states.
    pub const fn is_empty(self) -> bool {
        self.states.is_empty()
    }

    /// Returns a state by zero-based position in the borrowed slice.
    pub fn get(self, position: usize) -> Option<&'a SystemState> {
        self.states.get(position)
    }

    /// Returns the earliest borrowed state, or None when the view is empty.
    pub fn first(self) -> Option<&'a SystemState> {
        self.states.first()
    }

    /// Returns the latest borrowed state, or None when the view is empty.
    pub fn last(self) -> Option<&'a SystemState> {
        self.states.last()
    }

    /// Returns the complete immutable contiguous state slice.
    pub const fn states(self) -> &'a [SystemState] {
        self.states
    }

    /// Returns an iterator over states in increasing time-index order.
    pub fn iter(self) -> std::slice::Iter<'a, SystemState> {
        self.states.iter()
    }
}

impl fmt::Debug for SeriesRef<'_> {
    /// Formats bounded structural information without traversing payloads.
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("SeriesRef")
            .field("source", &self.spec.source())
            .field("states", &self.len())
            .field(
                "first_index",
                &self.first().map(|state| state.time().index()),
            )
            .field("last_index", &self.last().map(|state| state.time().index()))
            .finish_non_exhaustive()
    }
}

impl<'a> IntoIterator for SeriesRef<'a> {
    type Item = &'a SystemState;
    type IntoIter = std::slice::Iter<'a, SystemState>;

    /// Iterates over the borrowed states without cloning the view or payloads.
    fn into_iter(self) -> Self::IntoIter {
        self.iter()
    }
}

/// A failed append that retains ownership of the rejected state.
///
/// This follows the ownership-preserving pattern of standard-library channel
/// send errors: validation failure does not force callers to clone beforehand
/// or lose an expensive payload. Use [`PushError::into_parts`] to recover both
/// values after inspecting borrowed context through [`PushError::error`] and
/// [`PushError::state`].
#[derive(Debug)]
pub struct PushError {
    error: SeriesError,
    state: SystemState,
}

impl PushError {
    /// Creates one ownership-preserving append rejection.
    fn new(error: SeriesError, state: SystemState) -> Self {
        Self { error, state }
    }

    /// Returns the validation failure that rejected the state.
    pub const fn error(&self) -> &SeriesError {
        &self.error
    }

    /// Returns the unchanged rejected state by shared reference.
    pub const fn state(&self) -> &SystemState {
        &self.state
    }

    /// Consumes the rejection and returns its reason and original state.
    ///
    /// The returned state owns the same payload allocations passed to
    /// [`StateSeries::push`]; no clone occurs on either success or failure.
    pub fn into_parts(self) -> (SeriesError, SystemState) {
        (self.error, self.state)
    }
}

impl fmt::Display for PushError {
    /// Delegates user-facing formatting to the underlying `SeriesError`.
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        fmt::Display::fmt(&self.error, formatter)
    }
}

impl Error for PushError {
    /// Exposes the underlying `SeriesError` for standard error-chain traversal.
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        Some(&self.error)
    }
}

/// An owned completed series plus its writer-assigned chunk context.
///
/// A chunk wraps `StateSeries` instead of duplicating its specification and
/// vector fields. It performs no IO and does not imply that data has been
/// committed; the writer owns that lifecycle decision.
pub struct StateChunk {
    ordinal: u64,
    series: StateSeries,
    estimated_bytes: usize,
}

impl StateChunk {
    /// Attaches rollover metadata to one completed owned series.
    fn new(ordinal: u64, series: StateSeries, estimated_bytes: usize) -> Self {
        Self {
            ordinal,
            series,
            estimated_bytes,
        }
    }

    /// Returns the zero-based ordinal assigned by the writer.
    pub const fn ordinal(&self) -> u64 {
        self.ordinal
    }

    /// Returns the immutable specification shared by every state in the chunk.
    pub const fn spec(&self) -> &StateSpec {
        self.series.spec()
    }

    /// Creates a lightweight read-only view over this chunk's states.
    ///
    /// The returned view borrows the wrapped series and carries no ordinal or
    /// byte-estimate context. Use the chunk's own accessors when that writer
    /// metadata is also required.
    pub fn view(&self) -> SeriesRef<'_> {
        self.series.view()
    }

    /// Returns the number of states in the chunk.
    pub const fn len(&self) -> usize {
        self.series.len()
    }

    /// Reports whether the chunk contains no states.
    ///
    /// Writers normally avoid committing empty chunks, but representing one is
    /// valid and keeps the ownership type total for internal transformations.
    pub const fn is_empty(&self) -> bool {
        self.series.is_empty()
    }

    /// Returns the running in-memory byte estimate captured at rollover.
    ///
    /// This is a policy hint, not the actual encoded file length recorded in
    /// `series.json` after a successful write.
    pub const fn estimated_bytes(&self) -> usize {
        self.estimated_bytes
    }

    /// Returns the first authoritative integer time index, if present.
    pub fn first_index(&self) -> Option<u64> {
        self.series.first().map(|state| state.time().index())
    }

    /// Returns the last authoritative integer time index, if present.
    pub fn last_index(&self) -> Option<u64> {
        self.series.last().map(|state| state.time().index())
    }

    /// Returns a state by zero-based position within this chunk.
    pub fn get(&self, position: usize) -> Option<&SystemState> {
        self.series.get(position)
    }

    /// Returns all chunk states as one immutable contiguous slice.
    pub fn states(&self) -> &[SystemState] {
        self.series.states()
    }

    /// Returns an iterator over states in increasing integer time order.
    pub fn iter(&self) -> std::slice::Iter<'_, SystemState> {
        self.series.iter()
    }

    /// Consumes the chunk and returns the original owned series.
    ///
    /// Ordinal and estimate context are discarded; the state vector and all
    /// payload allocations move unchanged.
    pub fn into_series(self) -> StateSeries {
        self.series
    }
}

impl fmt::Debug for StateChunk {
    /// Formats bounded chunk metadata without traversing scientific payloads.
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("StateChunk")
            .field("ordinal", &self.ordinal)
            .field("states", &self.len())
            .field("estimated_bytes", &self.estimated_bytes)
            .field("first_index", &self.first_index())
            .field("last_index", &self.last_index())
            .finish_non_exhaustive()
    }
}

impl<'a> IntoIterator for &'a StateChunk {
    type Item = &'a SystemState;
    type IntoIter = std::slice::Iter<'a, SystemState>;

    /// Iterates over borrowed chunk states without exposing replacement.
    fn into_iter(self) -> Self::IntoIter {
        self.iter()
    }
}
