//! Template-defined scientific state and ordered in-memory state collections.
//!
//! Application authors normally import [`basic`] to load a JSON state schema,
//! initialize concrete payloads, evolve one [`basic::SystemState`], and collect
//! complete snapshots in a [`basic::StateSeries`]. Workflow internals and
//! integrations import [`advanced`], which adds schema inspection and explicit
//! structural maintenance without exposing the private erasure or slot layers.

mod error;
mod field;
mod schema;
mod series;
#[allow(clippy::module_inception)]
mod state;
mod time;
mod value;

/// Ordinary application-facing state API.
pub mod basic {
    pub use super::error::{PayloadInsertError, StateError, StateSeriesError};
    pub use super::schema::SystemStateSchema;
    pub use super::series::{StateSeries, StateSeriesPushError};
    pub use super::state::SystemState;
    pub use super::time::StateTime;
}

/// Supported state API for advanced users and Workflow peer subsystems.
///
/// This scope is a strict superset of [`basic`]. Importing its extension traits
/// enables metadata inspection and structural lifecycle operations.
pub mod advanced {
    pub use super::basic::*;
    pub use super::field::StateFieldSchema;
    pub use super::schema::StateSchemaAccess;
    pub(crate) use super::schema::{schema_from_fields, schema_from_json_value};
    #[doc(hidden)]
    pub use super::state::PayloadTuple;
    pub use super::state::StateMaintenance;
    pub(crate) use super::state::StateObservationAccess;
}
