//! Rust primitives for reproducible scientific workflows.
//!
//! `scientific-workflow` provides the data and execution foundations needed to
//! describe scientific systems, record their evolution, and organize scoped
//! computational work. The crate is intentionally divided by responsibility:
//! state representation, in-memory state time series, storage, dispatch, and
//! language bridges remain separate modules rather than accumulating behind
//! one monolithic interface.
//!
//! # Current modules
//!
//! [`configuration`] provides the standard `config/{fixed,sweep,paths}.json`
//! project layout, deterministic Cartesian or explicit-case task expansion,
//! immutable dict-like resolved parameters, named path resolution, and
//! byte-exact source export.
//!
//! [`system_state`] provides:
//!
//! - JSON-defined, immutable field layouts;
//! - optional natural-language field descriptions without persisted Rust types;
//! - heterogeneous concrete Rust payloads behind a typed API;
//! - clone-free payload insertion, mutation, and extraction;
//! - explicit per-payload cloning of complete states;
//! - mutable, checked time-point progression.
//!
//! Type erasure and boxing remain internal to that module. Downstream crates
//! work with their original concrete payload types.
//!
//! [`time_series`] provides the in-memory analysis collection for complete,
//! ordered states. It enforces shared-layout identity and increasing simulation
//! indices, offers a lightweight borrowed view, and permits field-level
//! mutation without exposing mutable state time. It deliberately performs no
//! serialization, chunking, or filesystem IO.
//!
//! [`storage`] provides named partial-state streams with writer-owned sampling
//! sampling intervals, borrowed JSON encoding only when due, bounded asynchronous
//! persistence through one worker per recording, byte-targeted chunking, atomic recording
//! metadata, per-key payload decoders, and verified analysis reconstruction.
//! Import [`prelude`] when an application wants the complete supported API in
//! scope without listing each module separately.
//!
//! # Basic use
//!
//! ```no_run
//! use scientific_workflow::prelude::*;
//!
//! # fn main() -> Result<(), Box<dyn std::error::Error>> {
//! let spec = SystemStateSchema::load_json_template("state.json")?;
//! let mut state = spec.create_empty_state(SimulationTime::from_iteration(0));
//!
//! assert!(
//!     state
//!         .insert_payload("population", vec![10_u64, 20, 30])?
//!         .is_none()
//! );
//! state
//!     .payload_mut::<Vec<u64>>("population")?
//!     .push(40);
//! let time = state.advance_simulation_time(None)?;
//! assert_eq!(time.iteration(), 1);
//! let population = state.take_payload::<Vec<u64>>("population")?;
//!
//! assert_eq!(population, vec![10, 20, 30, 40]);
//! # Ok(())
//! # }
//! ```
//!
//! Future dispatcher functionality will organize scoped workflow execution
//! without changing the public state-value ownership or storage contracts.

pub mod configuration;
pub mod prelude;
pub mod storage;
pub mod system_state;
pub mod time_series;
