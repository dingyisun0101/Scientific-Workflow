//! Runtime mapping from stable state-template tags to concrete Serde payloads.
//!
//! A [`SystemState`](crate::system_state::SystemState) stores heterogeneous Rust
//! values behind typed accessors, while its [`StateSpec`](crate::system_state::StateSpec)
//! records stable serialization tags that remain meaningful across processes.
//! [`CodecRegistry`] connects those two identities for time-series IO.
//!
//! # Registration model
//!
//! Each stable tag is registered once with one concrete Rust type. The type
//! must support Serde serialization and owned deserialization, explicit state
//! cloning, thread transfer, and runtime type identification:
//!
//! ```text
//! Serialize + DeserializeOwned + Clone + Send + 'static
//! ```
//!
//! This is an open registration model rather than a closed payload enum. User
//! structs, standard collections, scalar values, and scientific tensor types
//! all use the same registry API.
//!
//! # Streaming and ownership
//!
//! Encoding returns a borrowed object-safe Serde view of the original payload.
//! It does not clone the concrete value or construct an intermediate
//! [`serde_json::Value`]. Decoding constructs one owned concrete value and
//! moves it directly into the destination state.
//!
//! The no-copy statement applies to this registry's ownership boundaries. A
//! concrete type's own [`serde::Serialize`] implementation remains responsible
//! for avoiding internal full-buffer copies.
//!
//! # Size estimation
//!
//! JSON byte length is unknown until encoding. A registration may therefore
//! provide a cheap memory-size estimator for automatic chunking. Estimates are
//! optional and must inspect only borrowed data. State-count limits remain the
//! exact fallback when a payload type has no useful estimator.

use std::any::type_name;
use std::collections::HashMap;
use std::fmt;

use serde::Serialize;
use serde::de::DeserializeOwned;

use crate::system_state::{FieldSpec, StateError, SystemState};

use super::error::SeriesError;

/// Associates stable field type tags with concrete, round-trippable Rust
/// payload types.
///
/// Registrations are immutable once inserted: attempting to reuse a tag is an
/// error, even for the same concrete type. This prevents initialization order
/// from silently changing persisted meaning. Lookup is constant-time on
/// average; deterministic serialized field order comes from `StateSpec`, not
/// this hash map's iteration order.
///
/// The registry is `Send + Sync` when its estimator callbacks are, so a caller
/// may share it with a future background writer through an ordinary `Arc`.
#[derive(Default)]
pub struct CodecRegistry {
    codecs: HashMap<Box<str>, Box<dyn ErasedCodec>>,
}

impl CodecRegistry {
    /// Creates an empty payload-codec registry.
    ///
    /// No tensor or application-specific type is registered implicitly. This
    /// keeps the core crate independent of any one scientific data library and
    /// makes every stable-tag-to-Rust-type mapping explicit.
    pub fn new() -> Self {
        Self::default()
    }

    /// Registers a Serde-compatible concrete type for one stable template tag.
    ///
    /// This form does not provide a byte estimator. Values of `T` remain fully
    /// readable and writable, while an automatic chunk policy must rely on its
    /// exact state-count threshold or estimates from other fields.
    ///
    /// `type_tag` is stored exactly as supplied. Callers should use the
    /// normalized tag exposed by [`FieldSpec::type_tag`]; the registry does not
    /// trim or reinterpret stable identifiers.
    ///
    /// # Errors
    ///
    /// Returns [`SeriesError::DuplicateCodec`] if `type_tag` is already
    /// registered. The original registration remains unchanged.
    pub fn register<T>(&mut self, type_tag: impl Into<Box<str>>) -> Result<(), SeriesError>
    where
        T: Serialize + DeserializeOwned + Clone + Send + 'static,
    {
        self.insert::<T>(type_tag.into(), None)
    }

    /// Registers a concrete type together with a borrowed size estimator.
    ///
    /// The callback receives the original stored `T` by shared reference and
    /// returns an inexpensive estimate in bytes. It must not serialize, clone,
    /// or traverse an entire large buffer merely to obtain an exact encoded
    /// length. For a dense tensor, a typical estimate combines scalar width,
    /// element count, and small shape overhead.
    ///
    /// Estimates guide soft chunk rollover and are not persisted as integrity
    /// facts. The writer records the actual encoded chunk length after IO.
    ///
    /// # Errors
    ///
    /// Returns [`SeriesError::DuplicateCodec`] if `type_tag` is already
    /// registered. The original registration remains unchanged and the new
    /// callback is dropped.
    pub fn register_with_size<T, F>(
        &mut self,
        type_tag: impl Into<Box<str>>,
        estimate: F,
    ) -> Result<(), SeriesError>
    where
        T: Serialize + DeserializeOwned + Clone + Send + 'static,
        F: Fn(&T) -> usize + Send + Sync + 'static,
    {
        self.insert::<T>(type_tag.into(), Some(Box::new(estimate)))
    }

    /// Reports whether an exact stable type tag has a registered codec.
    pub fn contains(&self, type_tag: &str) -> bool {
        self.codecs.contains_key(type_tag)
    }

    /// Returns the number of stable type tags in the registry.
    pub fn len(&self) -> usize {
        self.codecs.len()
    }

    /// Reports whether the registry contains no payload codecs.
    pub fn is_empty(&self) -> bool {
        self.codecs.is_empty()
    }

    /// Returns a borrowed erased Serde view for one populated state field.
    ///
    /// This crate-private boundary lets the JSON format serialize a field as a
    /// map value without knowing its concrete Rust type. The returned object
    /// borrows the payload stored in `state`; no payload owner or backing buffer
    /// is cloned.
    pub(crate) fn value<'a>(
        &self,
        state: &'a SystemState,
        field: &FieldSpec,
    ) -> Result<&'a dyn erased_serde::Serialize, SeriesError> {
        self.get(field.type_tag())?.value(state, field)
    }

    /// Decodes one value and moves it into its declared state field.
    ///
    /// The caller supplies a deserializer positioned at exactly one encoded
    /// payload. Dynamic dispatch selects the concrete registered type, Serde
    /// constructs that value, and [`SystemState::set`] takes ownership.
    pub(crate) fn decode<'de>(
        &self,
        state: &mut SystemState,
        field: &FieldSpec,
        deserializer: &mut dyn erased_serde::Deserializer<'de>,
    ) -> Result<(), SeriesError> {
        self.get(field.type_tag())?
            .decode(state, field, deserializer)
    }

    /// Estimates the in-memory byte contribution of one populated field.
    ///
    /// `None` means the registered type has no estimator. The writer can still
    /// apply its exact state-count threshold. `Some(0)` is distinct and means
    /// the estimator intentionally reported zero bytes.
    pub(crate) fn estimate(
        &self,
        state: &SystemState,
        field: &FieldSpec,
    ) -> Result<Option<usize>, SeriesError> {
        self.get(field.type_tag())?.estimate(state, field)
    }

    /// Inserts one fully constructed typed codec after checking tag uniqueness.
    fn insert<T>(
        &mut self,
        type_tag: Box<str>,
        estimate: Option<Box<SizeEstimator<T>>>,
    ) -> Result<(), SeriesError>
    where
        T: Serialize + DeserializeOwned + Clone + Send + 'static,
    {
        if self.codecs.contains_key(type_tag.as_ref()) {
            return Err(SeriesError::DuplicateCodec {
                type_tag: type_tag.into(),
            });
        }

        self.codecs
            .insert(type_tag, Box::new(TypedCodec { estimate }));
        Ok(())
    }

    /// Resolves an exact stable type tag to its erased codec implementation.
    fn get(&self, type_tag: &str) -> Result<&dyn ErasedCodec, SeriesError> {
        self.codecs
            .get(type_tag)
            .map(Box::as_ref)
            .ok_or_else(|| SeriesError::MissingCodec {
                type_tag: type_tag.to_owned(),
            })
    }
}

impl fmt::Debug for CodecRegistry {
    /// Formats bounded structural information without exposing callbacks or
    /// depending on randomized hash-map iteration order.
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("CodecRegistry")
            .field("codecs", &self.len())
            .finish_non_exhaustive()
    }
}

/// Borrowed, allocation-free estimator stored for one concrete payload type.
type SizeEstimator<T> = dyn Fn(&T) -> usize + Send + Sync + 'static;

/// Object-safe behavior needed from every registered concrete payload type.
///
/// This trait remains private so callers cannot bypass registry uniqueness or
/// the typed bounds enforced by [`CodecRegistry::register`].
trait ErasedCodec: Send + Sync {
    /// Obtains an object-safe borrowed serialization view after exact type
    /// validation.
    fn value<'a>(
        &self,
        state: &'a SystemState,
        field: &FieldSpec,
    ) -> Result<&'a dyn erased_serde::Serialize, SeriesError>;

    /// Reconstructs and inserts one concrete payload from an erased
    /// deserializer.
    fn decode<'de>(
        &self,
        state: &mut SystemState,
        field: &FieldSpec,
        deserializer: &mut dyn erased_serde::Deserializer<'de>,
    ) -> Result<(), SeriesError>;

    /// Runs the optional borrowed size estimator after exact type validation.
    fn estimate(
        &self,
        state: &SystemState,
        field: &FieldSpec,
    ) -> Result<Option<usize>, SeriesError>;
}

/// Concrete implementation hidden behind one [`ErasedCodec`] trait object.
struct TypedCodec<T> {
    estimate: Option<Box<SizeEstimator<T>>>,
}

impl<T> TypedCodec<T>
where
    T: Serialize + DeserializeOwned + Clone + Send + 'static,
{
    /// Borrows and validates the concrete payload stored in `field`.
    ///
    /// Ordinary missing-field and lookup failures remain `StateError` sources.
    /// A concrete type disagreement is promoted to the codec-specific variant
    /// containing both the stable type tag and Rust diagnostic type names.
    fn payload<'a>(&self, state: &'a SystemState, field: &FieldSpec) -> Result<&'a T, SeriesError> {
        match state.get::<T>(field.name()) {
            Ok(payload) => Ok(payload),
            Err(StateError::TypeMismatch { actual, .. }) => Err(SeriesError::CodecTypeMismatch {
                field: field.name().to_owned(),
                type_tag: field.type_tag().to_owned(),
                expected: type_name::<T>(),
                actual,
            }),
            Err(source) => Err(source.into()),
        }
    }
}

impl<T> ErasedCodec for TypedCodec<T>
where
    T: Serialize + DeserializeOwned + Clone + Send + 'static,
{
    fn value<'a>(
        &self,
        state: &'a SystemState,
        field: &FieldSpec,
    ) -> Result<&'a dyn erased_serde::Serialize, SeriesError> {
        self.payload(state, field)
            .map(|payload| payload as &dyn erased_serde::Serialize)
    }

    fn decode<'de>(
        &self,
        state: &mut SystemState,
        field: &FieldSpec,
        deserializer: &mut dyn erased_serde::Deserializer<'de>,
    ) -> Result<(), SeriesError> {
        let payload = erased_serde::deserialize::<T>(deserializer).map_err(|source| {
            SeriesError::DecodePayload {
                field: field.name().to_owned(),
                type_tag: field.type_tag().to_owned(),
                source: Box::new(source),
            }
        })?;

        state.set(field.name(), payload).map_err(SeriesError::from)
    }

    fn estimate(
        &self,
        state: &SystemState,
        field: &FieldSpec,
    ) -> Result<Option<usize>, SeriesError> {
        let Some(estimate) = &self.estimate else {
            // Type validation still occurs when no estimator exists. This
            // prevents a mismatched populated payload from escaping detection
            // merely because byte-based chunking is disabled for its type.
            self.payload(state, field)?;
            return Ok(None);
        };

        self.payload(state, field)
            .map(|payload| Some(estimate(payload)))
    }
}
