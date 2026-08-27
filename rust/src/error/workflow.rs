//! Complete-workflow error type.

use thiserror::Error;

use crate::runtime::advanced::RuntimeError;
use crate::study::advanced::StudyError;

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
