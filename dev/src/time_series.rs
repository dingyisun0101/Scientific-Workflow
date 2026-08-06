//! In-memory collections of ordered scientific system states.
//!
//! This module provides the analysis-facing representation of a system-state
//! time series. [`StateSeries`] owns a growable array of complete
//! [`SystemState`](crate::system_state::SystemState) snapshots, while
//! [`SeriesRef`] provides a lightweight read-only view over that array.
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
//! [`StateSpec`](crate::system_state::StateSpec) layout allocation and carry a
//! simulation index greater than the current final index. Gaps between indices
//! are valid; optional physical time does not determine ordering.
//!
//! A complete mutable state is never exposed from the collection because its
//! time could then be changed behind the ordering invariant. Use
//! [`StateSeries::field_mut`] to mutate one typed payload at one position.
//!
//! # Ownership
//!
//! Appending, removing, consuming, and iterating an owned series move
//! `SystemState` owners without cloning their payloads. [`PushError`] returns an
//! unchanged rejected state. Explicit [`Clone`] of `StateSeries` is different:
//! it deep-clones every populated payload and should be avoided for lightweight
//! sharing. Use [`StateSeries::view`] or `Arc<StateSeries>` instead.
//!
//! # Example
//!
//! ```no_run
//! use scientific_workflow::system_state::{StateSpec, TimePoint};
//! use scientific_workflow::time_series::StateSeries;
//!
//! # fn main() -> Result<(), Box<dyn std::error::Error>> {
//! let spec = StateSpec::load("state.json")?;
//! let mut first = spec.empty(TimePoint::new(0));
//! drop(first.set("population", vec![10_u64, 20, 30])?);
//!
//! let mut series = StateSeries::new(spec.clone());
//! series.push(first)?;
//! series
//!     .field_mut::<Vec<u64>>(0, "population")?
//!     .push(40);
//!
//! let view = series.view();
//! assert_eq!(view.len(), 1);
//! assert_eq!(
//!     view.first()
//!         .expect("one state was appended")
//!         .get::<Vec<u64>>("population")?,
//!     &vec![10, 20, 30, 40]
//! );
//! # Ok(())
//! # }
//! ```

mod error;
mod series;

pub use error::SeriesError;
pub use series::{PushError, SeriesRef, StateSeries};
