//! Lazy reconstruction of one complete nested task document.

use serde_json::Value;

use super::super::parameter_tree::reconstruct;
use super::ParameterSpaceInner;

pub(super) fn document(inner: &ParameterSpaceInner, ordinal: u64) -> Value {
    reconstruct(
        inner
            .fixed
            .iter()
            .chain(inner.sweep.selected_leaves(ordinal)),
    )
}
