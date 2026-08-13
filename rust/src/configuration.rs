//! Standard JSON configuration for scientific projects and parameter sweeps.
//!
//! This module separates immutable project configuration from simulation state,
//! analysis series, persistent state recording, and later task execution. Its
//! standard on-disk layout is:
//!
//! ```text
//! project-root/
//! └── config/
//!     ├── fixed.json
//!     ├── sweep.json
//!     └── paths.json
//! ```
//!
//! - `fixed.json` is an object of parameter values shared by every generated
//!   task.
//! - `sweep.json` is a tagged Cartesian-axis or explicit-case definition.
//! - `paths.json` is an object of named project-wide path strings.
//!
//! [`ProjectConfig`] loads all three files and lazily produces cheap owned
//! [`TaskConfig`] handles that combine one fixed-plus-sweep selection with the
//! shared path table. [`ParameterSpace`] and [`TaskParameters`] remain the
//! lower-level parameter-only API. [`ProjectPaths`] resolves named relative
//! paths against the project root without canonicalization or existence checks.
//!
//! # Basic workflow
//!
//! ```no_run
//! use scientific_workflow::configuration::ProjectConfig;
//!
//! # fn main() -> Result<(), Box<dyn std::error::Error>> {
//! let project = ProjectConfig::load("scientific-project")?;
//! for task in project.task_configs() {
//!     let temperature = task.decode_value::<f64>("temperature")?;
//!     let seed = task.decode_value::<u64>("seed")?;
//!     let output_root = task.resolve_path("output_root")?;
//!     println!(
//!         "task={} temperature={temperature} seed={seed} output={}",
//!         task.task_ordinal(),
//!         output_root.display()
//!     );
//! }
//! # Ok(())
//! # }
//! ```
//!
//! # Ownership and round trips
//!
//! Configuration is immutable after loading. `ParameterSpace`,
//! `TaskParameters`, `TaskConfig`, their iterators, and `ProjectPaths` retain
//! shared parsed allocations; task generation does not clone JSON values or
//! allocate merged maps. Typed decoding is the explicit point at which an
//! application creates an owned Rust value.
//!
//! The three original validated source byte sequences are retained unchanged.
//! [`ProjectConfig::write_source_config`] can therefore reproduce the complete
//! input configuration byte for byte. [`TaskParameters::to_json`] instead
//! emits one deterministic derived fixed-plus-sweep dictionary for provenance
//! or task metadata.
//!
//! # Failure behavior
//!
//! [`ConfigurationError`] retains source paths, exact keys, task ordinals, and
//! underlying IO or Serde errors where applicable. Loaders never publish
//! partially validated objects. Exact export never overwrites an existing
//! `config/` directory.

mod error;
mod parameters;
mod paths;
mod project_config;

pub use error::ConfigurationError;
pub use parameters::{ParameterSpace, TaskParameters, TaskParametersIter};
pub use paths::ProjectPaths;
pub use project_config::{MatchingTaskConfigIter, ProjectConfig, TaskConfig, TaskConfigIter};
