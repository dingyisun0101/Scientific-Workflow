//! Public observation declaration contract.

use scientific_workflow::prelude::advanced::*;

#[test]
fn basic_declarations_infer_the_common_case() {
    let _all_fields = ObservationPlan::all_fields();
    let _selected = ObservationPlan::fields(["position", "velocity"]).unwrap();
    let _multi_stream = ObservationPlan::streams([
        ObservationStream::fields("signal", ["position"]).unwrap(),
        ObservationStream::all_fields("checkpoint")
            .unwrap()
            .every_iterations(10)
            .unwrap(),
    ])
    .unwrap()
    .with_iteration_unit("step")
    .unwrap()
    .with_physical_time_unit("s")
    .unwrap();
}

#[test]
fn declaration_errors_are_reported_before_study_binding() {
    assert!(matches!(
        ObservationPlan::streams([]),
        Err(ObservationError::EmptyPlan)
    ));
    assert!(matches!(
        ObservationPlan::streams([
            ObservationStream::all_fields("same").unwrap(),
            ObservationStream::all_fields(" same ").unwrap(),
        ]),
        Err(ObservationError::DuplicateStreamName { .. })
    ));
    assert!(matches!(
        ObservationStream::fields("signal", std::iter::empty::<String>()),
        Err(ObservationError::EmptyFieldSelection { .. })
    ));
    assert!(matches!(
        ObservationStream::all_fields("signal")
            .unwrap()
            .every_iterations(0),
        Err(ObservationError::InvalidSamplingInterval { .. })
    ));
    assert!(matches!(
        ObservationPlan::all_fields().with_iteration_unit("  "),
        Err(ObservationError::EmptyAxisUnit { axis: "iteration" })
    ));
}

#[test]
fn advanced_is_a_strict_scope_superset_without_exposing_binding_plumbing() {
    fn accepts_plan(_: ObservationPlan) {}
    fn accepts_stream(_: ObservationStream) {}
    fn accepts_error(_: Option<ObservationError>) {}

    accepts_plan(ObservationPlan::all_fields());
    accepts_stream(ObservationStream::all_fields("state").unwrap());
    accepts_error(None);
}
