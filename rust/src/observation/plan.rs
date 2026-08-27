//! Observation plans and schema-bound plans.

use std::collections::HashSet;

use crate::state::advanced::SystemStateSchema;

use super::error::ObservationError;
use super::stream::{BoundObservationStream, ObservationStream};

/// An immutable application definition of scientific observation.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ObservationPlan {
    streams: Box<[ObservationStream]>,
    iteration_unit: Option<Box<str>>,
    physical_time_unit: Option<Box<str>>,
}

impl ObservationPlan {
    /// Defines one inferred stream named `state` containing every state field.
    pub fn all_fields() -> Self {
        Self {
            streams: vec![
                ObservationStream::all_fields("state").expect("the inferred name is valid"),
            ]
            .into_boxed_slice(),
            iteration_unit: None,
            physical_time_unit: None,
        }
    }

    /// Defines one inferred stream named `state` containing selected fields.
    pub fn fields<I, S>(fields: I) -> Result<Self, ObservationError>
    where
        I: IntoIterator<Item = S>,
        S: Into<String>,
    {
        Self::streams([ObservationStream::fields("state", fields)?])
    }

    /// Defines several explicitly named scientific streams.
    pub fn streams<I>(streams: I) -> Result<Self, ObservationError>
    where
        I: IntoIterator<Item = ObservationStream>,
    {
        let streams = streams.into_iter().collect::<Vec<_>>();
        if streams.is_empty() {
            return Err(ObservationError::EmptyPlan);
        }
        let mut names = HashSet::with_capacity(streams.len());
        for stream in &streams {
            if !names.insert(stream.name().to_owned()) {
                return Err(ObservationError::DuplicateStreamName {
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
    pub fn with_iteration_unit(
        mut self,
        unit: impl Into<String>,
    ) -> Result<Self, ObservationError> {
        self.iteration_unit = Some(normalize_unit("iteration", unit)?);
        Ok(self)
    }

    /// Attaches an optional scientific unit to the inferred `physical_time` axis.
    pub fn with_physical_time_unit(
        mut self,
        unit: impl Into<String>,
    ) -> Result<Self, ObservationError> {
        self.physical_time_unit = Some(normalize_unit("physical_time", unit)?);
        Ok(self)
    }
}

fn normalize_unit(
    axis: &'static str,
    unit: impl Into<String>,
) -> Result<Box<str>, ObservationError> {
    let unit = unit.into();
    let unit = unit.trim();
    if unit.is_empty() {
        return Err(ObservationError::EmptyAxisUnit { axis });
    }
    Ok(unit.into())
}

/// An observation plan validated against one immutable state schema.
#[derive(Clone, Debug)]
pub(crate) struct BoundObservationPlan {
    schema: SystemStateSchema,
    streams: Box<[BoundObservationStream]>,
    iteration_unit: Option<Box<str>>,
    physical_time_unit: Option<Box<str>>,
}

impl BoundObservationPlan {
    /// Validates and binds an observation plan to a state schema.
    pub(crate) fn bind(
        plan: ObservationPlan,
        schema: &SystemStateSchema,
    ) -> Result<Self, ObservationError> {
        let streams = plan
            .streams
            .into_vec()
            .into_iter()
            .map(|stream| stream.bind(schema))
            .collect::<Result<Vec<_>, _>>()?;
        Ok(Self {
            schema: schema.clone(),
            streams: streams.into_boxed_slice(),
            iteration_unit: plan.iteration_unit,
            physical_time_unit: plan.physical_time_unit,
        })
    }

    /// Returns the state schema against which this descriptor was validated.
    pub(crate) fn schema(&self) -> &SystemStateSchema {
        &self.schema
    }

    /// Returns validated streams in deterministic definition order.
    pub(crate) fn streams(&self) -> &[BoundObservationStream] {
        &self.streams
    }

    /// Returns the optional unit of the inferred `iteration` axis.
    pub(crate) fn iteration_unit(&self) -> Option<&str> {
        self.iteration_unit.as_deref()
    }

    /// Returns the optional unit of the inferred `physical_time` axis.
    pub(crate) fn physical_time_unit(&self) -> Option<&str> {
        self.physical_time_unit.as_deref()
    }
}
