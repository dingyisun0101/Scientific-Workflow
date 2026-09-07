//! Complete-workflow error composition.
//!
//! The crate root exposes the ordinary facade error; detailed Study and Runtime
//! errors remain owned by their respective subsystem modules.

use thiserror::Error;

use crate::runtime::RuntimeError;
use crate::study::StudyError;

/// A failure while loading, preflighting, or executing one project.
#[derive(Debug, Error)]
#[non_exhaustive]
pub enum WorkflowError {
    /// Declarative study compilation failed before output creation.
    #[error(transparent)]
    Study(#[from] StudyError),
    /// Active execution failed after a valid study was available.
    #[error(transparent)]
    Runtime(#[from] RuntimeError),
}
