//! Public writer-definition and advanced observation contract.

use std::convert::Infallible;
use std::path::PathBuf;

use scientific_workflow::prelude::advanced::*;

fn fixture_path() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/state.json")
}

fn populated_state(schema: &SystemStateSchema, iteration: u64) -> SystemState {
    let mut state = schema.create_empty_state(
        StateTime::from_iteration_and_physical_time(iteration, iteration as f64 * 0.25).unwrap(),
    );
    state.insert_payload("population", vec![1.0, 2.0]).unwrap();
    state.insert_payload("space", vec![3_u64, 4]).unwrap();
    state
        .insert_payload("activity", String::from("active"))
        .unwrap();
    state
}

#[test]
fn basic_definitions_bind_with_inferred_defaults_and_canonical_fields() {
    let schema = SystemStateSchema::load_json_template(&fixture_path()).unwrap();
    let default = WriterDescriptor::bind(Writer::all_fields(), &schema).unwrap();
    assert_eq!(default.streams().len(), 1);
    assert_eq!(default.streams()[0].name(), "state");
    assert_eq!(default.streams()[0].every_iterations(), 1);
    assert!(default.streams()[0].covers_complete_state());
    assert_eq!(
        default.streams()[0]
            .fields()
            .iter()
            .map(StateFieldSchema::name)
            .collect::<Vec<_>>(),
        ["population", "space", "activity"]
    );

    let selected =
        WriterDescriptor::bind(Writer::fields(["activity", "population"]).unwrap(), &schema)
            .unwrap();
    assert_eq!(
        selected.streams()[0]
            .fields()
            .iter()
            .map(StateFieldSchema::name)
            .collect::<Vec<_>>(),
        ["population", "activity"]
    );
    assert!(!selected.streams()[0].covers_complete_state());
}

#[test]
fn multi_stream_units_and_validation_are_explicit_at_the_definition_boundary() {
    let schema = SystemStateSchema::load_json_template(&fixture_path()).unwrap();
    let definition = Writer::streams([
        Stream::fields("signal", ["activity"]).unwrap(),
        Stream::all_fields("checkpoint")
            .unwrap()
            .every_iterations(10)
            .unwrap(),
    ])
    .unwrap()
    .with_iteration_unit("step")
    .unwrap()
    .with_physical_time_unit("s")
    .unwrap();
    let descriptor = WriterDescriptor::bind(definition, &schema).unwrap();
    assert_eq!(descriptor.iteration_unit(), Some("step"));
    assert_eq!(descriptor.physical_time_unit(), Some("s"));
    assert_eq!(descriptor.streams()[1].every_iterations(), 10);
    assert!(descriptor.streams()[1].covers_complete_state());

    assert!(matches!(Writer::streams([]), Err(WriterError::EmptyWriter)));
    assert!(matches!(
        Writer::streams([
            Stream::all_fields("same").unwrap(),
            Stream::all_fields(" same ").unwrap(),
        ]),
        Err(WriterError::DuplicateStreamName { .. })
    ));
    assert!(matches!(
        Stream::fields("signal", std::iter::empty::<String>()),
        Err(WriterError::EmptyFieldSelection { .. })
    ));
    assert!(matches!(
        Stream::fields("signal", ["activity", "activity"]),
        Err(WriterError::DuplicateField { .. })
    ));
    assert!(matches!(
        Stream::all_fields("signal").unwrap().every_iterations(0),
        Err(WriterError::InvalidSamplingInterval { .. })
    ));
    assert!(matches!(
        WriterDescriptor::bind(Writer::fields(["absent"]).unwrap(), &schema),
        Err(WriterError::UnknownField { .. })
    ));
    assert!(matches!(
        Writer::all_fields().with_iteration_unit("  "),
        Err(WriterError::EmptyAxisUnit { axis: "iteration" })
    ));
}

#[derive(Default)]
struct MemorySink {
    observations: Vec<EncodedObservation>,
    outcome: Option<SessionOutcome>,
}

impl ObservationSink for MemorySink {
    type Error = Infallible;

    fn submit(&mut self, observation: EncodedObservation) -> Result<(), Self::Error> {
        self.observations.push(observation);
        Ok(())
    }

    fn finish(&mut self, outcome: SessionOutcome) -> Result<(), Self::Error> {
        self.outcome = Some(outcome);
        Ok(())
    }
}

#[test]
fn advanced_observations_encode_owned_backend_handoffs_without_payload_clones() {
    let schema = SystemStateSchema::load_json_template(&fixture_path()).unwrap();
    let state = populated_state(&schema, 4);
    let descriptor = WriterDescriptor::bind(
        Writer::streams([Stream::fields("signal", ["activity", "population"]).unwrap()]).unwrap(),
        &schema,
    )
    .unwrap();
    let observation = Observation::new(&descriptor, &state).unwrap();
    assert_eq!(observation.time(), state.time());
    assert_eq!(observation.writer().streams()[0].name(), "signal");

    let encoded = observation.encode_stream(&descriptor.streams()[0]).unwrap();
    assert_eq!(encoded.stream(), "signal");
    assert_eq!(encoded.time(), state.time());
    assert_eq!(
        serde_json::from_slice::<serde_json::Value>(encoded.bytes()).unwrap(),
        serde_json::json!({
            "iteration": 4,
            "physical_time": 1.0,
            "values": [[1.0, 2.0], "active"]
        })
    );

    let mut sink = MemorySink::default();
    sink.submit(encoded).unwrap();
    sink.finish(SessionOutcome::Complete).unwrap();
    assert_eq!(sink.observations.len(), 1);
    assert_eq!(sink.outcome, Some(SessionOutcome::Complete));

    let independent_schema = SystemStateSchema::load_json_template(&fixture_path()).unwrap();
    let independent_state = populated_state(&independent_schema, 4);
    assert!(matches!(
        Observation::new(&descriptor, &independent_state),
        Err(WriterError::SchemaMismatch { iteration: 4 })
    ));
}

#[test]
fn advanced_scopes_are_strict_basic_supersets() {
    fn accepts_writer(_: Writer) {}
    fn accepts_stream(_: Stream) {}
    fn accepts_error(_: Option<WriterError>) {}
    fn accepts_descriptor(_: Option<WriterDescriptor>) {}

    accepts_writer(Writer::all_fields());
    accepts_stream(Stream::all_fields("state").unwrap());
    accepts_error(None);
    accepts_descriptor(None);
}
