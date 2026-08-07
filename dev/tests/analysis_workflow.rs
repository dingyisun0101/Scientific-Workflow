//! Logged integration workflow for in-memory state-series analysis.

use std::error::Error as _;
use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};

use scientific_workflow::system_state::{SimulationTime, StateError, SystemStateSchema};
use scientific_workflow::time_series::{StateSeries, StateSeriesError};
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
    spec: &SystemStateSchema,
    index: u64,
    values: Vec<u64>,
    clones: &Arc<AtomicUsize>,
) -> scientific_workflow::system_state::SystemState {
    let mut state = spec.create_empty_state(SimulationTime::from_step(index));
    assert!(
        state
            .insert_payload(
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
    let spec = SystemStateSchema::load_json_template(fixture_path())
        .expect("real state fixture must load");
    let clones = Arc::new(AtomicUsize::new(0));

    let empty = StateSeries::new(spec.clone());
    assert!(empty.is_empty());
    assert_eq!(empty.len(), 0);
    assert_eq!(empty.capacity(), 0);
    assert!(std::ptr::eq(
        empty.schema().field_schemas(),
        spec.field_schemas()
    ));

    let initial = sample_state(&spec, 0, vec![2, 3, 5, 7], &clones);
    let initial_pointer = initial
        .payload::<Sample>("population")
        .unwrap()
        .values
        .as_ptr();
    let later = sample_state(&spec, 4, vec![11, 13], &clones);

    let mut series = StateSeries::with_capacity(spec.clone(), 2);
    assert!(series.capacity() >= 2);
    series.reserve(2);
    let reserved_capacity = series.capacity();
    series.push_state(initial).unwrap();
    series.push_state(later).unwrap();

    assert_eq!(series.len(), 2);
    assert!(!series.is_empty());
    assert_eq!(series.state_at(0).unwrap().simulation_time().step(), 0);
    assert!(series.state_at(2).is_none());
    assert_eq!(series.first_state().unwrap().simulation_time().step(), 0);
    assert_eq!(series.last_state().unwrap().simulation_time().step(), 4);
    assert_eq!(series.as_state_slice().len(), 2);
    assert_eq!(series.iter().count(), 2);
    assert_eq!(
        series
            .first_state()
            .unwrap()
            .payload::<Sample>("population")
            .unwrap()
            .values
            .as_ptr(),
        initial_pointer
    );
    assert_eq!(clones.load(Ordering::Relaxed), 0);

    series
        .payload_mut_at::<Sample>(0, "population")
        .unwrap()
        .values
        .push(17);
    assert_eq!(
        series
            .first_state()
            .unwrap()
            .payload::<Sample>("population")
            .unwrap()
            .values,
        vec![2, 3, 5, 7, 17]
    );

    let bounds = series
        .payload_mut_at::<Sample>(9, "population")
        .expect_err("missing collection position must fail");
    assert!(matches!(
        bounds,
        StateSeriesError::PositionOutOfBounds {
            position: 9,
            len: 2
        }
    ));
    let mismatch = series
        .payload_mut_at::<String>(0, "population")
        .expect_err("wrong concrete payload type must fail");
    assert!(matches!(
        &mismatch,
        StateSeriesError::PayloadAccess {
            position: 0,
            source: StateError::TypeMismatch { .. }
        }
    ));
    assert!(mismatch.source().unwrap().is::<StateError>());

    let view = series.as_view();
    let copied_view = view;
    assert!(std::ptr::eq(
        view.schema().field_schemas(),
        spec.field_schemas()
    ));
    assert_eq!(view.len(), 2);
    assert!(!view.is_empty());
    assert_eq!(view.state_at(0).unwrap().simulation_time().step(), 0);
    assert!(view.state_at(5).is_none());
    assert_eq!(view.first_state().unwrap().simulation_time().step(), 0);
    assert_eq!(view.last_state().unwrap().simulation_time().step(), 4);
    assert_eq!(view.as_state_slice().len(), 2);
    assert_eq!(view.iter().count(), 2);
    assert_eq!((&series).into_iter().count(), 2);
    assert_eq!(copied_view.into_iter().count(), 2);
    assert!(format!("{view:?}").contains("StateSeriesView"));
    assert!(format!("{series:?}").contains("StateSeries"));

    let foreign_spec = SystemStateSchema::load_json_template(fixture_path()).unwrap();
    assert!(!std::ptr::eq(
        foreign_spec.field_schemas(),
        spec.field_schemas()
    ));
    let foreign = sample_state(&foreign_spec, 8, vec![19], &clones);
    let layout_rejection = series
        .push_state(foreign)
        .expect_err("foreign layout must fail");
    assert!(matches!(
        layout_rejection.error(),
        StateSeriesError::SchemaMismatch { index: 8 }
    ));
    assert_eq!(layout_rejection.state().simulation_time().step(), 8);
    assert!(format!("{layout_rejection:?}").contains("StateSeriesPushError"));
    assert!(layout_rejection.to_string().contains("does not share"));
    assert!(layout_rejection.source().is_some());
    let (layout_error, recovered_foreign) = layout_rejection.into_parts();
    assert!(matches!(
        layout_error,
        StateSeriesError::SchemaMismatch { index: 8 }
    ));
    assert_eq!(recovered_foreign.simulation_time().step(), 8);

    let duplicate = sample_state(&spec, 4, vec![23], &clones);
    let ordering_rejection = series
        .push_state(duplicate)
        .expect_err("duplicate simulation index must fail");
    assert!(matches!(
        ordering_rejection.error(),
        StateSeriesError::NonIncreasingTime {
            previous: 4,
            next: 4
        }
    ));
    let (_, recovered_duplicate) = ordering_rejection.into_parts();
    assert_eq!(recovered_duplicate.simulation_time().step(), 4);

    let popped = series.pop_state().expect("last state must move out");
    assert_eq!(popped.simulation_time().step(), 4);
    assert_eq!(series.len(), 1);
    series.push_state(popped).unwrap();
    assert_eq!(clones.load(Ordering::Relaxed), 0);

    let cloned = series.clone();
    assert_eq!(clones.load(Ordering::Relaxed), 2);
    assert_eq!(cloned.len(), 2);
    let owned_from_clone = cloned.into_iter().collect::<Vec<_>>();
    assert_eq!(owned_from_clone.len(), 2);

    let mut reusable = StateSeries::with_capacity(spec.clone(), 2);
    reusable
        .push_state(sample_state(&spec, 1, vec![29], &clones))
        .unwrap();
    reusable.clear_states();
    assert!(reusable.is_empty());
    assert!(reusable.capacity() >= 2);

    let state_vector_pointer = series.as_state_slice().as_ptr();
    let states = series.into_states();
    assert_eq!(states.as_ptr(), state_vector_pointer);
    assert_eq!(states.len(), 2);

    println!(
        "[series] states={} capacity={} indices={:?}",
        states.len(),
        reserved_capacity,
        states
            .iter()
            .map(|state| state.simulation_time().step())
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
