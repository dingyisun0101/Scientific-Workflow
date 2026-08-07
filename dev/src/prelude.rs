//! Convenient import of the complete supported end-user API.
//!
//! Importing this module brings every crate-owned public structure, error, and
//! extension trait into scope:
//!
//! ```
//! use scientific_workflow::prelude::*;
//!
//! let time = SimulationTime::from_step(0);
//! assert_eq!(time.step(), 0);
//! let decoders = JsonPayloadDecoderRegistry::new();
//! assert!(decoders.is_empty());
//! ```
//!
//! The prelude is explicit rather than a wildcard re-export of internal
//! modules. General-purpose external traits, including Serde traits, remain the
//! responsibility of the application that uses them.

pub use crate::storage::{
    JsonPayloadDecoder, JsonPayloadDecoderRegistry, JsonStringDecoder, JsonVecF64Decoder,
    StateStreamConfig, StorageError, StoredStateSeriesReader, SystemStateWriter,
    SystemStateWriterBuilder, TimeAxisMetadata,
};
pub use crate::system_state::{
    PayloadInsertError, SimulationTime, StateError, StateFieldSchema, SystemState,
    SystemStateSchema,
};
pub use crate::time_series::{
    StateSeries, StateSeriesError, StateSeriesPushError, StateSeriesView,
};
