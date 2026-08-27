//! Per-payload decoding contracts and key-based decoder registration.
//!
//! A payload decoder receives only one field's borrowed raw JSON and returns
//! one concrete Rust value. It does not receive record time, sibling fields,
//! chunk metadata, a destination state, or a series. [`JsonPayloadDecoderRegistry`] matches keys
//! to these independently reusable conversions and privately adapts their
//! heterogeneous results for insertion into `SystemState`.
//!
//! [`JsonPayloadDecoder::decode_json_payload`] accepts `&str` containing exactly one JSON value.
//! The reader can obtain this slice from `serde_json::value::RawValue` while
//! retaining the enclosing line buffer. A tensor decoder can therefore build
//! its final allocation without an intermediate `serde_json::Value` tree. The
//! returned payload is owned and cannot retain the temporary input.
//!
//! Type erasure occurs only inside the registry adapter. Public decoders still
//! return their real `T`; the adapter moves that value directly into the state
//! through `SystemState::insert_payload` and never invokes `T::clone`.
//!
//! This module performs no JSON parsing, filesystem access, chunk validation,
//! record-key validation, state construction, or series collection. The reader
//! owns those operations and dispatches fields in canonical schema order.

use std::collections::{HashMap, HashSet};
use std::error::Error;
use std::fmt;
use std::marker::PhantomData;

use serde::Serialize;
use serde::de::DeserializeOwned;

use crate::state::advanced::{StateError, SystemState};

use super::error::PersistenceError;

#[path = "json_payload_decoder/string.rs"]
mod string;
#[path = "json_payload_decoder/vec_f64.rs"]
mod vec_f64;

pub use string::JsonStringDecoder;
pub use vec_f64::JsonVecF64Decoder;

/// Object-safe error boundary for application-defined payload conversion.
type BoxError = Box<dyn Error + Send + Sync + 'static>;

/// Converts one borrowed raw JSON value into one concrete state payload.
///
/// `T` stays explicit at this boundary. Any thread-safe
/// `Fn(&str) -> Result<T, E>` implements the trait automatically; named decoder
/// types may implement it directly when they own configuration or shared state.
pub trait JsonPayloadDecoder<T>: Send + Sync + 'static {
    /// Decoder-specific failure retained by [`PersistenceError::DecodeField`].
    type Error: Error + Send + Sync + 'static;

    /// Decodes exactly one complete raw JSON value into an owned payload.
    fn decode_json_payload(&self, raw_json: &str) -> Result<T, Self::Error>;
}

impl<T, E, F> JsonPayloadDecoder<T> for F
where
    F: Fn(&str) -> Result<T, E> + Send + Sync + 'static,
    E: Error + Send + Sync + 'static,
{
    type Error = E;

    /// Invokes the registered closure without wrapping or copying its output.
    fn decode_json_payload(&self, raw_json: &str) -> Result<T, Self::Error> {
        self(raw_json)
    }
}

/// Heterogeneous per-key payload decoder registry.
///
/// One registry may contain the union of keys used by several output streams.
/// Coverage therefore requires every selected key but permits additional
/// registrations. Keys are exact and are not trimmed or case-normalized.
///
/// This type is intentionally non-Clone because registered decoders may own
/// caches, handles, or synchronization state with unknown clone semantics.
#[derive(Default)]
pub struct JsonPayloadDecoderRegistry {
    entries: HashMap<Box<str>, Box<dyn ErasedPayloadDecoder>>,
}

impl JsonPayloadDecoderRegistry {
    /// Creates an empty registry.
    pub fn new() -> Self {
        Self::default()
    }

    /// Creates an empty registry with capacity for at least `capacity` keys.
    pub fn with_capacity(capacity: usize) -> Self {
        Self {
            entries: HashMap::with_capacity(capacity),
        }
    }

    /// Adds a field decoded directly through Serde JSON into `T`.
    ///
    /// This is the concise path for payload types whose JSON representation
    /// already matches their Rust representation. Specialized decoders remain
    /// available through [`JsonPayloadDecoderRegistry::register_for_field`]
    /// when conversion requires configuration, validation, or a different
    /// wire shape.
    ///
    /// # Errors
    ///
    /// Returns the same empty-key or duplicate-key errors as
    /// [`JsonPayloadDecoderRegistry::register_for_field`]. Payload parse
    /// failures are reported later with their field context by the reader.
    pub fn with_json_field<T>(mut self, key: impl Into<String>) -> Result<Self, PersistenceError>
    where
        T: DeserializeOwned + Serialize + Clone + Send + 'static,
    {
        self.register_for_field::<T, _>(key, |raw_json: &str| serde_json::from_str::<T>(raw_json))?;
        Ok(self)
    }

    /// Registers one typed decoder under one exact state field key.
    ///
    /// Successful decoded values are moved directly into `SystemState`; the
    /// adapter never invokes `T::clone`.
    ///
    /// # Errors
    ///
    /// Returns [`PersistenceError::InvalidConfiguration`] for an empty key and
    /// [`PersistenceError::DuplicateDecoder`] for an existing key. The incoming
    /// decoder is dropped on either configuration error.
    pub fn register_for_field<T, D>(
        &mut self,
        key: impl Into<String>,
        decoder: D,
    ) -> Result<(), PersistenceError>
    where
        T: Serialize + Clone + Send + 'static,
        D: JsonPayloadDecoder<T>,
    {
        let key = key.into();
        if key.is_empty() {
            return Err(PersistenceError::InvalidConfiguration {
                setting: "decoder.key",
                reason: "decoder key must not be empty".to_owned(),
            });
        }
        if self.entries.contains_key(key.as_str()) {
            return Err(PersistenceError::DuplicateDecoder { field: key });
        }
        self.entries.insert(
            key.into_boxed_str(),
            Box::new(TypedDecoder {
                decoder,
                payload: PhantomData,
            }),
        );
        Ok(())
    }

    /// Returns the number of registered field keys.
    pub fn len(&self) -> usize {
        self.entries.len()
    }

    /// Reports whether no payload decoder is registered.
    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    /// Reports whether an exact field key has a registered decoder.
    pub fn has_decoder_for_field(&self, key: &str) -> bool {
        self.entries.contains_key(key)
    }

    /// Iterates registered keys in unspecified hash-map order.
    ///
    /// Reader dispatch never uses this order. Callers needing stable display
    /// should sort the returned strings.
    pub fn registered_field_names(&self) -> impl ExactSizeIterator<Item = &str> {
        self.entries.keys().map(AsRef::as_ref)
    }

    /// Verifies that every selected stream field has a decoder.
    ///
    /// Additional keys remain valid for other streams. Repeated input keys are
    /// harmless and checked once, though metadata validation rejects them
    /// before the reader reaches this boundary.
    pub(crate) fn require<'a>(
        &self,
        fields: impl IntoIterator<Item = &'a str>,
    ) -> Result<(), PersistenceError> {
        let mut checked = HashSet::new();
        for field in fields {
            if checked.insert(field) && !self.has_decoder_for_field(field) {
                return Err(PersistenceError::MissingDecoder {
                    field: field.to_owned(),
                });
            }
        }
        Ok(())
    }

    /// Decodes one matched field and moves it into an empty destination slot.
    ///
    /// The reader supplies context after validating record keys. A conversion
    /// or unexpected insertion failure becomes [`PersistenceError::DecodeField`]
    /// with its original source chain preserved.
    pub(crate) fn decode_into(
        &self,
        stream: &str,
        iteration: u64,
        field: &str,
        raw_json: &str,
        state: &mut SystemState,
    ) -> Result<(), PersistenceError> {
        let decoder = self
            .entries
            .get(field)
            .ok_or_else(|| PersistenceError::MissingDecoder {
                field: field.to_owned(),
            })?;
        decoder
            .decode_into(raw_json, field, state)
            .map_err(|source| PersistenceError::DecodeField {
                stream: stream.to_owned(),
                iteration,
                field: field.to_owned(),
                source,
            })
    }
}

impl fmt::Debug for JsonPayloadDecoderRegistry {
    /// Formats sorted keys without exposing decoder internals or payloads.
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        let mut keys = self.registered_field_names().collect::<Vec<_>>();
        keys.sort_unstable();
        formatter
            .debug_struct("JsonPayloadDecoderRegistry")
            .field("keys", &keys)
            .finish_non_exhaustive()
    }
}

/// Object-safe insertion adapter stored by [`JsonPayloadDecoderRegistry`].
trait ErasedPayloadDecoder: Send + Sync {
    /// Decodes one field and moves its concrete result into `state`.
    fn decode_into(
        &self,
        raw_json: &str,
        field: &str,
        state: &mut SystemState,
    ) -> Result<(), BoxError>;
}

/// Concrete typed decoder hidden behind [`ErasedPayloadDecoder`].
struct TypedDecoder<D, T> {
    decoder: D,
    /// Associates the adapter with its concrete output without owning a `T`.
    payload: PhantomData<fn() -> T>,
}

impl<T, D> ErasedPayloadDecoder for TypedDecoder<D, T>
where
    T: Serialize + Clone + Send + 'static,
    D: JsonPayloadDecoder<T>,
{
    /// Preserves `T` through conversion, then transfers it into the state.
    fn decode_into(
        &self,
        raw_json: &str,
        field: &str,
        state: &mut SystemState,
    ) -> Result<(), BoxError> {
        let payload = self
            .decoder
            .decode_json_payload(raw_json)
            .map_err(|source| Box::new(source) as BoxError)?;

        match state.insert_payload(field, payload) {
            Ok(None) => Ok(()),
            Ok(Some(previous)) => {
                // Restore the pre-existing payload transactionally. Setting an
                // identical concrete type must succeed and returns the newly
                // decoded replacement, which is then dropped.
                let decoded = state
                    .insert_payload(field, previous)
                    .expect("restoring an identical concrete payload type must succeed");
                drop(decoded);
                Err(Box::new(DecoderInsertError::Occupied {
                    field: field.to_owned(),
                }))
            }
            Err(rejection) => {
                let (source, payload) = rejection.into_parts();
                drop(payload);
                Err(Box::new(DecoderInsertError::State(source)))
            }
        }
    }
}

/// Internal state-insertion failure after payload conversion succeeded.
#[derive(Debug)]
enum DecoderInsertError {
    /// Reader attempted to populate the same state field more than once.
    Occupied { field: String },
    /// Destination state did not declare the expected field.
    State(StateError),
}

impl fmt::Display for DecoderInsertError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Occupied { field } => {
                write!(
                    formatter,
                    "decoded state field `{field}` is already populated"
                )
            }
            Self::State(source) => source.fmt(formatter),
        }
    }
}

impl Error for DecoderInsertError {
    /// Preserves an underlying SystemState insertion failure when present.
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        match self {
            Self::Occupied { .. } => None,
            Self::State(source) => Some(source),
        }
    }
}
