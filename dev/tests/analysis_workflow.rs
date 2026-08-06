//! Logged integration workflow for in-memory state-series analysis.

use std::error::Error as _;
use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};

use scientific_workflow::system_state::{StateError, StateSpec, TimePoint};
use scientific_workflow::time_series::{SeriesError, StateSeries};
use serde::Serialize;

/// Payload whose clone counter makes expensive series cloning observable.
#[derive(Debug, Serialize)]
struct Sample {
    values: Vec<u64>,
    #[serde(skip)]
    clones: Arc<AtomicUsize>,
}

impl Clone for Sample {
    fn clone(&self) -> Self {
        self.clones.fetch_add(1, Ordering::Relaxed);
        Self {
            values: self.values.clone(),
            clones: Arc::clone(&self.clones),
        }
    }
}

fn fixture_path() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/state.json")
}

fn sample_state(
    spec: &StateSpec,
    index: u64,
    values: Vec<u64>,
    clones: &Arc<AtomicUsize>,
) -> scientific_workflow::system_state::SystemState {
    let mut state = spec.empty(TimePoint::new(index));
    assert!(
        state
            .set(
                "population",
                Sample {
                    values,
                    clones: Arc::clone(clones),
                },
            )
            .unwrap()
            .is_none()
    );
    state
}

#[test]
fn ordered_analysis_preserves_ownership_and_invariants() {
    let spec = StateSpec::load(fixture_path()).expect("real state fixture must load");
    let clones = Arc::new(AtomicUsize::new(0));

    let empty = StateSeries::new(spec.clone());
    assert!(empty.is_empty());
    assert_eq!(empty.len(), 0);
    assert_eq!(empty.capacity(), 0);
    assert!(empty.spec().shares_layout(&spec));

    let initial = sample_state(&spec, 0, vec![2, 3, 5, 7], &clones);
    let initial_pointer = initial.get::<Sample>("population").unwrap().values.as_ptr();
    let later = sample_state(&spec, 4, vec![11, 13], &clones);

    let mut series = StateSeries::with_capacity(spec.clone(), 2);
    assert!(series.capacity() >= 2);
    series.reserve(2);
    let reserved_capacity = series.capacity();
    series.push(initial).unwrap();
    series.push(later).unwrap();

    assert_eq!(series.len(), 2);
    assert!(!series.is_empty());
    assert_eq!(series.get(0).unwrap().time().index(), 0);
    assert!(series.get(2).is_none());
    assert_eq!(series.first().unwrap().time().index(), 0);
    assert_eq!(series.last().unwrap().time().index(), 4);
    assert_eq!(series.states().len(), 2);
    assert_eq!(series.iter().count(), 2);
    assert_eq!(
        series
            .first()
            .unwrap()
            .get::<Sample>("population")
            .unwrap()
            .values
            .as_ptr(),
        initial_pointer
    );
    assert_eq!(clones.load(Ordering::Relaxed), 0);

    series
        .field_mut::<Sample>(0, "population")
        .unwrap()
        .values
        .push(17);
    assert_eq!(
        series
            .first()
            .unwrap()
            .get::<Sample>("population")
            .unwrap()
            .values,
        vec![2, 3, 5, 7, 17]
    );

    let bounds = series
        .field_mut::<Sample>(9, "population")
        .expect_err("missing collection position must fail");
    assert!(matches!(
        bounds,
        SeriesError::PositionOutOfBounds {
            position: 9,
            len: 2
        }
    ));
    let mismatch = series
        .field_mut::<String>(0, "population")
        .expect_err("wrong concrete payload type must fail");
    assert!(matches!(
        &mismatch,
        SeriesError::FieldAccess {
            position: 0,
            source: StateError::TypeMismatch { .. }
        }
    ));
    assert!(mismatch.source().unwrap().is::<StateError>());

    let view = series.view();
    let copied_view = view;
    assert!(view.spec().shares_layout(&spec));
    assert_eq!(view.len(), 2);
    assert!(!view.is_empty());
    assert_eq!(view.get(0).unwrap().time().index(), 0);
    assert!(view.get(5).is_none());
    assert_eq!(view.first().unwrap().time().index(), 0);
    assert_eq!(view.last().unwrap().time().index(), 4);
    assert_eq!(view.states().len(), 2);
    assert_eq!(view.iter().count(), 2);
    assert_eq!((&series).into_iter().count(), 2);
    assert_eq!(copied_view.into_iter().count(), 2);
    assert!(format!("{view:?}").contains("SeriesRef"));
    assert!(format!("{series:?}").contains("StateSeries"));

    let foreign_spec = StateSpec::load(fixture_path()).unwrap();
    assert!(!foreign_spec.shares_layout(&spec));
    let foreign = sample_state(&foreign_spec, 8, vec![19], &clones);
    let layout_rejection = series.push(foreign).expect_err("foreign layout must fail");
    assert!(matches!(
        layout_rejection.error(),
        SeriesError::SpecMismatch { index: 8 }
    ));
    assert_eq!(layout_rejection.state().time().index(), 8);
    assert!(format!("{layout_rejection:?}").contains("PushError"));
    assert!(layout_rejection.to_string().contains("does not share"));
    assert!(layout_rejection.source().is_some());
    let (layout_error, recovered_foreign) = layout_rejection.into_parts();
    assert!(matches!(
        layout_error,
        SeriesError::SpecMismatch { index: 8 }
    ));
    assert_eq!(recovered_foreign.time().index(), 8);

    let duplicate = sample_state(&spec, 4, vec![23], &clones);
    let ordering_rejection = series
        .push(duplicate)
        .expect_err("duplicate simulation index must fail");
    assert!(matches!(
        ordering_rejection.error(),
        SeriesError::NonIncreasingTime {
            previous: 4,
            next: 4
        }
    ));
    let (_, recovered_duplicate) = ordering_rejection.into_parts();
    assert_eq!(recovered_duplicate.time().index(), 4);

    let popped = series.pop().expect("last state must move out");
    assert_eq!(popped.time().index(), 4);
    assert_eq!(series.len(), 1);
    series.push(popped).unwrap();
    assert_eq!(clones.load(Ordering::Relaxed), 0);

    let cloned = series.clone();
    assert_eq!(clones.load(Ordering::Relaxed), 2);
    assert_eq!(cloned.len(), 2);
    let owned_from_clone = cloned.into_iter().collect::<Vec<_>>();
    assert_eq!(owned_from_clone.len(), 2);

    let mut reusable = StateSeries::with_capacity(spec.clone(), 2);
    reusable
        .push(sample_state(&spec, 1, vec![29], &clones))
        .unwrap();
    reusable.clear();
    assert!(reusable.is_empty());
    assert!(reusable.capacity() >= 2);

    let state_vector_pointer = series.states().as_ptr();
    let states = series.into_states();
    assert_eq!(states.as_ptr(), state_vector_pointer);
    assert_eq!(states.len(), 2);

    println!(
        "[series] states={} capacity={} indices={:?}",
        states.len(),
        reserved_capacity,
        states
            .iter()
            .map(|state| state.time().index())
            .collect::<Vec<_>>()
    );
    println!("[invariants] layout_rejected=true ordering_rejected=true");
    println!("[ownership] push_pop_pointer_preserved=true rejected_state_recovered=true");
    println!(
        "[clone] payload_clone_calls={} independent=true",
        clones.load(Ordering::Relaxed)
    );
    println!("[result] analysis_workflow=passed");
}
