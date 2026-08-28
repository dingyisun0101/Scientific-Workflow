//! Application execution-unit and private task result boundaries.

/// The result returned by application-defined execution-unit operations.
///
/// Errors must be thread-safe and own every value they reference so a runtime
/// may move them across worker boundaries and retain their source chains.
pub type UnitResult<T = ()> = Result<T, Box<dyn std::error::Error + Send + Sync + 'static>>;

/// Private name for the same erased error boundary inside scheduled tasks.
pub(crate) type TaskResult<T = ()> = UnitResult<T>;
