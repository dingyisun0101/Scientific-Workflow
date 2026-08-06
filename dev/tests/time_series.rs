//! Unified public integration target for the in-memory time-series module.
//!
//! This target includes the focused filename-mirroring suites and adds one
//! downstream workflow spanning the public SystemState and StateSeries APIs.
//! It deliberately performs no serialization, decoding, chunking, queueing, or
//! file IO; those behaviors belong to the future storage integration target.

use std::path::PathBuf;

use serde::Serialize;

use scientific_workflow::system_state::{StateSpec, TimePoint};
use scientific_workflow::time_series::StateSeries;

#[path = "time_series/error.rs"]
mod error_tests;
#[path = "time_series/series.rs"]
mod series_tests;

/// Serializable application payload used by the public cross-module workflow.
#[derive(Clone, Debug, PartialEq, Serialize)]
struct Sample {
    values: Vec<u64>,
}

/// Resolves the checked-in template independently of the process directory.
fn fixture_path() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/state.json")
}

#[test]
fn public_state_series_workflow_moves_and_mutates_owned_payloads() {
    let spec = StateSpec::load(fixture_path()).expect("the real state fixture must load");
    let mut initial = spec.empty(TimePoint::new(0));
    let sample = Sample {
        values: vec![2, 3, 5, 7],
    };
    let original_buffer = sample.values.as_ptr();
    drop(
        initial
            .set("population", sample)
            .expect("the declared empty field must accept the payload"),
    );

    let mut later = spec.empty(TimePoint::new(4));
    drop(
        later
            .set(
                "population",
                Sample {
                    values: vec![11, 13],
                },
            )
            .expect("the later state must accept its payload"),
    );

    let mut series = StateSeries::with_capacity(spec.clone(), 2);
    series
        .push(initial)
        .expect("the initial canonical-layout state must append");
    series
        .push(later)
        .expect("the later canonical-layout state must append");

    assert_eq!(
        series
            .first()
            .expect("the initial state must exist")
            .get::<Sample>("population")
            .expect("the initial payload must remain typed")
            .values
            .as_ptr(),
        original_buffer
    );

    series
        .field_mut::<Sample>(0, "population")
        .expect("analysis may mutate one payload without accessing state time")
        .values
        .push(17);

    let view = series.view();
    assert!(view.spec().shares_layout(&spec));
    assert_eq!(
        view.iter()
            .map(|state| state.time().index())
            .collect::<Vec<_>>(),
        vec![0, 4]
    );
    assert_eq!(
        view.first()
            .expect("the first state must remain present")
            .get::<Sample>("population")
            .expect("the mutated payload must remain typed")
            .values,
        vec![2, 3, 5, 7, 17]
    );
    let mutated_buffer = view
        .first()
        .expect("the first state must remain present")
        .get::<Sample>("population")
        .expect("the mutated payload must remain typed")
        .values
        .as_ptr();

    let vector_allocation = series.states().as_ptr();
    let states = series.into_states();
    assert_eq!(states.as_ptr(), vector_allocation);
    assert_eq!(states.len(), 2);
    assert_eq!(
        states[0]
            .get::<Sample>("population")
            .expect("ownership extraction must retain the first payload")
            .values
            .as_ptr(),
        mutated_buffer
    );
}
