//! Contract tests for `time_series/series.rs`.
//!
//! This suite exercises the public collection API with the crate's real
//! SystemState types.
//!
//! Coverage includes collection invariants, ownership-preserving rejection,
//! clone-free state movement, explicit deep cloning, narrow field mutation,
//! lightweight views, allocation reuse, iteration, and bounded diagnostics.
//! Codecs, chunks, encoded sizes, files, and writers are intentionally absent.

use std::error::Error as _;
use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};

use serde::{Serialize, Serializer};

use scientific_workflow::system_state::{StateError, StateSpec, SystemState, TimePoint};
use scientific_workflow::time_series::{PushError, SeriesError, StateSeries};

/// Scientific payload whose backing allocation and deep clones are observable.
#[derive(Debug)]
struct TrackedPayload {
    values: Vec<u64>,
    clones: Arc<AtomicUsize>,
}

impl TrackedPayload {
    /// Returns a payload plus an independently held clone counter.
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
    /// Records and performs the deep copy required by SystemState cloning.
    fn clone(&self) -> Self {
        self.clones.fetch_add(1, Ordering::SeqCst);
        Self {
            values: self.values.clone(),
            clones: Arc::clone(&self.clones),
        }
    }
}

impl Serialize for TrackedPayload {
    /// Serializes only scientific data, excluding test instrumentation.
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        self.values.serialize(serializer)
    }
}

/// Resolves the checked-in template independently of the process directory.
fn fixture_path() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/state.json")
}

/// Loads a new canonical shared layout.
fn load_spec() -> StateSpec {
    StateSpec::load(fixture_path()).expect("the checked-in state template must load")
}

/// Creates an empty state sharing `spec` at `index`.
fn blank_state(spec: &StateSpec, index: u64) -> SystemState {
    spec.empty(TimePoint::new(index))
}

/// Creates a populated state without cloning `payload`.
fn populated_state(spec: &StateSpec, index: u64, payload: TrackedPayload) -> SystemState {
    let mut state = blank_state(spec, index);
    drop(
        state
            .set("population", payload)
            .expect("the fixture must declare an empty population field"),
    );
    state
}

#[test]
fn construction_capacity_access_and_borrowed_iteration_follow_vec_semantics() {
    let spec = load_spec();
    let empty = StateSeries::new(spec.clone());
    assert!(empty.is_empty());
    assert_eq!(empty.len(), 0);
    assert_eq!(empty.capacity(), 0);
    assert!(empty.spec().shares_layout(&spec));
    assert!(empty.get(0).is_none());
    assert!(empty.first().is_none());
    assert!(empty.last().is_none());

    let mut series = StateSeries::with_capacity(spec.clone(), 3);
    assert!(series.capacity() >= 3);
    let original_capacity = series.capacity();
    series.reserve(5);
    assert!(series.capacity() >= original_capacity);
    assert!(series.capacity() >= series.len() + 5);
    series.push(blank_state(&spec, 2)).expect("append 2");
    series
        .push(blank_state(&spec, 5))
        .expect("index gaps are valid");

    assert_eq!(series.len(), 2);
    assert_eq!(series.get(0).map(|state| state.time().index()), Some(2));
    assert_eq!(series.get(1).map(|state| state.time().index()), Some(5));
    assert!(series.get(2).is_none());
    assert_eq!(series.first().map(|state| state.time().index()), Some(2));
    assert_eq!(series.last().map(|state| state.time().index()), Some(5));
    assert_eq!(series.states().len(), 2);
    assert_eq!(
        series
            .iter()
            .map(|state| state.time().index())
            .collect::<Vec<_>>(),
        vec![2, 5]
    );
    assert_eq!(
        (&series)
            .into_iter()
            .map(|state| state.time().index())
            .collect::<Vec<_>>(),
        vec![2, 5]
    );
}

#[test]
fn field_mut_changes_only_one_payload_and_contextualizes_every_failure() {
    let spec = load_spec();
    let (payload, clones) = TrackedPayload::new(vec![3, 5, 8]);
    let mut series = StateSeries::new(spec.clone());
    series
        .push(populated_state(&spec, 10, payload))
        .expect("append populated state");
    series
        .push(blank_state(&spec, 20))
        .expect("append blank state");

    series
        .field_mut::<TrackedPayload>(0, "population")
        .expect("mutably borrow the stored payload")
        .values
        .push(13);
    assert_eq!(
        series
            .get(0)
            .unwrap()
            .get::<TrackedPayload>("population")
            .unwrap()
            .values,
        vec![3, 5, 8, 13]
    );
    assert_eq!(series.first().unwrap().time().index(), 10);
    assert_eq!(series.last().unwrap().time().index(), 20);
    assert_eq!(clones.load(Ordering::SeqCst), 0);

    assert!(matches!(
        series.field_mut::<TrackedPayload>(2, "population"),
        Err(SeriesError::PositionOutOfBounds {
            position: 2,
            len: 2
        })
    ));
    assert!(matches!(
        series.field_mut::<TrackedPayload>(1, "population"),
        Err(SeriesError::FieldAccess {
            position: 1,
            source: StateError::MissingValue { ref field }
        }) if field == "population"
    ));
    assert!(matches!(
        series.field_mut::<Vec<u64>>(0, "population"),
        Err(SeriesError::FieldAccess {
            position: 0,
            source: StateError::TypeMismatch { ref field, .. }
        }) if field == "population"
    ));
    assert!(matches!(
        series.field_mut::<TrackedPayload>(0, "undeclared"),
        Err(SeriesError::FieldAccess {
            position: 0,
            source: StateError::UnknownField { ref field }
        }) if field == "undeclared"
    ));
}

#[test]
fn push_pop_and_rejection_preserve_payload_ownership() {
    let canonical = load_spec();
    let independent = load_spec();
    let (payload, clones) = TrackedPayload::new(vec![21, 34]);
    let original_buffer = payload.values.as_ptr();
    let mut series = StateSeries::new(canonical.clone());

    let rejection = series
        .push(populated_state(&independent, 7, payload))
        .expect_err("independent layouts must be rejected");
    assert!(matches!(
        rejection.error(),
        SeriesError::SpecMismatch { index: 7 }
    ));
    assert_eq!(rejection.state().time().index(), 7);
    assert_eq!(rejection.to_string(), rejection.error().to_string());
    assert_eq!(
        rejection.source().unwrap().to_string(),
        rejection.error().to_string()
    );
    assert!(!format!("{rejection:?}").contains("21"));

    let (reason, mut state) = rejection.into_parts();
    assert!(matches!(reason, SeriesError::SpecMismatch { index: 7 }));
    let recovered_payload = state.take::<TrackedPayload>("population").unwrap();
    assert_eq!(recovered_payload.values.as_ptr(), original_buffer);
    assert_eq!(clones.load(Ordering::SeqCst), 0);
    assert!(series.is_empty());
}

#[test]
fn successful_push_and_pop_move_the_original_buffer_without_cloning() {
    let spec = load_spec();
    let (payload, clones) = TrackedPayload::new(vec![3, 5, 8, 13]);
    let buffer = payload.values.as_ptr();
    let mut series = StateSeries::new(spec.clone());
    series
        .push(populated_state(&spec, 0, payload))
        .expect("canonical state must append");
    assert_eq!(
        series
            .first()
            .unwrap()
            .get::<TrackedPayload>("population")
            .unwrap()
            .values
            .as_ptr(),
        buffer
    );
    let mut state = series.pop().unwrap();
    let payload = state.take::<TrackedPayload>("population").unwrap();
    assert_eq!(payload.values.as_ptr(), buffer);
    assert_eq!(clones.load(Ordering::SeqCst), 0);
}

#[test]
fn non_increasing_indices_return_unchanged_states() {
    let spec = load_spec();
    let mut series = StateSeries::new(spec.clone());
    series
        .push(blank_state(&spec, 9))
        .expect("append initial state");

    for next in [9, 4] {
        let rejection = series
            .push(blank_state(&spec, next))
            .expect_err("reject index");
        assert!(matches!(
            rejection.error(),
            SeriesError::NonIncreasingTime { previous: 9, next: found } if *found == next
        ));
        let (_, state) = rejection.into_parts();
        assert_eq!(state.time().index(), next);
        assert_eq!(series.len(), 1);
    }
}

#[test]
fn explicit_clone_deep_clones_payloads_but_shares_the_layout() {
    let spec = load_spec();
    let (payload, clones) = TrackedPayload::new(vec![55, 89, 144]);
    let mut original = StateSeries::new(spec.clone());
    original
        .push(populated_state(&spec, 1, payload))
        .expect("append");
    original.push(blank_state(&spec, 2)).expect("append blank");
    let original_buffer = original
        .first()
        .unwrap()
        .get::<TrackedPayload>("population")
        .unwrap()
        .values
        .as_ptr();

    let cloned = original.clone();
    let cloned_payload = cloned
        .first()
        .unwrap()
        .get::<TrackedPayload>("population")
        .unwrap();
    assert_eq!(clones.load(Ordering::SeqCst), 1);
    assert!(cloned.spec().shares_layout(original.spec()));
    assert_ne!(cloned_payload.values.as_ptr(), original_buffer);
    assert_eq!(cloned_payload.values, vec![55, 89, 144]);
}

#[test]
fn series_ref_is_copy_and_borrows_original_states_without_cloning() {
    fn clone_value<T: Clone>(value: &T) -> T {
        value.clone()
    }
    fn assert_copy_clone<T: Copy + Clone>(_: T) {}

    let spec = load_spec();
    let (payload, clones) = TrackedPayload::new(vec![987_654_321]);
    let mut series = StateSeries::new(spec.clone());
    series
        .push(populated_state(&spec, 3, payload))
        .expect("append");
    series.push(blank_state(&spec, 8)).expect("append later");

    let view = series.view();
    assert_copy_clone(view);
    let copied = view;
    let cloned = clone_value(&view);
    assert!(view.spec().shares_layout(&spec));
    assert_eq!(view.len(), 2);
    assert!(!view.is_empty());
    assert_eq!(view.states().as_ptr(), series.states().as_ptr());
    assert_eq!(view.get(0).unwrap().time().index(), 3);
    assert!(view.get(2).is_none());
    assert_eq!(view.first().unwrap().time().index(), 3);
    assert_eq!(view.last().unwrap().time().index(), 8);
    assert_eq!(
        copied.iter().map(|s| s.time().index()).collect::<Vec<_>>(),
        vec![3, 8]
    );
    assert_eq!(
        cloned
            .into_iter()
            .map(|s| s.time().index())
            .collect::<Vec<_>>(),
        vec![3, 8]
    );
    assert_eq!(clones.load(Ordering::SeqCst), 0);

    for debug in [format!("{view:?}"), format!("{series:?}")] {
        assert!(debug.contains("states: 2"));
        assert!(debug.contains("first_index: Some(3)"));
        assert!(debug.contains("last_index: Some(8)"));
        assert!(!debug.contains("987654321"));
        assert!(!debug.contains("population"));
    }

    let empty = StateSeries::new(spec);
    let empty_view = empty.view();
    assert!(empty_view.is_empty());
    assert_eq!(empty_view.len(), 0);
    assert!(empty_view.first().is_none());
    assert!(empty_view.last().is_none());
}

#[test]
fn clear_and_owned_extraction_preserve_reusable_allocations() {
    let spec = load_spec();
    let mut reusable = StateSeries::with_capacity(spec.clone(), 4);
    reusable.push(blank_state(&spec, 100)).expect("append");
    let capacity = reusable.capacity();
    reusable.clear();
    assert!(reusable.is_empty());
    assert_eq!(reusable.capacity(), capacity);
    reusable
        .push(blank_state(&spec, 1))
        .expect("empty series accepts any index");

    let mut extracted = StateSeries::with_capacity(spec.clone(), 3);
    extracted.push(blank_state(&spec, 2)).expect("append");
    extracted.push(blank_state(&spec, 4)).expect("append");
    let buffer = extracted.states().as_ptr();
    let states = extracted.into_states();
    assert_eq!(states.as_ptr(), buffer);
    assert_eq!(
        states.iter().map(|s| s.time().index()).collect::<Vec<_>>(),
        vec![2, 4]
    );

    let mut iterable = StateSeries::new(spec.clone());
    iterable.push(blank_state(&spec, 6)).expect("append");
    iterable.push(blank_state(&spec, 9)).expect("append");
    assert_eq!(
        iterable
            .into_iter()
            .map(|s| s.time().index())
            .collect::<Vec<_>>(),
        vec![6, 9]
    );
}

#[test]
fn push_error_remains_thread_transferable() {
    fn assert_send<T: Send>() {}
    assert_send::<PushError>();
}
