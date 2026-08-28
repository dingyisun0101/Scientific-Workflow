//! Template-defined scientific state and ordered in-memory state collections.
//!
//! The module root exposes construction, manipulation, inspection, and explicit
//! maintenance without exposing the private erasure or slot layers.

mod error;
mod field;
mod schema;
mod series;
#[allow(clippy::module_inception)]
mod state;
mod time;
mod value;

pub use error::{PayloadInsertError, StateError, StateSeriesError};
pub use field::StateFieldSchema;
pub use schema::{StateSchemaProvider, SystemStateSchema};
pub(crate) use schema::{schema_from_fields, schema_from_json_bytes, schema_from_json_value};
pub use series::{StateSeries, StateSeriesPushError};
#[doc(hidden)]
pub use state::PayloadTuple;
pub(crate) use state::StateObservationAccess;
pub use state::SystemState;
pub use time::StateTime;
