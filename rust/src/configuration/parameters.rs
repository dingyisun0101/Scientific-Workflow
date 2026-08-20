//! Nested fixed and swept parameter definitions with deterministic expansion.

mod resolved;
mod space;
mod task;

pub use space::ParameterSpace;
pub use task::{TaskParameters, TaskParametersIter};

pub(crate) use space::ParameterSpaceInner;
