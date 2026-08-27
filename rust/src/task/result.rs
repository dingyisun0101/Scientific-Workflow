//! Application task result boundary.

/// The result returned by application-defined task operations.
///
/// Errors must be thread-safe and own every value they reference so a runtime
/// may move them across worker boundaries and retain their source chains.
pub type TaskResult<T = ()> = Result<T, Box<dyn std::error::Error + Send + Sync + 'static>>;
