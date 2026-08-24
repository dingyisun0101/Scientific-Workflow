//! Nested fixed and swept parameter definitions with deterministic expansion.

mod reconstruction;
mod resolved_configuration;
mod space;

pub use resolved_configuration::{ConfigurationIter, ResolvedConfiguration};
pub use space::{StudyConfiguration, WorkloadConfiguration};

pub(crate) use space::WorkloadConfigurationInner;
