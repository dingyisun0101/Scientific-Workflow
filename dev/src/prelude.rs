//! Convenient import of the complete supported end-user API.
//!
//! Importing this module brings every crate-owned public structure, error, and
//! extension trait into scope:
//!
//! ```
//! use scientific_workflow::prelude::*;
//!
//! let time = TimePoint::new(0);
//! assert_eq!(time.index(), 0);
//! let decoders = Decoders::new();
//! assert!(decoders.is_empty());
//! ```
//!
//! The prelude is explicit rather than a wildcard re-export of internal
//! modules. General-purpose external traits, including Serde traits, remain the
//! responsibility of the application that uses them.

pub use crate::storage::{
    Decoders, PayloadDecoder, RunOutput, RunOutputBuilder, SeriesReader, StorageError,
    StreamConfig, StringDecoder, TimeAxis, VecF64Decoder,
};
pub use crate::system_state::{FieldSpec, SetError, StateError, StateSpec, SystemState, TimePoint};
pub use crate::time_series::{PushError, SeriesError, SeriesRef, StateSeries};
