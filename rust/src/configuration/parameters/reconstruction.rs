//! Lazy reconstruction of one complete nested task document.

use serde_json::Value;

use super::super::parameter_tree::reconstruct;
use super::PhaseConfigurationInner;

pub(super) fn document(inner: &PhaseConfigurationInner, ordinal: u64) -> Value {
    reconstruct(inner.fixed_leaves().chain(inner.selected_leaves(ordinal)))
}
