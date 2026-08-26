//! Centrally aggregated import scopes.
//!
//! This module owns no behavior. [`basic`] gathers ordinary application APIs;
//! [`advanced`] is its strict superset for integrations and Workflow internals.
//! The existing [`study`] scope remains available while that subsystem awaits
//! its own inference-first refactor.

/// Ordinary application-facing imports.
pub mod basic {
    pub use crate::artifact::{
        ArtifactDescriptor, ArtifactDisposition, ArtifactError, ArtifactLoadError,
        PersistedArtifact, VerifiedArtifact, load_verified_artifact, persist_artifact,
    };
    pub use crate::configuration::{
        ConfigurationError, ConfigurationIter, ProjectPaths, ReplicateFailurePolicy,
        ReplicateScheduling, ReplicateSettings, ResolvedConfiguration, StudyConfiguration,
        StudySettings, WorkloadConfiguration,
    };
    pub use crate::execution::{
        ExecutionScope, ExecutionScopeError, ReplicateContext, ReplicateExecutionError,
        ReplicateExecutor,
    };
    pub use crate::rng_record::{
        DerivedSeed, RNG_RECORDS_METADATA_KEY, ReplicateSeedDeriver, RngRecord, RngRecordError,
    };
    pub use crate::state::basic::*;
    pub use crate::storage::{
        CompletedRecording, CompletedStreamSummary, JsonPayloadDecoder, JsonPayloadDecoderRegistry,
        JsonStringDecoder, JsonVecF64Decoder, RecordingTiming, StateStreamLayout,
        StateStreamStorage, StorageError, StoredStateSeriesReader, SystemStateWriter,
        SystemStateWriterBuilder,
    };
    pub use crate::writer::basic::*;
}

/// Supported imports for advanced users and Workflow integrations.
pub mod advanced {
    pub use super::basic::*;
    #[doc(hidden)]
    pub use crate::state::advanced::PayloadTuple;
    pub use crate::state::advanced::{
        StateFieldSchema, StateMaintenance, StateSchemaAccess, StateSchemaSource,
    };
    pub use crate::writer::advanced::{
        EncodedObservation, Observation, ObservationSink, SessionOutcome, StreamDescriptor,
        WriterDescriptor,
    };
}

/// Existing orchestration imports pending the task/runtime/ui refactor.
pub mod study;
