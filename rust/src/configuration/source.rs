//! Strict, declaration-ordered JSON source parsing.

use std::collections::HashSet;
use std::fmt;
use std::fs;
use std::path::Path;

use serde::de::{Deserialize, Deserializer, MapAccess, SeqAccess, Visitor};
use serde_json::{Number, Value};

use super::error::ConfigurationError;

/// Duplicate-preserving syntax tree used until semantic validation completes.
pub(crate) enum StrictValue {
    Null,
    Bool(bool),
    Number(Number),
    String(String),
    Array(Vec<Self>),
    Object(Vec<(String, Self)>),
}

impl StrictValue {
    pub(super) fn into_json(self) -> Value {
        match self {
            Self::Null => Value::Null,
            Self::Bool(value) => Value::Bool(value),
            Self::Number(value) => Value::Number(value),
            Self::String(value) => Value::String(value),
            Self::Array(values) => {
                Value::Array(values.into_iter().map(StrictValue::into_json).collect())
            }
            Self::Object(entries) => Value::Object(
                entries
                    .into_iter()
                    .map(|(key, value)| (key, value.into_json()))
                    .collect(),
            ),
        }
    }

    fn duplicate_key(&self) -> Option<&str> {
        match self {
            Self::Array(values) => values.iter().find_map(StrictValue::duplicate_key),
            Self::Object(entries) => {
                let mut seen = HashSet::with_capacity(entries.len());
                for (key, value) in entries {
                    if !seen.insert(key.as_str()) {
                        return Some(key);
                    }
                    if let Some(duplicate) = value.duplicate_key() {
                        return Some(duplicate);
                    }
                }
                None
            }
            Self::Null | Self::Bool(_) | Self::Number(_) | Self::String(_) => None,
        }
    }
}

impl<'de> Deserialize<'de> for StrictValue {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        deserializer.deserialize_any(StrictValueVisitor)
    }
}

struct StrictValueVisitor;

impl<'de> Visitor<'de> for StrictValueVisitor {
    type Value = StrictValue;

    fn expecting(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("any valid JSON value")
    }

    fn visit_unit<E>(self) -> Result<Self::Value, E> {
        Ok(StrictValue::Null)
    }

    fn visit_none<E>(self) -> Result<Self::Value, E> {
        Ok(StrictValue::Null)
    }

    fn visit_some<D>(self, deserializer: D) -> Result<Self::Value, D::Error>
    where
        D: Deserializer<'de>,
    {
        StrictValue::deserialize(deserializer)
    }

    fn visit_bool<E>(self, value: bool) -> Result<Self::Value, E> {
        Ok(StrictValue::Bool(value))
    }

    fn visit_i64<E>(self, value: i64) -> Result<Self::Value, E> {
        Ok(StrictValue::Number(Number::from(value)))
    }

    fn visit_u64<E>(self, value: u64) -> Result<Self::Value, E> {
        Ok(StrictValue::Number(Number::from(value)))
    }

    fn visit_f64<E>(self, value: f64) -> Result<Self::Value, E>
    where
        E: serde::de::Error,
    {
        Number::from_f64(value)
            .map(StrictValue::Number)
            .ok_or_else(|| E::custom("JSON numbers must be finite"))
    }

    fn visit_str<E>(self, value: &str) -> Result<Self::Value, E>
    where
        E: serde::de::Error,
    {
        Ok(StrictValue::String(value.to_owned()))
    }

    fn visit_string<E>(self, value: String) -> Result<Self::Value, E> {
        Ok(StrictValue::String(value))
    }

    fn visit_seq<A>(self, mut sequence: A) -> Result<Self::Value, A::Error>
    where
        A: SeqAccess<'de>,
    {
        let mut values = Vec::with_capacity(sequence.size_hint().unwrap_or(0));
        while let Some(value) = sequence.next_element()? {
            values.push(value);
        }
        Ok(StrictValue::Array(values))
    }

    fn visit_map<A>(self, mut map: A) -> Result<Self::Value, A::Error>
    where
        A: MapAccess<'de>,
    {
        let mut entries = Vec::with_capacity(map.size_hint().unwrap_or(0));
        while let Some((key, value)) = map.next_entry()? {
            entries.push((key, value));
        }
        Ok(StrictValue::Object(entries))
    }
}

pub(crate) fn read_source(path: &Path) -> Result<Vec<u8>, ConfigurationError> {
    fs::read(path).map_err(|source| ConfigurationError::ReadConfigurationFile {
        path: path.to_path_buf(),
        source,
    })
}

pub(crate) fn parse_strict_json(
    path: &Path,
    source: &[u8],
) -> Result<StrictValue, ConfigurationError> {
    let value: StrictValue = serde_json::from_slice(source).map_err(|source| {
        ConfigurationError::ParseConfigurationFile {
            path: path.to_path_buf(),
            source,
        }
    })?;
    if let Some(key) = value.duplicate_key() {
        return Err(ConfigurationError::DuplicateConfigurationKey {
            path: path.to_path_buf(),
            key: key.to_owned(),
        });
    }
    Ok(value)
}

pub(crate) fn require_object(
    path: &Path,
    value: StrictValue,
    reason: impl Into<String>,
) -> Result<Vec<(String, StrictValue)>, ConfigurationError> {
    match value {
        StrictValue::Object(entries) => Ok(entries),
        _ => invalid(path, reason),
    }
}

pub(crate) fn validate_name(path: &Path, name: &str, kind: &str) -> Result<(), ConfigurationError> {
    if name.trim().is_empty() {
        return invalid(
            path,
            format!("{kind} name must not be empty or whitespace-only"),
        );
    }
    Ok(())
}

pub(crate) fn invalid<T>(path: &Path, reason: impl Into<String>) -> Result<T, ConfigurationError> {
    Err(ConfigurationError::InvalidConfigurationDocument {
        path: path.to_path_buf(),
        reason: reason.into(),
    })
}
