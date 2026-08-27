//! JSON decoding for a state field whose concrete payload is `Vec<f64>`.
//!
//! [`JsonVecF64Decoder`] converts exactly one raw JSON field value into an owned
//! vector of double-precision values. The
//! [`StoredStateSeriesReader`](crate::persistence::advanced::StoredStateSeriesReader)
//! remains responsible for finding the field by key, selecting this decoder,
//! and inserting the returned vector into the matching state slot.
//!
//! The expected JSON representation is an array of JSON numbers accepted by
//! Serde JSON for `f64`, for example `[1.0,-2.5,3.25]`.
//! Deserialization allocates the final vector directly and does not first build
//! a [`serde_json::Value`] tree. An empty array is valid. JSON `null`, scalar
//! values, nested arrays, and non-numeric elements are rejected by Serde.
//!
//! This decoder performs no key lookup, record parsing, state mutation,
//! filesystem access, or domain-specific validation of vector length and
//! values. Applications needing constraints such as a fixed dimension or a
//! finite-only vector should register a custom decoder for that key.

use super::JsonPayloadDecoder;

/// Stateless default decoder for payloads stored as `Vec<f64>`.
///
/// The unit struct is zero-sized and may be copied freely while configuring
/// decoder registries. Each registry entry still binds it to exactly one key
/// and the concrete `Vec<f64>` output type.
#[derive(Clone, Copy, Debug, Default)]
pub struct JsonVecF64Decoder;

impl JsonPayloadDecoder<Vec<f64>> for JsonVecF64Decoder {
    type Error = serde_json::Error;

    /// Deserializes one complete raw JSON array directly into `Vec<f64>`.
    ///
    /// The returned vector owns its allocation. On failure, the original
    /// `serde_json::Error` is returned so the decoder registry can retain it as
    /// the source of a stream-, index-, and key-aware persistence error.
    fn decode_json_payload(&self, raw_json: &str) -> Result<Vec<f64>, Self::Error> {
        serde_json::from_str(raw_json)
    }
}
