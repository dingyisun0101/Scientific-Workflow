//! Deterministic replicate seeds and provenance for application-owned RNGs.
//!
//! [`ReplicateSeedDeriver`] lazily derives a stable named `u64` seed from one
//! study base seed and replicate index. The result includes an [`RngRecord`]
//! ready for storage provenance. Workflow does not construct RNG engines,
//! sample distributions, or maintain RNG cursors.
//!
//! # Boundary
//!
//! This module owns one versioned seed-derivation algorithm plus the metadata
//! schema and deduplication rules for RNG records. Callers retain control of RNG
//! construction, sampling, and state restoration order.

use sha2::{Digest, Sha256};

use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};
use thiserror::Error;

/// Reserved user-metadata key containing records indexed by application namespace.
pub const RNG_RECORDS_METADATA_KEY: &str = "rng_records";

const REPLICATE_SEED_METHOD: &str = "sha256-domain-separated";
const REPLICATE_SEED_VERSION: &str = "scientific-workflow.replicate-seed.v1";

/// Lazy source of deterministic, order-independent named replicate seeds.
///
/// A namespace identifies one scientific random stream, such as `matrix`,
/// `pairing`, or `replacement/task-17`. The same base seed, replicate index,
/// and namespace always produce the same value regardless of process,
/// scheduling, or request order. Different namespaces are domain-separated.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ReplicateSeedDeriver {
    base_seed: u64,
    replicate_index: u64,
}

/// One derived seed together with its exact persistable provenance record.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct DerivedSeed {
    value: u64,
    record: RngRecord,
}

impl ReplicateSeedDeriver {
    /// Creates a lazy deriver for one replicate.
    pub const fn new(base_seed: u64, replicate_index: u64) -> Self {
        Self {
            base_seed,
            replicate_index,
        }
    }

    /// Returns the study-level base seed.
    pub const fn base_seed(self) -> u64 {
        self.base_seed
    }

    /// Returns the zero-based replicate index.
    pub const fn replicate_index(self) -> u64 {
        self.replicate_index
    }

    /// Derives one named stream seed and its matching provenance record.
    ///
    /// Derivation occurs only when requested. The namespace must be nonblank
    /// and should remain stable across executions. The versioned byte contract
    /// is SHA-256 over the method version, base seed, replicate index, namespace
    /// byte length, and namespace bytes; the first eight digest bytes are read
    /// as a big-endian `u64`.
    pub fn derive(self, namespace: &str) -> Result<DerivedSeed, RngRecordError> {
        if namespace.trim().is_empty() {
            return Err(RngRecordError::EmptySeedNamespace);
        }
        let namespace_length =
            u64::try_from(namespace.len()).map_err(|_| RngRecordError::SeedNamespaceTooLong)?;
        let mut hasher = Sha256::new();
        hasher.update(REPLICATE_SEED_VERSION.as_bytes());
        hasher.update([0]);
        hasher.update(self.base_seed.to_be_bytes());
        hasher.update(self.replicate_index.to_be_bytes());
        hasher.update(namespace_length.to_be_bytes());
        hasher.update(namespace.as_bytes());
        let digest = hasher.finalize();
        let value = u64::from_be_bytes(
            digest[..8]
                .try_into()
                .expect("SHA-256 always contains at least eight bytes"),
        );
        let parameters = Map::from_iter([
            ("base_seed".to_owned(), Value::from(self.base_seed)),
            (
                "replicate_index".to_owned(),
                Value::from(self.replicate_index),
            ),
        ]);
        let record = RngRecord::new(
            namespace,
            REPLICATE_SEED_METHOD,
            REPLICATE_SEED_VERSION,
            "u64-decimal",
            value.to_string(),
            Some(parameters),
        )?;
        Ok(DerivedSeed { value, record })
    }
}

impl DerivedSeed {
    /// Returns the derived `u64` suitable for application RNG construction.
    pub const fn value(&self) -> u64 {
        self.value
    }

    /// Borrows the exact record that should accompany persisted RNG output.
    pub const fn record(&self) -> &RngRecord {
        &self.record
    }

    /// Splits the result into its seed and owned provenance record.
    pub fn into_parts(self) -> (u64, RngRecord) {
        (self.value, self.record)
    }
}

/// Immutable identity of one application-owned random source.
///
/// Keys are persisted as plain text and therefore must be reproducibility
/// material rather than secrets. `method` and `version` should identify every
/// implementation detail that affects the produced sequence, including a
/// distribution transform when relevant. When an upstream scientific API
/// accepts optional RNG settings, copy its resolved method and seed here rather
/// than recording the unresolved request.
#[derive(Clone, Debug, Eq, PartialEq, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct RngRecord {
    namespace: String,
    method: String,
    version: String,
    key_encoding: String,
    key: String,
    #[serde(default, skip_serializing_if = "Map::is_empty")]
    parameters: Map<String, Value>,
}

impl RngRecord {
    /// Creates and validates one application-namespaced RNG record.
    pub fn new(
        namespace: impl Into<String>,
        method: impl Into<String>,
        version: impl Into<String>,
        key_encoding: impl Into<String>,
        key: impl Into<String>,
        parameters: Option<Map<String, Value>>,
    ) -> Result<Self, RngRecordError> {
        let record = Self {
            namespace: namespace.into(),
            method: method.into(),
            version: version.into(),
            key_encoding: key_encoding.into(),
            key: key.into(),
            parameters: parameters.unwrap_or_default(),
        };
        record.validate()?;
        Ok(record)
    }

    /// Returns the collision domain used inside recording metadata.
    pub fn namespace(&self) -> &str {
        &self.namespace
    }

    /// Returns the application-declared RNG or sampling method.
    pub fn method(&self) -> &str {
        &self.method
    }

    /// Returns the application-declared sequence-affecting version.
    pub fn version(&self) -> &str {
        &self.version
    }

    /// Returns the application-declared key representation.
    pub fn key_encoding(&self) -> &str {
        &self.key_encoding
    }

    /// Returns the persisted reproducibility key.
    pub fn key(&self) -> &str {
        &self.key
    }

    /// Borrows opaque application-defined method parameters.
    pub const fn parameters(&self) -> &Map<String, Value> {
        &self.parameters
    }

    /// Inserts this record beneath the reserved user-metadata object.
    ///
    /// Existing unrelated metadata is preserved. Reusing a namespace is
    /// rejected rather than overwritten.
    pub fn insert_into_metadata(
        &self,
        metadata: &mut Map<String, Value>,
    ) -> Result<(), RngRecordError> {
        self.validate()?;
        let records = metadata
            .entry(RNG_RECORDS_METADATA_KEY.to_owned())
            .or_insert_with(|| Value::Object(Map::new()))
            .as_object_mut()
            .ok_or(RngRecordError::InvalidMetadataShape)?;
        if records.contains_key(&self.namespace) {
            return Err(RngRecordError::DuplicateNamespace {
                namespace: self.namespace.clone(),
            });
        }
        records.insert(
            self.namespace.clone(),
            serde_json::to_value(self).expect("RNG records contain only JSON-compatible values"),
        );
        Ok(())
    }

    /// Reads and validates one namespaced record from user metadata.
    pub fn from_metadata(
        metadata: &Map<String, Value>,
        namespace: &str,
    ) -> Result<Option<Self>, RngRecordError> {
        let Some(value) = metadata.get(RNG_RECORDS_METADATA_KEY) else {
            return Ok(None);
        };
        let records = value
            .as_object()
            .ok_or(RngRecordError::InvalidMetadataShape)?;
        let Some(value) = records.get(namespace) else {
            return Ok(None);
        };
        let record: Self = serde_json::from_value(value.clone()).map_err(|source| {
            RngRecordError::InvalidStoredRecord {
                namespace: namespace.to_owned(),
                source,
            }
        })?;
        record.validate()?;
        if record.namespace != namespace {
            return Err(RngRecordError::NamespaceMismatch {
                index: namespace.to_owned(),
                record: record.namespace,
            });
        }
        Ok(Some(record))
    }

    fn validate(&self) -> Result<(), RngRecordError> {
        for (field, value) in [
            ("namespace", self.namespace.as_str()),
            ("method", self.method.as_str()),
            ("version", self.version.as_str()),
            ("key_encoding", self.key_encoding.as_str()),
            ("key", self.key.as_str()),
        ] {
            if value.trim().is_empty() {
                return Err(RngRecordError::EmptyField { field });
            }
        }
        Ok(())
    }
}

/// Rejection while constructing or embedding RNG provenance.
#[derive(Debug, Error)]
#[non_exhaustive]
pub enum RngRecordError {
    /// A requested replicate stream has no stable identity.
    #[error("replicate seed namespace must not be empty or whitespace-only")]
    EmptySeedNamespace,
    /// A namespace cannot be represented by the versioned length encoding.
    #[error("replicate seed namespace is too long")]
    SeedNamespaceTooLong,
    /// A required textual identity field is empty or whitespace-only.
    #[error("RNG record field `{field}` must not be empty")]
    EmptyField {
        /// Rejected stable field name.
        field: &'static str,
    },
    /// A metadata map already contains a record for the namespace.
    #[error("RNG namespace `{namespace}` is recorded more than once")]
    DuplicateNamespace {
        /// Repeated application namespace.
        namespace: String,
    },
    /// The reserved metadata entry is not an object indexed by namespace.
    #[error("user metadata `{RNG_RECORDS_METADATA_KEY}` entry must be an object")]
    InvalidMetadataShape,
    /// A stored namespaced value does not decode as an RNG record.
    #[error("invalid RNG record for namespace `{namespace}`")]
    InvalidStoredRecord {
        /// Namespace selected by the caller.
        namespace: String,
        /// JSON type or field failure.
        #[source]
        source: serde_json::Error,
    },
    /// The metadata index and embedded record namespace disagree.
    #[error("RNG metadata index `{index}` contains record namespace `{record}`")]
    NamespaceMismatch {
        /// Namespace used as the metadata object key.
        index: String,
        /// Namespace embedded in the record.
        record: String,
    },
}
