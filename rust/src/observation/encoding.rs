//! Canonical borrowed state encoding and backend handoff.

use std::cell::Cell;
use std::fmt;

use serde::ser::SerializeSeq;
use serde::{Serialize, Serializer};

use crate::state::{StateObservationAccess, StateTime};

use super::error::ObservationError;
use super::state_observation::StateObservation;
use super::stream::BoundObservationStream;

/// An owned encoded scientific observation ready for a persistence backend.
pub(crate) struct EncodedObservation {
    stream: String,
    time: StateTime,
    bytes: Vec<u8>,
}

impl EncodedObservation {
    /// Consumes the observation into its queue-ready owned components.
    pub(crate) fn into_parts(self) -> (String, StateTime, Vec<u8>) {
        (self.stream, self.time, self.bytes)
    }
}

impl fmt::Debug for EncodedObservation {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("EncodedObservation")
            .field("stream", &self.stream)
            .field("time", &self.time)
            .field("bytes", &self.bytes.len())
            .finish_non_exhaustive()
    }
}

pub(super) fn encode(
    observation: StateObservation<'_>,
    stream: &BoundObservationStream,
) -> Result<EncodedObservation, ObservationError> {
    let time = observation.time();
    let payloads = stream
        .fields()
        .iter()
        .map(|field| {
            observation
                .state()
                .serializable_payload(field.name())
                .map_err(|source| ObservationError::StateAccess {
                    stream: stream.name().to_owned(),
                    iteration: time.iteration(),
                    field: field.name().to_owned(),
                    source,
                })
        })
        .collect::<Result<Vec<_>, _>>()?;

    let active_field = Cell::new(None);
    let document = RecordRef {
        iteration: time.iteration(),
        physical_time: time.physical_time(),
        values: ValuesRef {
            fields: stream.fields(),
            payloads: &payloads,
            active_field: &active_field,
        },
    };
    let bytes = serde_json::to_vec(&document).map_err(|source| {
        let field = active_field
            .get()
            .and_then(|index| stream.fields().get(index))
            .map_or_else(|| "<record>".to_owned(), |field| field.name().to_owned());
        ObservationError::EncodeField {
            stream: stream.name().to_owned(),
            iteration: time.iteration(),
            field,
            source,
        }
    })?;
    Ok(EncodedObservation {
        stream: stream.name().to_owned(),
        time,
        bytes,
    })
}

#[derive(Serialize)]
struct RecordRef<'a> {
    iteration: u64,
    #[serde(skip_serializing_if = "Option::is_none")]
    physical_time: Option<f64>,
    values: ValuesRef<'a>,
}

struct ValuesRef<'a> {
    fields: &'a [crate::state::StateFieldSchema],
    payloads: &'a [&'a dyn erased_serde::Serialize],
    active_field: &'a Cell<Option<usize>>,
}

impl Serialize for ValuesRef<'_> {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        debug_assert_eq!(self.fields.len(), self.payloads.len());
        let mut sequence = serializer.serialize_seq(Some(self.fields.len()))?;
        for (index, payload) in self.payloads.iter().enumerate() {
            self.active_field.set(Some(index));
            sequence.serialize_element(&ErasedRef(*payload))?;
            self.active_field.set(None);
        }
        sequence.end()
    }
}

struct ErasedRef<'a>(&'a dyn erased_serde::Serialize);

impl Serialize for ErasedRef<'_> {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        erased_serde::serialize(self.0, serializer)
    }
}
