//! Private sampling and encoding session state.

use super::definition::WriterDescriptor;
use super::encoding::EncodedObservation;
use super::error::WriterError;
use super::observation::Observation;

/// Applies one validated writer definition across an ordered state sequence.
pub(crate) struct WriterSession {
    descriptor: WriterDescriptor,
    last_iterations: Vec<Option<u64>>,
}

impl WriterSession {
    pub(crate) fn new(descriptor: WriterDescriptor) -> Self {
        let last_iterations = vec![None; descriptor.streams().len()];
        Self {
            descriptor,
            last_iterations,
        }
    }

    pub(crate) fn with_last_iterations(
        descriptor: WriterDescriptor,
        last_iterations: Vec<Option<u64>>,
    ) -> Self {
        assert_eq!(descriptor.streams().len(), last_iterations.len());
        Self {
            descriptor,
            last_iterations,
        }
    }

    pub(crate) fn descriptor(&self) -> &WriterDescriptor {
        &self.descriptor
    }

    pub(crate) fn observe(
        &mut self,
        state: &crate::state::advanced::SystemState,
    ) -> Result<Vec<EncodedObservation>, WriterError> {
        self.encode_selected(state, false)
    }

    pub(crate) fn observe_final(
        &mut self,
        state: &crate::state::advanced::SystemState,
    ) -> Result<Vec<EncodedObservation>, WriterError> {
        self.encode_selected(state, true)
    }

    fn encode_selected(
        &mut self,
        state: &crate::state::advanced::SystemState,
        terminal: bool,
    ) -> Result<Vec<EncodedObservation>, WriterError> {
        let iteration = state.time().iteration();
        for (stream, previous) in self.descriptor.streams().iter().zip(&self.last_iterations) {
            if let Some(previous) = previous
                && iteration < *previous
            {
                return Err(WriterError::NonIncreasingObservation {
                    stream: stream.name().to_owned(),
                    previous: *previous,
                    next: iteration,
                });
            }
        }

        let observation = Observation::new(&self.descriptor, state)?;
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
