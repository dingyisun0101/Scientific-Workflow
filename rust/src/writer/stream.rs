//! Scientific stream definitions and validated descriptors.

use std::collections::HashSet;
use std::num::NonZeroU64;

use crate::state::advanced::{StateFieldSchema, StateSchemaAccess, SystemStateSchema};

use super::error::WriterError;
use super::sampling::IterationSampling;

/// One application-defined scientific observation stream.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Stream {
    name: Box<str>,
    fields: FieldSelection,
    sampling: IterationSampling,
}

#[derive(Clone, Debug, Eq, PartialEq)]
enum FieldSelection {
    All,
    Selected(Box<[Box<str>]>),
}

impl Stream {
    /// Defines a named stream containing every field in its bound state schema.
    pub fn all_fields(name: impl Into<String>) -> Result<Self, WriterError> {
        Ok(Self {
            name: normalize_stream_name(name)?,
            fields: FieldSelection::All,
            sampling: IterationSampling::EVERY,
        })
    }

    /// Defines a named stream containing the selected state fields.
    pub fn fields<I, S>(name: impl Into<String>, fields: I) -> Result<Self, WriterError>
    where
        I: IntoIterator<Item = S>,
        S: Into<String>,
    {
        let name = normalize_stream_name(name)?;
        let mut normalized = Vec::new();
        let mut unique = HashSet::new();
        for field in fields {
            let field = field.into();
            let field = field.trim();
            if field.is_empty() {
                return Err(WriterError::EmptyFieldName {
                    stream: name.to_string(),
                });
            }
            if !unique.insert(field.to_owned()) {
                return Err(WriterError::DuplicateField {
                    stream: name.to_string(),
                    field: field.to_owned(),
                });
            }
            normalized.push(Box::<str>::from(field));
        }
        if normalized.is_empty() {
            return Err(WriterError::EmptyFieldSelection {
                stream: name.to_string(),
            });
        }
        Ok(Self {
            name,
            fields: FieldSelection::Selected(normalized.into_boxed_slice()),
            sampling: IterationSampling::EVERY,
        })
    }

    /// Changes this stream's cadence from every iteration to every `iterations`.
    pub fn every_iterations(mut self, iterations: u64) -> Result<Self, WriterError> {
        let Some(iterations) = NonZeroU64::new(iterations) else {
            return Err(WriterError::InvalidSamplingInterval {
                stream: self.name.to_string(),
            });
        };
        self.sampling = IterationSampling::new(iterations);
        Ok(self)
    }

    pub(super) fn name(&self) -> &str {
        &self.name
    }

    pub(super) fn bind(self, schema: &SystemStateSchema) -> Result<StreamDescriptor, WriterError> {
        let fields = match self.fields {
            FieldSelection::All => schema.field_schemas().to_vec(),
            FieldSelection::Selected(selected) => {
                let mut positions = HashSet::with_capacity(selected.len());
                for field in &selected {
                    let declaration =
                        schema
                            .field_schema(field)
                            .ok_or_else(|| WriterError::UnknownField {
                                stream: self.name.to_string(),
                                field: field.to_string(),
                            })?;
                    positions.insert(declaration.position());
                }
                schema
                    .field_schemas()
                    .iter()
                    .filter(|field| positions.contains(&field.position()))
                    .cloned()
                    .collect()
            }
        };
        if fields.is_empty() {
            return Err(WriterError::EmptyFieldSelection {
                stream: self.name.to_string(),
            });
        }
        let covers_complete_state = fields.len() == schema.len();
        Ok(StreamDescriptor {
            name: self.name,
            fields: fields.into_boxed_slice(),
            sampling: self.sampling,
            covers_complete_state,
        })
    }
}

fn normalize_stream_name(name: impl Into<String>) -> Result<Box<str>, WriterError> {
    let name = name.into();
    let name = name.trim();
    if name.is_empty() {
        return Err(WriterError::EmptyStreamName);
    }
    Ok(name.into())
}

/// One stream validated and ordered against a concrete state schema.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct StreamDescriptor {
    name: Box<str>,
    fields: Box<[StateFieldSchema]>,
    sampling: IterationSampling,
    covers_complete_state: bool,
}

impl StreamDescriptor {
    /// Returns the normalized scientific stream name.
    pub fn name(&self) -> &str {
        &self.name
    }

    /// Returns selected fields in canonical state-schema order.
    pub fn fields(&self) -> &[StateFieldSchema] {
        &self.fields
    }

    /// Returns the positive iteration sampling cadence.
    pub fn every_iterations(&self) -> u64 {
        self.sampling.get()
    }

    /// Reports whether this stream contains the complete bound state schema.
    pub fn covers_complete_state(&self) -> bool {
        self.covers_complete_state
    }

    pub(crate) fn includes(&self, iteration: u64) -> bool {
        self.sampling.includes(iteration)
    }
}
