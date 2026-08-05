//! Rust primitives for reproducible scientific workflows.
//!
//! `scientific-workflow` provides the data and execution foundations needed to
//! describe scientific systems, record their evolution, and organize scoped
//! computational work. The crate is intentionally divided by responsibility:
//! state representation, state time series, dispatch, persistence, and
//! language bridges remain separate modules rather than accumulating behind
//! one monolithic interface.
//!
//! # Current module
//!
//! The first implemented module is [`system_state`]. It provides:
//!
//! - JSON-defined, immutable field layouts;
//! - heterogeneous concrete Rust payloads behind a typed API;
//! - clone-free payload insertion, mutation, and extraction;
//! - explicit deep cloning of complete states;
//! - mutable, checked time-point progression.
//!
//! Type erasure and boxing remain internal to that module. Downstream crates
//! work with their original concrete payload types.
//!
//! # Basic use
//!
//! ```no_run
//! use scientific_workflow::system_state::{StateSpec, TimePoint};
//!
//! # fn main() -> Result<(), Box<dyn std::error::Error>> {
//! let spec = StateSpec::load("state.json")?;
//! let mut state = spec.empty(TimePoint::new(0));
//!
//! assert!(
//!     state
//!         .set("population", vec![10_u64, 20, 30])?
//!         .is_none()
//! );
//! state
//!     .get_mut::<Vec<u64>>("population")?
//!     .push(40);
//! let time = state.advance(None)?;
//! assert_eq!(time.index(), 1);
//! let population = state.take::<Vec<u64>>("population")?;
//!
//! assert_eq!(population, vec![10, 20, 30, 40]);
//! # Ok(())
//! # }
//! ```
//!
//! Future time-series and dispatcher modules will build on the same ownership and
//! module-boundary principles without changing the public state-value
//! contract.

pub mod system_state;
