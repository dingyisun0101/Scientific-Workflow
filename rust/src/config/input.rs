//! Complete expanded task inputs and config-owned typed decoding.

use std::fmt;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use serde::de::DeserializeOwned;
use serde_json::Value;

use super::error::ConfigError;
use super::program::ResolvedProgramTask;

/// One centrally resolved generic task declaration.
#[derive(Clone, Debug)]
pub(crate) enum ResolvedTask {
    Model(ResolvedTaskInput),
    Program(ResolvedProgramTask),
}

/// One complete task input after deterministic selection expansion.
#[derive(Clone)]
pub struct ResolvedTaskInput {
    inner: Arc<ResolvedTaskInputInner>,
}

impl ResolvedTaskInput {
    pub(crate) fn new(
        model: Box<str>,
        source_path: PathBuf,
        ordinal: u64,
        value: Value,
        timeout: Option<std::time::Duration>,
    ) -> Self {
        let json = serde_json::to_vec(&value)
            .expect("serializing an already parsed serde_json::Value cannot fail");
        Self {
            inner: Arc::new(ResolvedTaskInputInner {
                model,
                source_path,
                ordinal,
                value,
                json: json.into_boxed_slice(),
                timeout,
            }),
        }
    }

    /// Returns the stable manifest key selecting a compiled scientific model.
    pub fn model(&self) -> &str {
        &self.inner.model
    }

    /// Returns the canonical task input document path.
    pub fn source_path(&self) -> &Path {
        &self.inner.source_path
    }

    /// Returns the zero-based deterministic combination ordinal.
    pub fn ordinal(&self) -> u64 {
        self.inner.ordinal
    }

    /// Returns the optional effective timeout for this task invocation.
    pub fn timeout(&self) -> Option<std::time::Duration> {
        self.inner.timeout
    }

    /// Decodes this complete resolved input as one owned typed value.
    ///
    /// This is the sole supported constants-supply operation. It never rereads
    /// or reparses the source file and contextualizes type errors with the
    /// model key, source path, and combination ordinal.
    pub fn decode<T>(&self) -> Result<T, ConfigError>
    where
        T: DeserializeOwned,
    {
        T::deserialize(&self.inner.value).map_err(|source| ConfigError::DecodeModelConstants {
            model: self.model().to_owned(),
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
            .field("model", &self.model())
            .field("source_path", &self.source_path())
            .field("ordinal", &self.ordinal())
            .field("timeout", &self.timeout())
            .finish_non_exhaustive()
    }
}

struct ResolvedTaskInputInner {
    model: Box<str>,
    source_path: PathBuf,
    ordinal: u64,
    value: Value,
    json: Box<[u8]>,
    timeout: Option<std::time::Duration>,
}
