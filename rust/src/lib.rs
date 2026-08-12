//! Rust primitives for reproducible scientific workflows.
//!
//! `scientific-workflow` provides the data and execution foundations needed to
//! describe scientific systems, record their evolution, and organize scoped
//! computational work. The crate is intentionally divided by responsibility:
//! state representation, in-memory state time series, storage, orchestration, and
//! language bridges remain separate modules rather than accumulating behind
//! one monolithic interface.
//!
//! # Current modules
//!
//! [`configuration`] provides the standard `config/{fixed,sweep,paths}.json`
//! project layout, deterministic Cartesian or explicit-case task expansion,
//! complete cheap task-configuration handles, exact sweep-value selection,
//! named path resolution, and byte-exact source export.
//! [`project`] combines task configuration with either a project-owned
//! `config/state.json` schema or a canonical schema supplied by a fixed-model
//! crate as one immutable [`project::ScientificProject`]. [`execution`] creates
//! collision-resistant or caller-named execution scopes and deterministic task
//! recording paths without taking ownership away from storage writers.
//! [`artifact`] atomically publishes and verifies content-addressed immutable
//! bytes while leaving their scientific representation to consumer crates.
//! [`reporting`] provides parameter-identified parallel progress tracking and
//! one process-wide human-facing terminal owner. [`rng_record`] provides only
//! validated, persisted RNG provenance records; random generation remains an
//! application responsibility.
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
//! Type erasure and boxing remain internal to that module. Consumer crates
//! work with their original concrete payload types.
//!
//! [`time_series`] provides the in-memory analysis collection for complete,
//! ordered states. It enforces shared-layout identity and increasing simulation
//! indices, offers a lightweight borrowed view, and permits field-level
//! mutation without exposing mutable state time. It deliberately performs no
//! serialization, chunking, or filesystem IO.
//!
//! [`storage`] provides named partial-state streams with writer-owned sampling
//! intervals, borrowed JSON encoding only when due, bounded asynchronous
//! persistence through one worker per recording, byte-targeted chunking, atomic recording
//! metadata, automatic operational timing, terminal summaries, per-key payload
//! decoders, and verified full-series or latest-state reconstruction.
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
//! Future orchestration-layer features will organize scoped workflow execution
//! without changing the public state-value ownership or storage contracts.

mod clock;

pub mod artifact;
pub mod configuration;
pub mod execution;
pub mod prelude;
pub mod project;
pub mod reporting;
pub mod rng_record;
pub mod storage;
pub mod system_state;
pub mod time_series;
