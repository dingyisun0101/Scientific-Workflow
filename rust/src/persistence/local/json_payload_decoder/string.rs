//! JSON decoding for a state field whose concrete payload is [`String`].
//!
//! [`JsonStringDecoder`] converts exactly one raw JSON field value into an owned
//! UTF-8 string. The series reader remains responsible for finding the field
//! by key, selecting this decoder, and inserting the returned string into the
//! matching state slot.
//!
//! The expected representation is a JSON string, including its surrounding
//! quotation marks. Standard JSON escapes are decoded by Serde JSON, so
//! `"line\nvalue"` becomes a string containing an actual newline. Empty strings
//! and all valid Unicode content are accepted. JSON `null`, numbers, booleans,
//! arrays, and objects are rejected rather than converted implicitly.
//!
//! This decoder deliberately performs no trimming, case conversion,
//! normalization, non-empty validation, key lookup, record parsing, state
//! mutation, or filesystem access. Applications requiring a constrained or
//! transformed string should register a custom decoder for that key.

use super::JsonPayloadDecoder;

/// Stateless default decoder for payloads stored as [`String`].
///
/// The unit struct is zero-sized and may be copied freely while configuring
/// decoder registries. Every registry entry still binds it to exactly one key
/// and to the concrete [`String`] output type.
#[derive(Clone, Copy, Debug, Default)]
pub struct JsonStringDecoder;

impl JsonPayloadDecoder<String> for JsonStringDecoder {
    type Error = serde_json::Error;

    /// Deserializes one complete JSON string into an owned [`String`].
    ///
    /// The returned value owns its UTF-8 buffer. On failure, the original
    /// [`serde_json::Error`] is returned so the decoder registry can retain it
    /// as the source of a stream-, index-, and key-aware persistence error.
    fn decode_json_payload(&self, raw_json: &str) -> Result<String, Self::Error> {
        serde_json::from_str(raw_json)
    }
}
