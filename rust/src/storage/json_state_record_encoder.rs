//! Borrowed JSON encoding for one logical output stream.
//!
//! [`JsonStateRecordEncoder`] is the synchronous boundary between a simulation-owned
//! [`SystemState`] and the asynchronous storage writer. It selects an ordered
//! subset of state fields, borrows their existing payloads through erased
//! Serde references, and returns one owned [`EncodedStateRecord`]. It never clones,
//! removes, replaces, or otherwise mutates a scientific payload.
//!
//! # Construction
//!
//! Field selection is validated once against the program's [`SystemStateSchema`]. The
//! caller may list keys in any order; the encoder stores them in canonical
//! template order so equivalent selections always produce the same JSON key
//! order and metadata schema. Empty selections are valid and produce
//! time-bearing records with an empty `values` object.
//!
//! # Sampling
//!
//! [`JsonStateRecordEncoder::encode`] first verifies that every selected slot is populated.
//! Each successful lookup is retained as a borrowed erased-Serde reference, so
//! the subsequent serialization pass does not look up the field again. The
//! local reference vector introduces no payload clone and cannot outlive the
//! call. On success, only the independently owned encoded bytes remain,
//! allowing the simulation to resume mutation immediately or move the record
//! into the recording's
//! [`StateWriterWorker`](super::queued_state_writer::StateWriterWorker).
//!
//! # Responsibility boundary
//!
//! This module performs no filesystem access, threading, queue admission,
//! chunking, checksumming, decoding, or payload-specific conversion. Concrete
//! payload types remain responsible only for their ordinary [`Serialize`]
//! implementation.

use std::cell::Cell;
use std::collections::HashSet;

use serde::ser::SerializeMap;
use serde::{Serialize, Serializer};

use crate::system_state::{SystemState, SystemStateSchema};

use super::error::StorageError;
use super::jsonl_format::EncodedStateRecord;

/// Reusable borrowed-state encoder for one logical output stream.
///
/// The encoder owns only a stream name and selected field names. The
/// [`SystemStateSchema`] supplied to construction is borrowed for validation and then
/// released; the recording writer remains the sole owner of its shared layout
/// handle. The encoder owns no payload, output buffer, mutable scratch space,
/// queue, or file handle. Consequently, separate sampling calls allocate only
/// their resulting encoded buffer.
#[derive(Clone, Debug)]
pub(crate) struct JsonStateRecordEncoder {
    stream: Box<str>,
    fields: Box<[Box<str>]>,
}

impl JsonStateRecordEncoder {
    /// Validates and canonicalizes one stream's selected state fields.
    ///
    /// `fields` may arrive in arbitrary order. Each exact field name must occur
    /// once in `spec`; successful construction stores names in template order.
    /// Normal builds retain no [`SystemStateSchema`] handle after validation.
    /// Scientific payloads do not exist at this stage and cannot be copied.
    ///
    /// # Errors
    ///
    /// Returns [`StorageError::InvalidConfiguration`] when the normalized stream name
    /// is empty, a selected name is empty or unknown, or a field is selected
    /// more than once.
    pub(crate) fn new<I, K>(
        stream: impl Into<String>,
        spec: &SystemStateSchema,
        fields: I,
    ) -> Result<Self, StorageError>
    where
        I: IntoIterator<Item = K>,
        K: AsRef<str>,
    {
        let stream = stream.into();
        let stream = stream.trim();
        if stream.is_empty() {
            return Err(StorageError::InvalidConfiguration {
                setting: "stream",
                reason: "stream name must not be empty".to_owned(),
            });
        }

        let mut selected = HashSet::new();
        for field in fields {
            let name = field.as_ref();
            if name.is_empty() {
                return Err(StorageError::InvalidConfiguration {
                    setting: "fields",
                    reason: format!("stream `{stream}` contains an empty field name"),
                });
            }
            let declaration =
                spec.field_schema(name)
                    .ok_or_else(|| StorageError::InvalidConfiguration {
                        setting: "fields",
                        reason: format!("stream `{stream}` selects unknown state field `{name}`"),
                    })?;
            if !selected.insert(declaration.position()) {
                return Err(StorageError::InvalidConfiguration {
                    setting: "fields",
                    reason: format!("stream `{stream}` selects field `{name}` more than once"),
                });
            }
        }

        let fields = spec
            .field_schemas()
            .iter()
            .filter(|field| selected.contains(&field.position()))
            .map(|field| Box::<str>::from(field.name()))
            .collect::<Vec<_>>()
            .into_boxed_slice();

        Ok(Self {
            stream: stream.into(),
            fields,
        })
    }

    /// Iterates selected field names in canonical template order.
    pub(crate) fn fields(&self) -> impl ExactSizeIterator<Item = &str> {
        self.fields.iter().map(AsRef::as_ref)
    }

    /// Encodes one borrowed state as a compact, complete JSONL record.
    ///
    /// The state need not share the `SystemStateSchema` allocation used during encoder
    /// construction. Each selected name is checked against the supplied state,
    /// allowing an independently reconstructed but compatible partial state to
    /// be encoded safely. Extra state fields are ignored.
    ///
    /// The produced buffer has this shape before [`EncodedStateRecord`] adds its
    /// framing newline:
    ///
    /// ```json
    /// {"iteration":12,"physical_time":0.25,"values":{"population":[1,2,3]}}
    /// ```
    ///
    /// `physical_time` is omitted when absent. `values` keys follow canonical
    /// template order. The payload objects are borrowed for serialization and
    /// every borrow ends before this method returns.
    ///
    /// # Errors
    ///
    /// Returns [`StorageError::StateAccess`] for an unknown or empty selected
    /// slot in the supplied state. Returns [`StorageError::EncodeField`] when a
    /// selected payload's own serializer rejects its value. A failed call
    /// drops its incomplete byte buffer and produces no [`EncodedStateRecord`].
    pub(crate) fn encode(&self, state: &SystemState) -> Result<EncodedStateRecord, StorageError> {
        let time = state.simulation_time();

        // Resolving before Serde preserves StateError as a typed source. The
        // successful borrows are retained for the serialization pass so each
        // selected field incurs exactly one state lookup.
        let payloads = self
            .fields
            .iter()
            .map(|field| {
                state
                    .serializable(field)
                    .map_err(|source| StorageError::StateAccess {
                        stream: self.stream.to_string(),
                        iteration: time.iteration(),
                        field: field.to_string(),
                        source,
                    })
            })
            .collect::<Result<Vec<_>, _>>()?;

        let active_field = Cell::new(None);
        let document = RecordRef {
            iteration: time.iteration(),
            physical_time: time.physical_time(),
            values: ValuesRef {
                fields: &self.fields,
                payloads: &payloads,
                active_field: &active_field,
            },
        };
        let json = serde_json::to_vec(&document).map_err(|source| {
            let field = active_field
                .get()
                .and_then(|index| self.fields.get(index))
                .map_or_else(|| "<record>".to_owned(), ToString::to_string);
            StorageError::EncodeField {
                stream: self.stream.to_string(),
                iteration: time.iteration(),
                field,
                source,
            }
        })?;

        Ok(EncodedStateRecord::new(time, json))
    }
}

/// Borrowed top-level JSON representation before newline framing.
#[derive(Serialize)]
struct RecordRef<'a> {
    iteration: u64,
    #[serde(skip_serializing_if = "Option::is_none")]
    physical_time: Option<f64>,
    values: ValuesRef<'a>,
}

/// Serializes pre-resolved selected values beside their canonical field keys.
struct ValuesRef<'a> {
    fields: &'a [Box<str>],
    payloads: &'a [&'a dyn erased_serde::Serialize],
    active_field: &'a Cell<Option<usize>>,
}

impl Serialize for ValuesRef<'_> {
    /// Emits string keys and erased borrowed values in canonical field order.
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        debug_assert_eq!(self.fields.len(), self.payloads.len());
        let mut map = serializer.serialize_map(Some(self.fields.len()))?;
        for (index, (field, payload)) in self.fields.iter().zip(self.payloads).enumerate() {
            self.active_field.set(Some(index));
            map.serialize_entry(field, &ErasedRef(*payload))?;
            self.active_field.set(None);
        }
        map.end()
    }
}

/// Adapts an erased-serde trait object back to Serde's generic trait method.
struct ErasedRef<'a>(&'a dyn erased_serde::Serialize);

impl Serialize for ErasedRef<'_> {
    /// Delegates directly to the concrete payload's erased Serialize vtable.
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        erased_serde::serialize(self.0, serializer)
    }
}
