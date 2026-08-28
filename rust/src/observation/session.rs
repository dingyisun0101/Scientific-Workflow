//! Private observation sampling and encoding session state.

use super::encoding::EncodedObservation;
use super::error::ObservationError;
use super::plan::BoundObservationPlan;
use super::state_observation::StateObservation;

/// Applies one bound observation plan across an ordered state sequence.
pub(crate) struct ObservationSession {
    descriptor: BoundObservationPlan,
    last_iterations: Vec<Option<u64>>,
}

impl ObservationSession {
    pub(crate) fn new(descriptor: BoundObservationPlan) -> Self {
        let last_iterations = vec![None; descriptor.streams().len()];
        Self {
            descriptor,
            last_iterations,
        }
    }

    pub(crate) fn observe(
        &mut self,
        state: &crate::state::SystemState,
    ) -> Result<Vec<EncodedObservation>, ObservationError> {
        self.encode_selected(state, false)
    }

    pub(crate) fn observe_final(
        &mut self,
        state: &crate::state::SystemState,
    ) -> Result<Vec<EncodedObservation>, ObservationError> {
        self.encode_selected(state, true)
    }

    fn encode_selected(
        &mut self,
        state: &crate::state::SystemState,
        terminal: bool,
    ) -> Result<Vec<EncodedObservation>, ObservationError> {
        let iteration = state.time().iteration();
        for (stream, previous) in self.descriptor.streams().iter().zip(&self.last_iterations) {
            if let Some(previous) = previous
                && iteration < *previous
            {
                return Err(ObservationError::NonIncreasingObservation {
                    stream: stream.name().to_owned(),
                    previous: *previous,
                    next: iteration,
                });
            }
        }

        let observation = StateObservation::new(&self.descriptor, state)?;
        let selected = self
            .descriptor
            .streams()
            .iter()
            .enumerate()
            .filter(|(index, stream)| {
                self.last_iterations[*index] != Some(iteration)
                    && (terminal || stream.includes(iteration))
            })
            .collect::<Vec<_>>();
        let encoded = selected
            .iter()
            .map(|(_, stream)| observation.encode_stream(stream))
            .collect::<Result<Vec<_>, _>>()?;
        for (index, _) in selected {
            self.last_iterations[index] = Some(iteration);
        }
        Ok(encoded)
    }
}
