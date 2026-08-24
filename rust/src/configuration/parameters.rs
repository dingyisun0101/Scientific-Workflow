//! Nested fixed and swept parameter definitions with deterministic expansion.

mod reconstruction;
mod resolved_configuration;
mod space;

pub use resolved_configuration::{ConfigurationIter, ResolvedConfiguration};
pub use space::{PhaseConfiguration, StudyConfiguration};

pub(crate) use space::PhaseConfigurationInner;
