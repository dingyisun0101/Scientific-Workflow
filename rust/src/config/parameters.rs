//! Complete expanded execution-unit parameters and config-owned typed decoding.

use std::fmt;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use serde::de::DeserializeOwned;
use serde_json::Value;

use super::error::ConfigError;
use super::program::ResolvedProgramTask;
use super::store::ConfigSnapshot;

/// One centrally resolved generic task declaration.
#[derive(Clone, Debug)]
pub(crate) enum ResolvedTask {
    ExecutionUnit {
        configuration: usize,
        snapshot: ConfigSnapshot,
        parameters: ResolvedExecutionUnitParameters,
        state: Option<Box<str>>,
    },
    Program {
        configuration: usize,
        snapshot: ConfigSnapshot,
        program: ResolvedProgramTask,
    },
}

impl ResolvedTask {
    pub(crate) const fn configuration(&self) -> usize {
        match self {
            Self::ExecutionUnit { configuration, .. } | Self::Program { configuration, .. } => {
                *configuration
            }
        }
    }

    pub(crate) fn snapshot(&self) -> &ConfigSnapshot {
        match self {
            Self::ExecutionUnit { snapshot, .. } | Self::Program { snapshot, .. } => snapshot,
        }
    }
}

/// One complete execution-unit parameter combination after deterministic expansion.
#[derive(Clone)]
pub(crate) struct ResolvedExecutionUnitParameters {
    inner: Arc<ResolvedExecutionUnitParametersInner>,
}

impl ResolvedExecutionUnitParameters {
    pub(crate) fn new(
        execution_unit: Box<str>,
        source_path: PathBuf,
        ordinal: u64,
        value: Value,
        timeout: Option<std::time::Duration>,
    ) -> Self {
        Self {
            inner: Arc::new(ResolvedExecutionUnitParametersInner {
                execution_unit,
                source_path,
                ordinal,
                value,
                timeout,
            }),
        }
    }

    /// Returns the stable manifest key selecting a compiled execution unit.
    pub(crate) fn execution_unit(&self) -> &str {
        &self.inner.execution_unit
    }

    /// Returns the canonical project parameters document path.
    pub(crate) fn source_path(&self) -> &Path {
        &self.inner.source_path
    }

    /// Returns the zero-based deterministic combination ordinal.
    pub(crate) fn ordinal(&self) -> u64 {
        self.inner.ordinal
    }

    /// Returns the optional effective timeout for this task invocation.
    pub(crate) fn timeout(&self) -> Option<std::time::Duration> {
        self.inner.timeout
    }

    /// Decodes this complete resolved parameter combination as one owned value.
    ///
    /// This is the sole supported constants-supply operation. It never rereads
    /// or reparses the source file and contextualizes type errors with the
    /// execution-unit key, source path, and combination ordinal.
    pub(crate) fn decode<T>(&self) -> Result<T, ConfigError>
    where
        T: DeserializeOwned,
    {
        T::deserialize(&self.inner.value).map_err(|source| {
            ConfigError::DecodeExecutionUnitConstants {
                execution_unit: self.execution_unit().to_owned(),
                path: self.source_path().to_path_buf(),
                ordinal: self.ordinal(),
                source,
            }
        })
    }

    /// Borrows the complete resolved value for internal provenance assembly.
    pub(crate) fn resolved_value(&self) -> &Value {
        &self.inner.value
    }
}

impl fmt::Debug for ResolvedExecutionUnitParameters {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("ResolvedExecutionUnitParameters")
            .field("execution_unit", &self.execution_unit())
            .field("source_path", &self.source_path())
            .field("ordinal", &self.ordinal())
            .field("timeout", &self.timeout())
            .finish_non_exhaustive()
    }
}

struct ResolvedExecutionUnitParametersInner {
    execution_unit: Box<str>,
    source_path: PathBuf,
    ordinal: u64,
    value: Value,
    timeout: Option<std::time::Duration>,
}
