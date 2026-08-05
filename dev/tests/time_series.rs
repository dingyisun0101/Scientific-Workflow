//! Unified integration test for all implemented Rust data-model modules.
//!
//! The public crate facade currently exports SystemState but intentionally
//! withholds the time-series API until its files complete individual review.
//! This Cargo-discovered test therefore re-exports the real public SystemState
//! boundary at the path expected by the production time-series modules and
//! includes those production modules directly.
//!
//! The test covers one complete in-memory persistence preparation path:
//!
//! 1. load a real JSON StateSpec fixture;
//! 2. move a serializable owned payload into a SystemState;
//! 3. register its stable type tag with a CodecRegistry;
//! 4. append the state to an ordered StateSeries;
//! 5. borrow and encode the original payload without an intermediate value;
//! 6. decode a second owned payload directly into a new state;
//! 7. append the reconstructed state and inspect both through SeriesRef;
//! 8. move the original state vector into StateChunk and back.
//!
//! There is no format, writer, or reader module yet, so JSON framing in this
//! test intentionally covers one payload value rather than claiming a durable
//! series-file round trip.

use std::path::PathBuf;

use serde::{Deserialize, Serialize};

/// Reproduces the production crate-root import used by private time-series
/// modules while retaining the real externally visible SystemState API.
mod system_state {
    pub use scientific_workflow::system_state::*;
}

/// Current production time-series modules, connected only for this test.
mod time_series {
    #[path = "../../src/time_series/error.rs"]
    #[allow(dead_code)]
    pub mod error;

    #[path = "../../src/time_series/codec.rs"]
    #[allow(dead_code)]
    pub mod codec;

    #[path = "../../src/time_series/series.rs"]
    #[allow(dead_code)]
    pub mod series;
}

use system_state::{StateSpec, TimePoint};
use time_series::codec::CodecRegistry;
use time_series::error::SeriesError;
use time_series::series::StateSeries;

/// Small application-defined payload standing in for an owned scientific
/// tensor while keeping the integration contract independent of one library.
#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
struct Sample {
    values: Vec<u64>,
}

/// Resolves the checked-in template independently of the process directory.
fn fixture_path() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/state.json")
}

#[test]
fn registered_payload_round_trip_integrates_all_current_modules() {
    let spec = StateSpec::load(fixture_path()).expect("the real state fixture must load");
    let field = spec
        .get("population")
        .expect("the fixture must declare population");

    let mut registry = CodecRegistry::new();
    registry
        .register_with_size::<Sample, _>(field.type_tag(), |sample| {
            sample.values.len() * size_of::<u64>()
        })
        .expect("the stable payload tag must register exactly once");
    assert!(registry.contains(field.type_tag()));
    assert_eq!(registry.len(), 1);

    let mut initial = spec.empty(TimePoint::new(0));
    initial
        .set(
            field.name(),
            Sample {
                values: vec![2, 3, 5, 7],
            },
        )
        .expect("the declared field must accept the concrete payload");
    let original_buffer = initial
        .get::<Sample>(field.name())
        .expect("the inserted payload must remain typed")
        .values
        .as_ptr();

    let mut series = StateSeries::with_capacity(spec.clone(), 2);
    series
        .push(initial)
        .expect("the canonical-layout state must append");
    assert_eq!(
        series
            .first()
            .expect("the initial state must exist")
            .get::<Sample>(field.name())
            .expect("the series must own the original payload")
            .values
            .as_ptr(),
        original_buffer
    );

    let initial_state = series.first().expect("the initial state must exist");
    assert_eq!(
        registry
            .estimate(initial_state, field)
            .expect("the registered estimator must inspect borrowed data"),
        Some(4 * size_of::<u64>())
    );
    let borrowed = registry
        .value(initial_state, field)
        .expect("the registered codec must expose a borrowed Serde value");
    let encoded =
        serde_json::to_string(borrowed).expect("the borrowed payload must encode as JSON");
    assert_eq!(encoded, r#"{"values":[2,3,5,7]}"#);
    assert_eq!(
        initial_state
            .get::<Sample>(field.name())
            .expect("encoding must leave the payload in place")
            .values
            .as_ptr(),
        original_buffer
    );

    let missing_registry = CodecRegistry::new();
    let missing = match missing_registry.value(initial_state, field) {
        Err(error) => error,
        Ok(_) => panic!("an unregistered stable tag must fail"),
    };
    assert!(matches!(
        missing,
        SeriesError::MissingCodec { ref type_tag } if type_tag == field.type_tag()
    ));

    let mut reconstructed = spec.empty(TimePoint::new(4));
    let mut json = serde_json::Deserializer::from_str(&encoded);
    let mut erased = <dyn erased_serde::Deserializer>::erase(&mut json);
    registry
        .decode(&mut reconstructed, field, &mut erased)
        .expect("the registered codec must construct the concrete payload");
    json.end()
        .expect("decoding must consume exactly one payload value");
    assert_eq!(
        reconstructed
            .get::<Sample>(field.name())
            .expect("the decoded payload must be moved into the state"),
        &Sample {
            values: vec![2, 3, 5, 7],
        }
    );

    series
        .push(reconstructed)
        .expect("the later reconstructed state must append");
    let view = series.view();
    assert!(view.spec().shares_layout(&spec));
    assert_eq!(
        view.iter()
            .map(|state| state.time().index())
            .collect::<Vec<_>>(),
        vec![0, 4]
    );
    assert_eq!(
        view.last()
            .expect("the reconstructed state must be last")
            .get::<Sample>(field.name())
            .expect("the reconstructed payload must remain typed")
            .values,
        vec![2, 3, 5, 7]
    );

    let state_vector = series.states().as_ptr();
    let chunk = series.into_chunk(0, 2 * 4 * size_of::<u64>());
    assert_eq!(chunk.states().as_ptr(), state_vector);
    assert_eq!(chunk.first_index(), Some(0));
    assert_eq!(chunk.last_index(), Some(4));
    assert_eq!(chunk.len(), 2);

    let restored = chunk.into_series();
    assert_eq!(restored.states().as_ptr(), state_vector);
    assert_eq!(restored.len(), 2);
    assert_eq!(
        restored
            .first()
            .expect("the original state must survive chunk movement")
            .get::<Sample>(field.name())
            .expect("the original payload must survive chunk movement")
            .values
            .as_ptr(),
        original_buffer
    );
}
