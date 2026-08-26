//! Scientific coordinates attached to one system state.

use super::error::StateError;

/// The scientific time coordinate associated with one system state.
///
/// Every state has a deterministic `iteration`. A state may additionally carry
/// finite `physical_time`, such as seconds or dimensionless model time. This is
/// distinct from operational wall-clock timestamps recorded by Workflow.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct StateTime {
    iteration: u64,
    physical_time: Option<f64>,
}

impl StateTime {
    /// Creates an iteration-only state time.
    pub const fn from_iteration(iteration: u64) -> Self {
        Self {
            iteration,
            physical_time: None,
        }
    }

    /// Creates a state time with a finite physical coordinate.
    ///
    /// Returns `None` for `NaN` or either infinity. Negative finite values are
    /// accepted because scientific coordinates may precede an arbitrary origin.
    pub fn from_iteration_and_physical_time(iteration: u64, physical_time: f64) -> Option<Self> {
        physical_time.is_finite().then_some(Self {
            iteration,
            physical_time: Some(physical_time),
        })
    }

    /// Returns the deterministic iteration coordinate.
    pub const fn iteration(self) -> u64 {
        self.iteration
    }

    /// Returns the optional physical coordinate.
    pub const fn physical_time(self) -> Option<f64> {
        self.physical_time
    }

    /// Computes the next state time without mutating a state.
    ///
    /// `None` advances only the iteration and preserves the optional physical
    /// coordinate. `Some(delta)` also advances an existing physical coordinate.
    pub fn checked_advance(self, physical_time_increment: Option<f64>) -> Result<Self, StateError> {
        let iteration = self
            .iteration
            .checked_add(1)
            .ok_or(StateError::IterationOverflow {
                iteration: self.iteration,
            })?;
        let physical_time = match (self.physical_time, physical_time_increment) {
            (physical_time, None) => physical_time,
            (None, Some(_)) => {
                return Err(StateError::MissingPhysicalTime {
                    iteration: self.iteration,
                });
            }
            (Some(current), Some(delta)) => {
                let next = current + delta;
                if !delta.is_finite() || !next.is_finite() {
                    return Err(StateError::InvalidPhysicalAdvance { current, delta });
                }
                Some(next)
            }
        };
        Ok(Self {
            iteration,
            physical_time,
        })
    }
}
