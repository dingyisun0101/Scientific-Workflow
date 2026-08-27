//! Checked borrowed observations of live scientific state.

use crate::state::advanced::{StateSchemaAccess, StateTime, SystemState};

use super::encoding::{EncodedObservation, encode};
use super::error::ObservationError;
use super::plan::BoundObservationPlan;
use super::stream::BoundObservationStream;

/// A checked borrowed state observation bound to one observation plan.
#[derive(Clone, Copy, Debug)]
pub(crate) struct StateObservation<'a> {
    state: &'a SystemState,
}

impl<'a> StateObservation<'a> {
    /// Validates that `state` shares the bound plan's schema allocation.
    pub(crate) fn new(
        plan: &'a BoundObservationPlan,
        state: &'a SystemState,
    ) -> Result<Self, ObservationError> {
        if !plan.schema().shares_schema_instance(state.schema()) {
            return Err(ObservationError::SchemaMismatch {
                iteration: state.time().iteration(),
            });
        }
        Ok(Self { state })
    }

    /// Returns the observed scientific coordinate.
    pub(crate) fn time(self) -> StateTime {
        self.state.time()
    }

    /// Returns the borrowed concrete state.
    pub(crate) fn state(self) -> &'a SystemState {
        self.state
    }

    /// Encodes one stream from this borrowed state in canonical field order.
    pub(crate) fn encode_stream(
        self,
        stream: &BoundObservationStream,
    ) -> Result<EncodedObservation, ObservationError> {
        encode(self, stream)
    }
}
