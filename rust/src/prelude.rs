//! Centrally aggregated import scopes.
//!
//! This module owns no behavior. [`basic`] gathers ordinary application APIs;
//! [`advanced`] is its strict superset for integrations and Workflow internals.

/// Ordinary application-facing imports.
pub mod basic {
    pub use crate::WorkflowError;
    #[allow(unused_imports)]
    pub use crate::config::basic::*;
    pub use crate::model;
    pub use crate::observation::basic::*;
    #[allow(unused_imports)]
    pub use crate::persistence::basic::*;
    pub use crate::runtime::basic::*;
    pub use crate::state::basic::*;
    #[allow(unused_imports)]
    pub use crate::study::basic::*;
    pub use crate::task::basic::*;
}

/// Supported imports for advanced users and Workflow integrations.
pub mod advanced {
    pub use super::basic::*;
    pub use crate::config::advanced::*;
    pub use crate::observation::advanced::*;
    pub use crate::persistence::advanced::*;
    pub use crate::runtime::advanced::*;
    #[doc(hidden)]
    pub use crate::state::advanced::PayloadTuple;
    pub use crate::state::advanced::*;
    pub use crate::study::advanced::*;
    #[allow(unused_imports)]
    pub use crate::task::advanced::*;
}
