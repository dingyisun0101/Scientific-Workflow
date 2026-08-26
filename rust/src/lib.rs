//! Rust primitives for reproducible scientific workflows.
//!
//! `scientific-workflow` provides the data and execution foundations needed to
//! describe scientific systems, record their evolution, and organize scoped
//! computational work. The crate is intentionally divided by responsibility:
//! state representation and series, scientific observation, storage, orchestration, and
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
//! - `configuration`: strict study-level replicate settings, study-wide
//!   `parameters.json`, phase-scoped expansion, named paths, and resolved
//!   combinations. It owns input validation and parameter expansion only. It
//!   does not own task construction, state schemas, or persistence.
//! - `state`: typed heterogeneous state values, schema, scientific time, and
//!   ordered in-memory collections.
//! - `writer`: minimal application definitions for scientific streams and the
//!   advanced observation/backend integration boundary.
//! - `storage`: asynchronous buffered persistence and completed-run reconstruction.
//! - `execution`: replicate subprocess dispatch, isolated output scopes, and
//!   directory-scoped recording path derivation.
//! - `artifact`: immutable input content-addressed publication under an execution
//!   scope, plus strict load-time verification.
//! - `rng_record`: lazy named replicate-seed derivation and validated
//!   reproducibility metadata for caller-owned RNG sources.
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
//! [`configuration`] is deliberately outside that hierarchy. It validates
//! process-level replicate policy from `study.json`, resolves a study-wide
//! `parameters.json`, then selects one string-keyed phase whose
//! global, group-shared, and local choices expand into deterministic
//! [`configuration::ResolvedConfiguration`] values. The downstream application
//! decides how each combination becomes a task and owns all schemas, model
//! inputs, storage, and other effects captured by the workload.
//!
//! # Supporting modules
//!
//! [`execution`] dispatches isolated replicate subprocesses, creates their
//! output scopes, and derives deterministic task recording paths. [`artifact`]
//! atomically publishes and verifies content-addressed immutable bytes.
//! [`rng_record`] stores validated RNG provenance while leaving random
//! generation to applications.
//!
//! [`state`] provides:
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
//! The same module provides the in-memory analysis collection for complete,
//! ordered states. It enforces shared-layout identity and increasing
//! iterations, supports ordinary immutable borrowing, and permits field-level
//! mutation without exposing mutable state time. It performs no serialization,
//! chunking, or filesystem IO.
//!
//! [`writer`] provides inferred all-field observation, optional named partial
//! streams and positive iteration cadences, schema-bound descriptors, and
//! clone-free borrowed encoding. [`storage`] adapts those owned encoded
//! observations into bounded asynchronous persistence through one worker per
//! recording, byte-targeted chunking, atomic recording
//! metadata, automatic operational timing, terminal summaries, name-selected payload
//! decoders, and verified full-series or latest-state reconstruction.
//! Import [`prelude::basic`] for these scientific primitives and
//! [`prelude::study`] only at orchestration boundaries.
//!
//! # Basic use
//!
//! ```no_run
//! use std::path::Path;
//!
//! use scientific_workflow::prelude::basic::*;
//!
//! # fn main() -> Result<(), Box<dyn std::error::Error>> {
//! let spec = SystemStateSchema::load_json_template(Path::new("state.json"))?;
//! let mut state = spec.create_empty_state(StateTime::from_iteration(0));
//!
//! state.initialize_payload("population", vec![10_u64, 20, 30])?;
//! state
//!     .payload_mut::<Vec<u64>>("population")?
//!     .push(40);
//! let time = state.advance_time(None)?;
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
//!
//! # Release stability
//!
//! This crate is a test release. Public API behavior is allowed to change across
//! updates without backward compatibility guarantees.
//!
//! ## Downstream no-overlap policy
//!
//! For downstream consumers, preserve boundary ownership:
//! keep orchestration in `study`, observation policy in `writer`, persistence
//! in `storage`, and pure state in `state`. Do not implement overlapping behavior in a
//! downstream layer; if a seam is missing, negotiate an explicit API addition.

#![deny(missing_docs, rustdoc::broken_intra_doc_links)]

mod clock;

pub mod artifact;
pub mod configuration;
pub mod execution;
pub mod prelude;
pub mod rng_record;
pub mod state;
pub mod storage;
#[path = "study.rs"]
pub mod study;
pub mod writer;
