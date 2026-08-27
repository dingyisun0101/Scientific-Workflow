//! Complete expanded model parameters and config-owned typed decoding.

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
    Model {
        parameters: ResolvedModelParameters,
        state: Box<str>,
    },
    Program(ResolvedProgramTask),
}

/// One complete model-parameter combination after deterministic expansion.
#[derive(Clone)]
pub struct ResolvedModelParameters {
    inner: Arc<ResolvedModelParametersInner>,
}

impl ResolvedModelParameters {
    pub(crate) fn new(
        model: Box<str>,
        source_path: PathBuf,
        ordinal: u64,
        value: Value,
        timeout: Option<std::time::Duration>,
    ) -> Self {
        Self {
            inner: Arc::new(ResolvedModelParametersInner {
                model,
                source_path,
                ordinal,
                value,
                timeout,
            }),
        }
    }

    /// Returns the stable manifest key selecting a compiled scientific model.
    pub fn model(&self) -> &str {
        &self.inner.model
    }

    /// Returns the canonical project parameters document path.
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

    /// Decodes this complete resolved parameter combination as one owned value.
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

    /// Borrows the complete resolved value for internal provenance assembly.
    pub(crate) fn resolved_value(&self) -> &Value {
        &self.inner.value
    }
}

impl fmt::Debug for ResolvedModelParameters {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("ResolvedModelParameters")
            .field("model", &self.model())
            .field("source_path", &self.source_path())
            .field("ordinal", &self.ordinal())
            .field("timeout", &self.timeout())
            .finish_non_exhaustive()
    }
}

struct ResolvedModelParametersInner {
    model: Box<str>,
    source_path: PathBuf,
    ordinal: u64,
    value: Value,
    timeout: Option<std::time::Duration>,
}
