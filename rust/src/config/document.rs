//! Immutable centrally parsed project source documents.

use std::fmt;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use serde::de::{Deserialize, Deserializer, MapAccess, SeqAccess, Visitor};
use serde_json::{Number, Value};

use super::error::ConfigError;

/// Exact source bytes and canonical path for one validated project document.
#[derive(Clone)]
pub struct ProjectDocument {
    inner: Arc<ProjectDocumentInner>,
}

impl ProjectDocument {
    /// Returns the canonical filesystem path from which this document was read.
    pub fn path(&self) -> &Path {
        &self.inner.path
    }

    /// Borrows the exact source bytes read and validated by config.
    pub fn bytes(&self) -> &[u8] {
        &self.inner.bytes
    }

    pub(crate) fn read(path: &Path) -> Result<(Self, Value), ConfigError> {
        let bytes = std::fs::read(path).map_err(|source| ConfigError::Read {
            path: path.to_path_buf(),
            source,
        })?;
        let strict: StrictValue =
            serde_json::from_slice(&bytes).map_err(|source| ConfigError::Parse {
                path: path.to_path_buf(),
                source,
            })?;
        if let Some(key) = strict.duplicate_key() {
            return Err(ConfigError::DuplicateKey {
                path: path.to_path_buf(),
                key: key.to_owned(),
            });
        }
        let document = Self {
            inner: Arc::new(ProjectDocumentInner {
                path: path.to_path_buf(),
                bytes: bytes.into_boxed_slice(),
            }),
        };
        Ok((document, strict.into_json()))
    }
}

impl fmt::Debug for ProjectDocument {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("ProjectDocument")
            .field("path", &self.path())
            .field("bytes", &self.bytes().len())
            .finish()
    }
}

struct ProjectDocumentInner {
    path: PathBuf,
    bytes: Box<[u8]>,
}

/// The centrally parsed `state.json` document awaiting state-semantic validation.
#[derive(Clone, Debug)]
pub struct StateSchemaDocument {
    source: ProjectDocument,
    value: Arc<Value>,
}

impl StateSchemaDocument {
    pub(crate) fn new(source: ProjectDocument, value: Value) -> Self {
        Self {
            source,
            value: Arc::new(value),
        }
    }

    /// Returns the canonical `state.json` source path.
    pub fn path(&self) -> &Path {
        self.source.path()
    }

    /// Borrows the exact validated `state.json` source bytes.
    pub fn bytes(&self) -> &[u8] {
        self.source.bytes()
    }

    /// Borrows the centrally parsed JSON value for state-semantic validation.
    pub fn json_value(&self) -> &Value {
        &self.value
    }
}

enum StrictValue {
    Null,
    Bool(bool),
    Number(Number),
    String(String),
    Array(Vec<Self>),
    Object(Vec<(String, Self)>),
}

impl StrictValue {
    fn into_json(self) -> Value {
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
            Self::Array(values) => values.iter().find_map(Self::duplicate_key),
            Self::Object(entries) => {
                let mut seen = std::collections::HashSet::with_capacity(entries.len());
                for (key, value) in entries {
                    if !seen.insert(key.as_str()) {
                        return Some(key);
                    }
                    if let Some(key) = value.duplicate_key() {
                        return Some(key);
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
        deserializer.deserialize_any(StrictVisitor)
    }
}

struct StrictVisitor;

impl<'de> Visitor<'de> for StrictVisitor {
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

    fn visit_str<E>(self, value: &str) -> Result<Self::Value, E> {
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
