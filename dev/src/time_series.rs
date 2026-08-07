//! In-memory collections of ordered scientific system states.
//!
//! This module provides the analysis-facing representation of a system-state
//! time series. [`StateSeries`] owns a growable array of complete
//! [`SystemState`](crate::system_state::SystemState) snapshots, while
//! [`StateSeriesView`] provides a lightweight read-only view over that array.
//!
//! # Responsibility boundary
//!
//! A time series is not the simulation's runtime write buffer. Simulations own
//! and evolve a live `SystemState`; the future `storage` module will
//! encode selected fields and queue completed byte records for asynchronous
//! writing. `time_series` is used when states are intentionally collected in
//! memory for analysis, including states reconstructed by a reader.
//!
//! Consequently, this module performs no JSON processing, decoding, file IO,
//! queue management, or chunking. It also has no payload codec registry.
//!
//! # Collection invariants
//!
//! Every state accepted by a series must share its exact immutable
//! [`SystemStateSchema`](crate::system_state::SystemStateSchema) layout allocation and carry a
//! simulation index greater than the current final index. Gaps between indices
//! are valid; optional physical time does not determine ordering.
//!
//! A complete mutable state is never exposed from the collection because its
//! time could then be changed behind the ordering invariant. Use
//! [`StateSeries::payload_mut_at`] to mutate one typed payload at one position.
//!
//! # Ownership
//!
//! Appending, removing, consuming, and iterating an owned series move
//! `SystemState` owners without cloning their payloads. [`StateSeriesPushError`] returns an
//! unchanged rejected state. Explicit [`Clone`] of `StateSeries` is different:
//! it deep-clones every populated payload and should be avoided for lightweight
//! sharing. Use [`StateSeries::as_view`] or `Arc<StateSeries>` instead.
//!
//! # Example
//!
//! ```no_run
//! use scientific_workflow::system_state::{SystemStateSchema, SimulationTime};
//! use scientific_workflow::time_series::StateSeries;
//!
//! # fn main() -> Result<(), Box<dyn std::error::Error>> {
//! let spec = SystemStateSchema::load_json_template("state.json")?;
//! let mut first = spec.create_empty_state(SimulationTime::from_step(0));
//! drop(first.insert_payload("population", vec![10_u64, 20, 30])?);
//!
//! let mut series = StateSeries::new(spec.clone());
//! series.push_state(first)?;
//! series
//!     .payload_mut_at::<Vec<u64>>(0, "population")?
//!     .push(40);
//!
//! let view = series.as_view();
//! assert_eq!(view.len(), 1);
//! assert_eq!(
//!     view.first_state()
//!         .expect("one state was appended")
//!         .payload::<Vec<u64>>("population")?,
//!     &vec![10, 20, 30, 40]
//! );
//! # Ok(())
//! # }
//! ```

mod error;
mod state_series;

pub use error::StateSeriesError;
pub use state_series::{StateSeries, StateSeriesPushError, StateSeriesView};
