//! Contract tests for the private time_series/series.rs implementation.
//!
//! The public time-series facade remains intentionally unwired while its
//! component files are reviewed individually. This suite therefore includes
//! the production series and error modules directly, while its test-only
//! system_state facade re-exports the crate's real public SystemState types at
//! the path expected by those production modules.
//!
//! These tests verify:
//!
//! - construction, reservation, indexing, and immutable iteration;
//! - exact shared-layout identity rather than structural schema equality;
//! - strictly increasing integer indices with gaps permitted;
//! - recovery of an unchanged rejected state from series::PushError;
//! - clone-free payload movement through append, removal, iteration, and chunk
//!   ownership boundaries;
//! - the currently documented deep-clone behavior of StateSeries::clone;
//! - lightweight SeriesRef copying, cloning, access, and chunk projection;
//! - destructive clearing with allocation reuse;
//! - bounded diagnostics that never format scientific payload contents.

use std::error::Error as _;
use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};

/// Reproduces the crate-root path imported by the private production modules.
///
/// Re-exporting instead of substituting test doubles ensures that series
/// validation exercises the real StateSpec, SystemState, and TimePoint
/// implementations, including their internal shared-layout identity.
mod system_state {
    pub use scientific_workflow::system_state::*;
}

#[path = "../../src/time_series/error.rs"]
#[allow(dead_code)]
mod error;

#[path = "../../src/time_series/series.rs"]
mod series;

use error::SeriesError;
use series::StateSeries;
use system_state::{StateSpec, SystemState, TimePoint};

/// An owned scientific payload whose explicit deep clones are observable.
///
/// Pointer comparisons on values distinguish ownership movement from buffer
/// copying. The shared counter independently detects calls to Clone, avoiding
/// assumptions about allocator behavior.
#[derive(Debug)]
struct TrackedPayload {
    values: Vec<u64>,
    clones: Arc<AtomicUsize>,
}

impl TrackedPayload {
    /// Creates a payload and returns a separate observer for clone calls.
    fn new(values: Vec<u64>) -> (Self, Arc<AtomicUsize>) {
        let clones = Arc::new(AtomicUsize::new(0));
        (
            Self {
                values,
                clones: Arc::clone(&clones),
            },
            clones,
        )
    }
}

impl Clone for TrackedPayload {
    /// Records the explicit deep clone required by cloned states or series.
    fn clone(&self) -> Self {
        self.clones.fetch_add(1, Ordering::SeqCst);
        Self {
            values: self.values.clone(),
            clones: Arc::clone(&self.clones),
        }
    }
}

/// Returns the repository fixture through an absolute Cargo-derived path.
fn fixture_path() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/state.json")
}

/// Loads a fresh canonical specification from the actual JSON fixture.
fn load_spec() -> StateSpec {
    StateSpec::load(fixture_path()).expect("the checked-in state template must load")
}

/// Creates one blank state sharing spec and carrying the supplied index.
fn blank_state(spec: &StateSpec, index: u64) -> SystemState {
    spec.empty(TimePoint::new(index))
}

/// Creates one populated state without cloning the supplied payload.
fn populated_state(spec: &StateSpec, index: u64, payload: TrackedPayload) -> SystemState {
    let mut state = blank_state(spec, index);
    state
        .set("population", payload)
        .expect("the fixture must declare the population field");
    state
}

#[test]
fn construction_reservation_and_borrowed_access_are_consistent() {
    let spec = load_spec();
    let mut series = StateSeries::with_capacity(spec.clone(), 3);

    assert!(series.is_empty());
    assert_eq!(series.len(), 0);
    assert!(series.capacity() >= 3);
    assert!(series.spec().shares_layout(&spec));
    assert!(series.get(0).is_none());
    assert!(series.first().is_none());
    assert!(series.last().is_none());

    let previous_capacity = series.capacity();
    series.reserve(5);
    assert!(series.capacity() >= previous_capacity);
    assert!(series.capacity() >= series.len() + 5);

    series
        .push(blank_state(&spec, 2))
        .expect("the first shared-layout state must append");
    series
        .push(blank_state(&spec, 5))
        .expect("strictly increasing indices may contain gaps");

    assert_eq!(series.len(), 2);
    assert_eq!(series.get(0).map(|state| state.time().index()), Some(2));
    assert_eq!(series.first().map(|state| state.time().index()), Some(2));
    assert_eq!(series.last().map(|state| state.time().index()), Some(5));

    let through_iter: Vec<_> = series.iter().map(|state| state.time().index()).collect();
    let through_borrow: Vec<_> = (&series)
        .into_iter()
        .map(|state| state.time().index())
        .collect();
    assert_eq!(through_iter, vec![2, 5]);
    assert_eq!(through_borrow, through_iter);
    assert_eq!(series.states().len(), 2);
}

#[test]
fn lightweight_views_borrow_complete_series_without_cloning_payloads() {
    /// Invokes Clone through a generic boundary so the test verifies the trait
    /// contract without triggering a clone-on-Copy lint at the call site.
    fn clone_value<T: Clone>(value: &T) -> T {
        value.clone()
    }

    /// Requires both lightweight marker traits at compile time.
    fn assert_copy_clone<T: Copy + Clone>(_: T) {}

    let spec = load_spec();
    let (payload, clones) = TrackedPayload::new(vec![987_654_321, 610]);
    let payload_buffer = payload.values.as_ptr();
    let mut series = StateSeries::new(spec.clone());
    series
        .push(populated_state(&spec, 3, payload))
        .expect("the first populated state must append");
    series
        .push(blank_state(&spec, 8))
        .expect("the later blank state may follow an index gap");

    let view = series.view();
    assert_copy_clone(view);
    let copied = view;
    let cloned = clone_value(&view);

    assert_eq!(clones.load(Ordering::SeqCst), 0);
    assert!(view.spec().shares_layout(&spec));
    assert_eq!(view.len(), 2);
    assert!(!view.is_empty());
    assert_eq!(view.states().as_ptr(), series.states().as_ptr());
    assert_eq!(view.get(0).map(|state| state.time().index()), Some(3));
    assert!(view.get(2).is_none());
    assert_eq!(view.first().map(|state| state.time().index()), Some(3));
    assert_eq!(view.last().map(|state| state.time().index()), Some(8));
    assert_eq!(
        copied
            .iter()
            .map(|state| state.time().index())
            .collect::<Vec<_>>(),
        vec![3, 8]
    );
    assert_eq!(
        cloned
            .into_iter()
            .map(|state| state.time().index())
            .collect::<Vec<_>>(),
        vec![3, 8]
    );
    assert_eq!(
        view.get(0)
            .expect("the populated state must remain borrowed")
            .get::<TrackedPayload>("population")
            .expect("the view must expose the original typed payload")
            .values
            .as_ptr(),
        payload_buffer
    );

    let debug = format!("{view:?}");
    assert!(debug.contains("SeriesRef"));
    assert!(debug.contains("states: 2"));
    assert!(debug.contains("first_index: Some(3)"));
    assert!(debug.contains("last_index: Some(8)"));
    assert!(!debug.contains("987654321"));
    assert!(!debug.contains("population"));
    assert_eq!(clones.load(Ordering::SeqCst), 0);

    let chunk = series.into_chunk(11, 4_096);
    let chunk_view = chunk.view();
    assert_eq!(chunk.ordinal(), 11);
    assert_eq!(chunk.estimated_bytes(), 4_096);
    assert_eq!(chunk_view.states().as_ptr(), chunk.states().as_ptr());
    assert!(chunk_view.spec().shares_layout(&spec));
    assert_eq!(
        chunk_view
            .iter()
            .map(|state| state.time().index())
            .collect::<Vec<_>>(),
        vec![3, 8]
    );
    assert_eq!(clones.load(Ordering::SeqCst), 0);

    let empty = StateSeries::new(spec);
    let empty_view = empty.view();
    assert!(empty_view.is_empty());
    assert_eq!(empty_view.len(), 0);
    assert!(empty_view.first().is_none());
    assert!(empty_view.last().is_none());
    assert_eq!(empty_view.iter().count(), 0);
}

#[test]
fn append_and_pop_move_the_original_payload_without_cloning() {
    let spec = load_spec();
    let (payload, clones) = TrackedPayload::new(vec![3, 5, 8, 13]);
    let original_buffer = payload.values.as_ptr();
    let state = populated_state(&spec, 0, payload);
    let mut series = StateSeries::new(spec);

    series
        .push(state)
        .expect("a state derived from the canonical spec must append");
    assert_eq!(clones.load(Ordering::SeqCst), 0);
    assert_eq!(
        series
            .first()
            .expect("the appended state must exist")
            .get::<TrackedPayload>("population")
            .expect("the payload must remain typed")
            .values
            .as_ptr(),
        original_buffer
    );

    let mut recovered = series.pop().expect("pop must return the appended state");
    let recovered_payload = recovered
        .take::<TrackedPayload>("population")
        .expect("take must return the original concrete payload");

    assert_eq!(recovered_payload.values.as_ptr(), original_buffer);
    assert_eq!(recovered_payload.values, vec![3, 5, 8, 13]);
    assert_eq!(clones.load(Ordering::SeqCst), 0);
    assert!(series.is_empty());
}

#[test]
fn structurally_equal_but_independently_loaded_specs_are_rejected_without_data_loss() {
    let canonical = load_spec();
    let independent = load_spec();
    assert!(!canonical.shares_layout(&independent));

    let (payload, clones) = TrackedPayload::new(vec![21, 34]);
    let original_buffer = payload.values.as_ptr();
    let rejected_state = populated_state(&independent, 7, payload);
    let mut series = StateSeries::new(canonical);

    let rejection = series
        .push(rejected_state)
        .expect_err("structural equality must not replace shared identity");
    assert!(matches!(
        rejection.error(),
        SeriesError::SpecMismatch { index: 7 }
    ));
    assert_eq!(rejection.state().time().index(), 7);
    assert_eq!(
        rejection
            .state()
            .get::<TrackedPayload>("population")
            .expect("the rejected state must retain its payload")
            .values
            .as_ptr(),
        original_buffer
    );
    assert_eq!(
        rejection.to_string(),
        "state at time index 7 does not share the series specification"
    );
    assert!(rejection.source().is_some());

    let (reason, mut state) = rejection.into_parts();
    assert!(matches!(reason, SeriesError::SpecMismatch { index: 7 }));
    let payload = state
        .take::<TrackedPayload>("population")
        .expect("the caller must recover the unchanged rejected owner");
    assert_eq!(payload.values.as_ptr(), original_buffer);
    assert_eq!(clones.load(Ordering::SeqCst), 0);
    assert!(series.is_empty());
}

#[test]
fn ordering_rejects_duplicates_and_regressions_but_accepts_gaps() {
    let spec = load_spec();
    let mut series = StateSeries::new(spec.clone());
    series
        .push(blank_state(&spec, 10))
        .expect("the first index establishes ordering");

    for next in [10, 9] {
        let rejection = series
            .push(blank_state(&spec, next))
            .expect_err("non-increasing indices must be rejected");
        assert!(matches!(
            rejection.error(),
            SeriesError::NonIncreasingTime {
                previous: 10,
                next: rejected,
            } if *rejected == next
        ));
        let (_, state) = rejection.into_parts();
        assert_eq!(state.time().index(), next);
    }

    series
        .push(blank_state(&spec, 15))
        .expect("an increasing index gap must be accepted");
    assert_eq!(
        series
            .iter()
            .map(|state| state.time().index())
            .collect::<Vec<_>>(),
        vec![10, 15]
    );

    let removed = series.pop().expect("the last state must be removable");
    assert_eq!(removed.time().index(), 15);
    series
        .push(blank_state(&spec, 11))
        .expect("ordering must compare against the new last state after pop");
}

#[test]
fn consuming_collection_paths_preserve_state_and_payload_allocations() {
    let spec = load_spec();
    let (payload, clones) = TrackedPayload::new(vec![55, 89]);
    let payload_buffer = payload.values.as_ptr();
    let mut series = StateSeries::new(spec.clone());
    series
        .push(populated_state(&spec, 1, payload))
        .expect("the populated state must append");
    let state_buffer = series.states().as_ptr();

    let states = series.into_states();
    assert_eq!(states.as_ptr(), state_buffer);
    assert_eq!(
        states[0]
            .get::<TrackedPayload>("population")
            .expect("the moved state must retain its payload")
            .values
            .as_ptr(),
        payload_buffer
    );
    assert_eq!(clones.load(Ordering::SeqCst), 0);

    let mut rebuilt = StateSeries::new(states[0].spec().clone());
    for state in states {
        rebuilt
            .push(state)
            .expect("moved states must retain the canonical layout");
    }
    let indices: Vec<_> = rebuilt
        .into_iter()
        .map(|state| state.time().index())
        .collect();
    assert_eq!(indices, vec![1]);
    assert_eq!(clones.load(Ordering::SeqCst), 0);
}

#[test]
fn explicit_series_clone_deep_clones_payloads_and_preserves_independence() {
    let spec = load_spec();
    let (payload, clones) = TrackedPayload::new(vec![1, 2, 3]);
    let original_buffer = payload.values.as_ptr();
    let mut original = StateSeries::new(spec.clone());
    original
        .push(populated_state(&spec, 4, payload))
        .expect("the populated state must append");

    let cloned = original.clone();
    assert_eq!(clones.load(Ordering::SeqCst), 1);

    let mut cloned_state = cloned
        .into_states()
        .pop()
        .expect("the cloned series must contain a cloned state");
    let cloned_payload = cloned_state
        .get_mut::<TrackedPayload>("population")
        .expect("the cloned payload must retain its concrete type");
    assert_ne!(cloned_payload.values.as_ptr(), original_buffer);
    cloned_payload.values[0] = 99;

    let original_payload = original
        .first()
        .expect("the original state must remain present")
        .get::<TrackedPayload>("population")
        .expect("the original payload must remain present");
    assert_eq!(original_payload.values, vec![1, 2, 3]);
}

#[test]
fn clear_drops_states_but_retains_layout_and_vector_allocation() {
    let spec = load_spec();
    let mut series = StateSeries::with_capacity(spec.clone(), 4);
    series
        .push(blank_state(&spec, 1))
        .expect("the shared-layout state must append");
    let allocation = series.states().as_ptr();
    let capacity = series.capacity();

    series.clear();

    assert!(series.is_empty());
    assert_eq!(series.capacity(), capacity);
    assert_eq!(series.states().as_ptr(), allocation);
    assert!(series.spec().shares_layout(&spec));
    series
        .push(blank_state(&spec, 0))
        .expect("a cleared series must accept a fresh first index");

    let debug = format!("{series:?}");
    assert!(debug.contains("StateSeries"));
    assert!(debug.contains("states: 1"));
    assert!(debug.contains("first_index: Some(0)"));
    assert!(!debug.contains("population"));
}

#[test]
fn chunk_conversion_moves_the_original_series_and_reports_bounded_context() {
    let spec = load_spec();
    let (payload, clones) = TrackedPayload::new(vec![144, 233]);
    let payload_buffer = payload.values.as_ptr();
    let mut series = StateSeries::new(spec.clone());
    series
        .push(populated_state(&spec, 20, payload))
        .expect("the first state must append");
    series
        .push(blank_state(&spec, 25))
        .expect("the second state may follow a gap");
    let state_buffer = series.states().as_ptr();

    let chunk = series.into_chunk(6, 65_536);
    assert_eq!(chunk.ordinal(), 6);
    assert_eq!(chunk.len(), 2);
    assert!(!chunk.is_empty());
    assert_eq!(chunk.estimated_bytes(), 65_536);
    assert_eq!(chunk.first_index(), Some(20));
    assert_eq!(chunk.last_index(), Some(25));
    assert!(chunk.spec().shares_layout(&spec));
    assert_eq!(chunk.states().as_ptr(), state_buffer);
    assert_eq!(chunk.get(1).map(|state| state.time().index()), Some(25));
    assert_eq!(
        chunk
            .iter()
            .map(|state| state.time().index())
            .collect::<Vec<_>>(),
        vec![20, 25]
    );
    assert_eq!((&chunk).into_iter().count(), 2);
    assert_eq!(
        chunk
            .get(0)
            .expect("the first chunk state must exist")
            .get::<TrackedPayload>("population")
            .expect("the moved payload must remain typed")
            .values
            .as_ptr(),
        payload_buffer
    );

    let debug = format!("{chunk:?}");
    assert!(debug.contains("StateChunk"));
    assert!(debug.contains("ordinal: 6"));
    assert!(debug.contains("estimated_bytes: 65536"));
    assert!(!debug.contains("144"));
    assert_eq!(clones.load(Ordering::SeqCst), 0);

    let recovered = chunk.into_series();
    assert_eq!(recovered.states().as_ptr(), state_buffer);
    assert_eq!(clones.load(Ordering::SeqCst), 0);

    let empty = StateSeries::new(spec).into_chunk(7, 0);
    assert!(empty.is_empty());
    assert_eq!(empty.first_index(), None);
    assert_eq!(empty.last_index(), None);
}
