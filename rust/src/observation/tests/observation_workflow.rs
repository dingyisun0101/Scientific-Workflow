//! Internal binding, encoding, and sampling contracts for Observation.

use std::path::Path;

use serde::Serialize;

use crate::state::{StateTime, SystemState, SystemStateSchema, schema_from_json_value};

use super::{
    BoundObservationPlan, ObservationError, ObservationPlan, ObservationSession, ObservationStream,
};

fn test_schema() -> SystemStateSchema {
    schema_from_json_value(
        Path::new("observation-test-state.json"),
        &serde_json::json!({
            "fields": [
                {"name": "first", "description": "First value"},
                {"name": "second", "description": "Second value"}
            ]
        }),
    )
    .expect("the internal observation test schema is valid")
}

fn populated_state(schema: &SystemStateSchema, iteration: u64) -> SystemState {
    let mut state = schema.create_empty_state(StateTime::from_iteration(iteration));
    state.initialize_payload("first", 10_u64).unwrap();
    state.initialize_payload("second", 20_u64).unwrap();
    state
}

fn encoded_parts(
    observations: Vec<super::encoding::EncodedObservation>,
) -> Vec<(String, StateTime, serde_json::Value)> {
    observations
        .into_iter()
        .map(|observation| {
            let (stream, time, bytes) = observation.into_parts();
            let document = serde_json::from_slice(&bytes).expect("observation JSON is valid");
            (stream, time, document)
        })
        .collect()
}

#[test]
fn binding_normalizes_declarations_and_encodes_in_schema_order() {
    let schema = test_schema();
    let selected = ObservationStream::fields(" selected ", [" second ", " first "]).unwrap();
    let plan = ObservationPlan::streams([selected])
        .unwrap()
        .with_iteration_unit(" step ")
        .unwrap()
        .with_physical_time_unit(" s ")
        .unwrap();
    let bound = BoundObservationPlan::bind(plan, &schema).unwrap();

    assert_eq!(bound.iteration_unit(), Some("step"));
    assert_eq!(bound.physical_time_unit(), Some("s"));
    assert_eq!(bound.streams()[0].name(), "selected");
    assert_eq!(
        bound.streams()[0]
            .fields()
            .iter()
            .map(|field| field.name())
            .collect::<Vec<_>>(),
        ["first", "second"]
    );

    let state = populated_state(&schema, 0);
    let mut session = ObservationSession::new(bound);
    let encoded = encoded_parts(session.observe(&state).unwrap());
    assert_eq!(encoded.len(), 1);
    assert_eq!(encoded[0].0, "selected");
    assert_eq!(encoded[0].1, StateTime::from_iteration(0));
    assert_eq!(encoded[0].2["values"], serde_json::json!([10, 20]));

    assert!(matches!(
        ObservationStream::all_fields("  "),
        Err(ObservationError::EmptyStreamName)
    ));
    assert!(matches!(
        ObservationStream::fields("state", ["first", " "]),
        Err(ObservationError::EmptyFieldName { stream }) if stream == "state"
    ));
    assert!(matches!(
        ObservationStream::fields("state", ["first", " first "]),
        Err(ObservationError::DuplicateField { stream, field })
            if stream == "state" && field == "first"
    ));
    assert!(matches!(
        ObservationPlan::all_fields().with_physical_time_unit(" "),
        Err(ObservationError::EmptyAxisUnit {
            axis: "physical_time"
        })
    ));
}

#[test]
fn sessions_apply_cadence_terminal_deduplication_and_ordering() {
    let schema = test_schema();
    let plan = ObservationPlan::streams([
        ObservationStream::fields("even", ["first"])
            .unwrap()
            .every_iterations(2)
            .unwrap(),
        ObservationStream::all_fields("thirds")
            .unwrap()
            .every_iterations(3)
            .unwrap(),
    ])
    .unwrap();
    let bound = BoundObservationPlan::bind(plan, &schema).unwrap();
    let mut session = ObservationSession::new(bound);
    let mut state = populated_state(&schema, 0);

    let initial = encoded_parts(session.observe(&state).unwrap());
    assert_eq!(
        initial
            .iter()
            .map(|(stream, _, _)| stream.as_str())
            .collect::<Vec<_>>(),
        ["even", "thirds"]
    );
    assert!(session.observe(&state).unwrap().is_empty());

    state.replace_time(StateTime::from_iteration(1));
    assert!(session.observe(&state).unwrap().is_empty());
    let terminal = encoded_parts(session.observe_final(&state).unwrap());
    assert_eq!(terminal.len(), 2);
    assert!(session.observe_final(&state).unwrap().is_empty());

    state.replace_time(StateTime::from_iteration(2));
    let due = encoded_parts(session.observe(&state).unwrap());
    assert_eq!(due.len(), 1);
    assert_eq!(due[0].0, "even");

    state.replace_time(StateTime::from_iteration(1));
    assert!(matches!(
        session.observe(&state),
        Err(ObservationError::NonIncreasingObservation {
            stream,
            previous: 2,
            next: 1
        }) if stream == "even"
    ));

    let foreign_schema = test_schema();
    let foreign = populated_state(&foreign_schema, 2);
    let bound = BoundObservationPlan::bind(ObservationPlan::all_fields(), &schema).unwrap();
    let mut foreign_session = ObservationSession::new(bound);
    assert!(matches!(
        foreign_session.observe(&foreign),
        Err(ObservationError::SchemaMismatch { iteration: 2 })
    ));
}

#[derive(Clone, Debug)]
struct ConditionalEncoding {
    reject: bool,
}

impl Serialize for ConditionalEncoding {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: serde::Serializer,
    {
        if self.reject {
            return Err(serde::ser::Error::custom("intentional observation failure"));
        }
        serializer.serialize_u64(30)
    }
}

#[test]
fn observation_failures_do_not_advance_any_stream_marker() {
    let schema = test_schema();
    let plan = ObservationPlan::streams([
        ObservationStream::fields("ready", ["first"]).unwrap(),
        ObservationStream::fields("missing", ["second"]).unwrap(),
    ])
    .unwrap();
    let bound = BoundObservationPlan::bind(plan, &schema).unwrap();
    let mut session = ObservationSession::new(bound);
    let mut state = schema.create_empty_state(StateTime::from_iteration(0));
    state.initialize_payload("first", 10_u64).unwrap();

    assert!(matches!(
        session.observe(&state),
        Err(ObservationError::StateAccess {
            stream,
            iteration: 0,
            field,
            ..
        }) if stream == "missing" && field == "second"
    ));
    state.initialize_payload("second", 20_u64).unwrap();
    assert_eq!(session.observe(&state).unwrap().len(), 2);

    let encoding_schema = test_schema();
    let bound = BoundObservationPlan::bind(
        ObservationPlan::fields(["first"]).unwrap(),
        &encoding_schema,
    )
    .unwrap();
    let mut encoding_session = ObservationSession::new(bound);
    let mut encoding_state = encoding_schema.create_empty_state(StateTime::from_iteration(0));
    encoding_state
        .initialize_payload("first", ConditionalEncoding { reject: true })
        .unwrap();

    assert!(matches!(
        encoding_session.observe(&encoding_state),
        Err(ObservationError::EncodeField {
            stream,
            iteration: 0,
            field,
            ..
        }) if stream == "state" && field == "first"
    ));
    encoding_state
        .payload_mut::<ConditionalEncoding>("first")
        .unwrap()
        .reject = false;
    assert_eq!(encoding_session.observe(&encoding_state).unwrap().len(), 1);
}
