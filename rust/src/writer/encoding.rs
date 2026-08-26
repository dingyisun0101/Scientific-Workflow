//! Canonical borrowed state encoding and backend handoff.

use std::cell::Cell;
use std::error::Error;
use std::fmt;

use serde::ser::SerializeSeq;
use serde::{Serialize, Serializer};

use crate::state::advanced::StateTime;

use super::error::WriterError;
use super::observation::Observation;
use super::stream::StreamDescriptor;

/// An owned encoded scientific observation ready for a persistence backend.
pub struct EncodedObservation {
    stream: Box<str>,
    time: StateTime,
    bytes: Vec<u8>,
}

impl EncodedObservation {
    /// Returns the logical scientific stream name.
    pub fn stream(&self) -> &str {
        &self.stream
    }

    /// Returns the encoded state's scientific coordinate.
    pub fn time(&self) -> StateTime {
        self.time
    }

    /// Borrows the complete unframed canonical payload bytes.
    pub fn bytes(&self) -> &[u8] {
        &self.bytes
    }

    /// Consumes the observation and returns its encoded allocation.
    pub fn into_bytes(self) -> Vec<u8> {
        self.bytes
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
    observation: Observation<'_>,
    stream: &StreamDescriptor,
) -> Result<EncodedObservation, WriterError> {
    let time = observation.time();
    let payloads = stream
        .fields()
        .iter()
        .map(|field| {
            observation
                .state()
                .serializable(field.name())
                .map_err(|source| WriterError::StateAccess {
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
        WriterError::EncodeField {
            stream: stream.name().to_owned(),
            iteration: time.iteration(),
            field,
            source,
        }
    })?;
    Ok(EncodedObservation {
        stream: stream.name().into(),
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
    fields: &'a [crate::state::advanced::StateFieldSchema],
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

/// Runtime-owned terminal status supplied exactly once to a backend sink.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum SessionOutcome {
    /// Task work completed successfully.
    Complete,
    /// Task work failed.
    Failed {
        /// Stable user-facing failure reason.
        reason: String,
    },
    /// Task work was cancelled.
    Cancelled {
        /// Optional stable cancellation reason.
        reason: Option<String>,
    },
}

/// Persistence port receiving owned encoded observations from writer sessions.
pub trait ObservationSink: Send {
    /// Backend-specific failure composed by the runtime boundary.
    type Error: Error + Send + Sync + 'static;

    /// Accepts one complete owned observation, applying backpressure if needed.
    fn submit(&mut self, observation: EncodedObservation) -> Result<(), Self::Error>;

    /// Commits exactly one runtime-owned terminal outcome.
    fn finish(&mut self, outcome: SessionOutcome) -> Result<(), Self::Error>;
}
