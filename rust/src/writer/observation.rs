//! Checked borrowed observations of live scientific state.

use crate::state::advanced::{StateSchemaAccess, StateTime, SystemState};

use super::definition::WriterDescriptor;
use super::encoding::{EncodedObservation, encode};
use super::error::WriterError;
use super::stream::StreamDescriptor;

/// A checked borrowed state observation bound to one writer descriptor.
#[derive(Clone, Copy, Debug)]
pub struct Observation<'a> {
    writer: &'a WriterDescriptor,
    state: &'a SystemState,
}

impl<'a> Observation<'a> {
    /// Validates that `state` shares the writer descriptor's schema allocation.
    pub fn new(writer: &'a WriterDescriptor, state: &'a SystemState) -> Result<Self, WriterError> {
        if !writer.schema().shares_schema_instance(state.schema()) {
            return Err(WriterError::SchemaMismatch {
                iteration: state.time().iteration(),
            });
        }
        Ok(Self { writer, state })
    }

    /// Returns the observed scientific coordinate.
    pub fn time(self) -> StateTime {
        self.state.time()
    }

    /// Returns the borrowed concrete state.
    pub fn state(self) -> &'a SystemState {
        self.state
    }

    /// Returns the writer descriptor used to validate this observation.
    pub fn writer(self) -> &'a WriterDescriptor {
        self.writer
    }

    /// Encodes one stream from this borrowed state in canonical field order.
    pub fn encode_stream(
        self,
        stream: &StreamDescriptor,
    ) -> Result<EncodedObservation, WriterError> {
        encode(self, stream)
    }
}
