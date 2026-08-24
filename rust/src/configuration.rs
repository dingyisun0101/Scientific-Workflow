//! Pure fixed-and-sweep configuration expansion.
//!
//! The canonical API reads one directory containing exactly the parameter
//! inputs relevant to combination expansion:
//!
//! ```text
//! config/
//! ├── fixed.json
//! └── sweep.json
//! ```
//!
//! - `fixed.json` is an arbitrarily nested object of leaves shared by every
//!   resolved configuration.
//! - `sweep.json` is a tagged nested Cartesian-axis or explicit-case definition.
//!
//! [`ConfigurationSpace`] validates those two documents and lazily produces
//! every [`ResolvedConfiguration`]. It does not know about tasks, phases,
//! studies, workloads, display, storage, or scientific state. Callers decide
//! how each resolved configuration is used. The independent [`ProjectPaths`]
//! utility strictly validates `config/paths.json` when a downstream project
//! uses the conventional named-path document.
//!
//! # Basic workflow
//!
//! ```no_run
//! use scientific_workflow::configuration::ConfigurationSpace;
//!
//! # fn main() -> Result<(), Box<dyn std::error::Error>> {
//! let configurations = ConfigurationSpace::load("scientific-study/config")?;
//! for configuration in configurations.combinations() {
//!     let (temperature, seed): (f64, u64) =
//!         configuration.decode_values(("/temperature", "/seed"))?;
//!     println!(
//!         "combination={} temperature={temperature} seed={seed}",
//!         configuration.ordinal(),
//!     );
//! }
//! # Ok(())
//! # }
//! ```
//!
//! # Ownership and round trips
//!
//! Configuration is immutable after loading. `ConfigurationSpace`,
//! `ResolvedConfiguration`, and `ConfigurationIter` retain shared parsed leaf
//! allocations. Exact leaf lookup does not clone JSON. Nested subtrees spanning
//! fixed and swept leaves are reconstructed lazily and remain ordinary nested
//! JSON to callers.
//!
//! The two original validated source byte sequences are retained unchanged.
//! [`ResolvedConfiguration::to_json`] emits one deterministic derived
//! fixed-plus-sweep dictionary when the caller needs serialization.
//!
//! # Failure behavior
//!
//! [`ConfigurationError`] retains source paths, exact keys, combination ordinals, and
//! underlying IO or Serde errors where applicable. Loaders never publish
//! partially validated objects.

mod error;
mod parameter_key_tuple;
mod parameter_path;
mod parameter_tree;
mod parameters;
mod paths;
pub(crate) mod source;
mod sweep;

pub use error::ConfigurationError;
#[doc(hidden)]
pub use parameter_key_tuple::ParameterKeyTuple;
pub use parameters::{ConfigurationIter, ConfigurationSpace, ResolvedConfiguration};
pub use paths::ProjectPaths;
