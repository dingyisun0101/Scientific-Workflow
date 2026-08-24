//! Strict study settings, named paths, and phase-scoped parameter expansion.
//!
//! [`StudySettings`] loads the process-level replicate policy and preserves one
//! opaque application-owned object from `study.json`. It remains separate from
//! the scientific parameter registry:
//!
//! ```json
//! {
//!   "replicate_settings": {
//!     "replicates": 1,
//!     "scheduling": "sequential",
//!     "failure_policy": "fail_fast",
//!     "base_seed": 1101
//!   },
//!   "application": {
//!     "protocol": "my-application-study"
//!   }
//! }
//! ```
//!
//! A study stores scientific parameters in `config/parameters.json` and named
//! paths independently in `config/paths.json`. Parameter configuration has one
//! global scope and one or more string-keyed components. Each component
//! contains shared parameters and string-keyed workloads:
//!
//! ```text
//! global
//! components
//! └── <component key>
//!     ├── shared
//!     └── workloads
//!         └── <workload key>
//! ```
//!
//! [`StudyConfiguration`] validates the complete registry. Calling
//! [`StudyConfiguration::workload`] returns a [`WorkloadConfiguration`], whose
//! combinations are the Cartesian composition of global, component-shared,
//! and workload-local selections. There is deliberately no component-level
//! combination API: components share values but are not expandable spaces.
//!
//! Ordinary JSON values, including arrays, are literal. An object containing
//! exactly `"$sweep"` declares independent Cartesian choices. A scope-level
//! `"$cases"` array declares correlated alternatives. One scope cannot mix the
//! two forms.
//!
//! ```json
//! {
//!   "global": {
//!     "temperature": {"$sweep": [280.0, 300.0]},
//!     "lattice_shape": [64]
//!   },
//!   "components": {
//!     "models": {
//!       "shared": {"seed": {"$sweep": [7, 11]}},
//!       "workloads": {
//!         "glv": {"solver": {"step": 0.01}},
//!         "analysis": {"include_space": true}
//!       }
//!     }
//!   }
//! }
//! ```
//!
//! ```no_run
//! use scientific_workflow::configuration::{ProjectPaths, StudyConfiguration};
//!
//! # fn main() -> Result<(), Box<dyn std::error::Error>> {
//! let study = StudyConfiguration::load("scientific-study")?;
//! let models = study.workload("models", "glv")?;
//! let paths = ProjectPaths::load("scientific-study")?;
//!
//! for configuration in models.combinations() {
//!     let (temperature, seed): (f64, u64) =
//!         configuration.decode_values(("/temperature", "/seed"))?;
//!     println!("temperature={temperature} seed={seed}");
//! }
//! println!("recordings={}", paths.resolve_path("recordings")?.display());
//! # Ok(())
//! }
//! ```
//!
//! Configuration loading is immutable and side-effect free beyond reading its
//! source. It validates execution policy but does not enact it, create tasks,
//! resolve model semantics, create output, or inspect path targets.
//! Applications pass [`ReplicateSettings`] to the execution module and map
//! [`ResolvedConfiguration`] values into their own workloads.

mod error;
mod parameter_key_tuple;
mod parameter_path;
mod parameter_tree;
mod parameters;
mod paths;
mod settings;
pub(crate) mod source;
mod sweep;

pub use error::ConfigurationError;
#[doc(hidden)]
pub use parameter_key_tuple::ParameterKeyTuple;
pub use parameters::{
    ConfigurationIter, ResolvedConfiguration, StudyConfiguration, WorkloadConfiguration,
};
pub use paths::ProjectPaths;
pub use settings::{ReplicateFailurePolicy, ReplicateScheduling, ReplicateSettings, StudySettings};
