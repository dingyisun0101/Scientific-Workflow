//! Scientific configuration, state, storage, execution, and artifact APIs.

pub use crate::artifact::{
    ArtifactDescriptor, ArtifactDisposition, ArtifactError, ArtifactLoadError, PersistedArtifact,
    VerifiedArtifact, load_verified_artifact, persist_artifact,
};
pub use crate::configuration::{
    ConfigurationError, MatchingTaskConfigIter, ParameterSpace, ProjectConfig, ProjectPaths,
    TaskConfig, TaskConfigIter, TaskParameters, TaskParametersIter,
};
pub use crate::execution::{ExecutionScope, ExecutionScopeError};
pub use crate::project::{ScientificProject, ScientificProjectError};
pub use crate::rng_record::{RNG_RECORDS_METADATA_KEY, RngRecord, RngRecordError};
pub use crate::storage::{
    CompletedRecording, CompletedStreamSummary, JsonPayloadDecoder, JsonPayloadDecoderRegistry,
    JsonStringDecoder, JsonVecF64Decoder, RecordingTiming, SamplingInterval, StateStreamConfig,
    StorageError, StoredStateSeriesReader, SystemStateWriter, SystemStateWriterBuilder,
    TimeAxisMetadata,
};
pub use crate::system_state::{
    PayloadInsertError, SimulationTime, StateError, StateFieldSchema, StateSchemaSource,
    SystemState, SystemStateSchema,
};
pub use crate::time_series::{
    StateSeries, StateSeriesError, StateSeriesPushError, StateSeriesView,
};
