//! Validated Cartesian and correlated-case sweep plans.

mod parse;
mod plan;

pub(crate) use parse::parse_sweep;
pub(crate) use plan::SweepPlan;
