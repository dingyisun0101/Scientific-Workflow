//! Rust primitives for reproducible scientific workflows.
//!
//! `scientific-workflow` provides the data and execution foundations needed to
//! describe scientific systems, record their evolution, and organize scoped
//! computational work. The crate is intentionally divided by responsibility:
//! state representation, in-memory state time series, storage, orchestration, and
//! language bridges remain separate modules rather than accumulating behind
//! one monolithic interface.
//!
//! # Module boundaries (public API ownership)
//!
//! The boundary map is strict: each module owns only one slice of behavior, and
//! callers move data between boundaries without duplicating the same concern.
//!
//! - `study`: declarative study/phase/task planning and run execution. It owns
//!   declaration validation, scheduling, cancellation, execution timing, and
//!   progress summaries. It does not own model semantics, storage formats, or
//!   schema declarations.
//! - `configuration`: experiment-space declarations (`fixed.json` + `sweep.json`)
//!   and resolved combinations. It owns parameter expansion only. It does not
//!   own task construction, state schemas, persistence, or execution control.
//! - `system_state`: typed heterogeneous fielded state values and schema.
//! - `time_series`: ordered in-memory complete-state collections for analysis.
//! - `storage`: asynchronous buffered persistence and completed-run reconstruction.
//! - `execution`: directory-scoped recording lifecycle and path derivation.
//! - `artifact`: immutable input content-addressed publication under an execution
//!   scope, plus strict load-time verification.
//! - `rng_record`: validated reproducibility metadata for caller-owned RNG sources.
//! - `prelude`: curated import surfaces that preserve public boundaries.
//!
//! # Study vocabulary
//!
//! A [`study::Study`] is the largest scope. It owns scheduling, cancellation,
//! recording, and display for an ordered set of [`study::Phase`] values. A
//! phase owns many [`study::Task`] values plus their concurrency, delay,
//! timeout, dependency, and failure policies. A task owns one workload, which
//! reports progress, detail, messages, and cancellation through
//! [`study::TaskContext`]. Progress and one-shot work are modes of the same
//! task type.
//!
//! [`configuration`] is deliberately outside that hierarchy. It resolves a
//! directory containing `fixed.json` and `sweep.json` into every deterministic
//! [`configuration::ResolvedConfiguration`]. The downstream application decides
//! how each combination becomes a task and owns all paths, schemas, model
//! inputs, storage, and other effects captured by the workload.
//!
//! # Supporting modules
//!
//! [`execution`] creates collision-resistant or caller-named execution scopes
//! and deterministic task recording paths. [`artifact`] atomically publishes
//! and verifies content-addressed immutable bytes. [`rng_record`] stores
//! validated RNG provenance while leaving random generation to applications.
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
//! metadata, automatic operational timing, terminal summaries, name-selected payload
//! decoders, and verified full-series or latest-state reconstruction.
//! Import [`prelude::basics`] for these scientific primitives and
//! [`prelude::study`] only at orchestration boundaries.
//!
//! # Basic use
//!
//! ```no_run
//! use scientific_workflow::prelude::basics::*;
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
pub mod rng_record;
pub mod storage;
#[path = "study.rs"]
pub mod study;
pub mod system_state;
pub mod time_series;
