//! Writer definitions and schema-bound descriptors.

use std::collections::HashSet;

use crate::state::advanced::SystemStateSchema;

use super::error::WriterError;
use super::stream::{Stream, StreamDescriptor};

/// An immutable application definition of scientific output.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Writer {
    streams: Box<[Stream]>,
    iteration_unit: Option<Box<str>>,
    physical_time_unit: Option<Box<str>>,
}

impl Writer {
    /// Defines one inferred stream named `state` containing every state field.
    pub fn all_fields() -> Self {
        Self {
            streams: vec![Stream::all_fields("state").expect("the inferred name is valid")]
                .into_boxed_slice(),
            iteration_unit: None,
            physical_time_unit: None,
        }
    }

    /// Defines one inferred stream named `state` containing selected fields.
    pub fn fields<I, S>(fields: I) -> Result<Self, WriterError>
    where
        I: IntoIterator<Item = S>,
        S: Into<String>,
    {
        Self::streams([Stream::fields("state", fields)?])
    }

    /// Defines several explicitly named scientific streams.
    pub fn streams<I>(streams: I) -> Result<Self, WriterError>
    where
        I: IntoIterator<Item = Stream>,
    {
        let streams = streams.into_iter().collect::<Vec<_>>();
        if streams.is_empty() {
            return Err(WriterError::EmptyWriter);
        }
        let mut names = HashSet::with_capacity(streams.len());
        for stream in &streams {
            if !names.insert(stream.name().to_owned()) {
                return Err(WriterError::DuplicateStreamName {
                    stream: stream.name().to_owned(),
                });
            }
        }
        Ok(Self {
            streams: streams.into_boxed_slice(),
            iteration_unit: None,
            physical_time_unit: None,
        })
    }

    /// Attaches an optional scientific unit to the inferred `iteration` axis.
    pub fn with_iteration_unit(mut self, unit: impl Into<String>) -> Result<Self, WriterError> {
        self.iteration_unit = Some(normalize_unit("iteration", unit)?);
        Ok(self)
    }

    /// Attaches an optional scientific unit to the inferred `physical_time` axis.
    pub fn with_physical_time_unit(mut self, unit: impl Into<String>) -> Result<Self, WriterError> {
        self.physical_time_unit = Some(normalize_unit("physical_time", unit)?);
        Ok(self)
    }
}

fn normalize_unit(axis: &'static str, unit: impl Into<String>) -> Result<Box<str>, WriterError> {
    let unit = unit.into();
    let unit = unit.trim();
    if unit.is_empty() {
        return Err(WriterError::EmptyAxisUnit { axis });
    }
    Ok(unit.into())
}

/// A writer definition validated against one immutable state schema.
#[derive(Clone, Debug)]
pub struct WriterDescriptor {
    schema: SystemStateSchema,
    streams: Box<[StreamDescriptor]>,
    iteration_unit: Option<Box<str>>,
    physical_time_unit: Option<Box<str>>,
}

impl WriterDescriptor {
    /// Validates and binds a writer definition to a state schema.
    pub fn bind(writer: Writer, schema: &SystemStateSchema) -> Result<Self, WriterError> {
        let streams = writer
            .streams
            .into_vec()
            .into_iter()
            .map(|stream| stream.bind(schema))
            .collect::<Result<Vec<_>, _>>()?;
        Ok(Self {
            schema: schema.clone(),
            streams: streams.into_boxed_slice(),
            iteration_unit: writer.iteration_unit,
            physical_time_unit: writer.physical_time_unit,
        })
    }

    /// Returns the state schema against which this descriptor was validated.
    pub fn schema(&self) -> &SystemStateSchema {
        &self.schema
    }

    /// Returns validated streams in deterministic definition order.
    pub fn streams(&self) -> &[StreamDescriptor] {
        &self.streams
    }

    /// Returns the optional unit of the inferred `iteration` axis.
    pub fn iteration_unit(&self) -> Option<&str> {
        self.iteration_unit.as_deref()
    }

    /// Returns the optional unit of the inferred `physical_time` axis.
    pub fn physical_time_unit(&self) -> Option<&str> {
        self.physical_time_unit.as_deref()
    }
}
