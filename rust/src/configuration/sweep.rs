//! Validated Cartesian and correlated-case sweep plans.

mod parse;
mod plan;

pub(crate) use parse::{ParsedScope, parse_scope};
pub(crate) use plan::SweepPlan;
