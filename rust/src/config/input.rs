//! Complete expanded task inputs and config-owned typed decoding.

use std::fmt;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use serde::de::DeserializeOwned;
use serde_json::Value;

use super::error::ConfigError;

/// One complete task input after deterministic selection expansion.
#[derive(Clone)]
pub struct ResolvedTaskInput {
    inner: Arc<ResolvedTaskInputInner>,
}

impl ResolvedTaskInput {
    pub(crate) fn new(
        definition: Box<str>,
        source_path: PathBuf,
        ordinal: u64,
        value: Value,
        display_fields: Arc<[Box<str>]>,
        timeout: Option<std::time::Duration>,
    ) -> Self {
        let json = serde_json::to_vec(&value)
            .expect("serializing an already parsed serde_json::Value cannot fail");
        Self {
            inner: Arc::new(ResolvedTaskInputInner {
                definition,
                source_path,
                ordinal,
                value,
                json: json.into_boxed_slice(),
                display_fields,
                timeout,
            }),
        }
    }

    /// Returns the manifest key selecting compiled task behavior.
    pub fn definition(&self) -> &str {
        &self.inner.definition
    }

    /// Returns the canonical task input document path.
    pub fn source_path(&self) -> &Path {
        &self.inner.source_path
    }

    /// Returns the zero-based deterministic combination ordinal.
    pub fn ordinal(&self) -> u64 {
        self.inner.ordinal
    }

    /// Iterates additional scientific state fields selected for display.
    pub fn display_fields(&self) -> impl ExactSizeIterator<Item = &str> {
        self.inner.display_fields.iter().map(Box::as_ref)
    }

    /// Returns the optional effective timeout for this task invocation.
    pub fn timeout(&self) -> Option<std::time::Duration> {
        self.inner.timeout
    }

    /// Decodes this complete resolved input as one owned typed value.
    ///
    /// This is the sole supported constants-supply operation. It never rereads
    /// or reparses the source file and contextualizes type errors with the
    /// definition, source path, and combination ordinal.
    pub fn decode<T>(&self) -> Result<T, ConfigError>
    where
        T: DeserializeOwned,
    {
        T::deserialize(&self.inner.value).map_err(|source| ConfigError::DecodeTaskInput {
            definition: self.definition().to_owned(),
            path: self.source_path().to_path_buf(),
            ordinal: self.ordinal(),
            source,
        })
    }

    /// Borrows deterministic compact JSON for provenance persistence.
    pub fn resolved_json(&self) -> &[u8] {
        &self.inner.json
    }
}

impl fmt::Debug for ResolvedTaskInput {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("ResolvedTaskInput")
            .field("definition", &self.definition())
            .field("source_path", &self.source_path())
            .field("ordinal", &self.ordinal())
            .field("display_fields", &self.inner.display_fields)
            .field("timeout", &self.timeout())
            .finish_non_exhaustive()
    }
}

struct ResolvedTaskInputInner {
    definition: Box<str>,
    source_path: PathBuf,
    ordinal: u64,
    value: Value,
    json: Box<[u8]>,
    display_fields: Arc<[Box<str>]>,
    timeout: Option<std::time::Duration>,
}
