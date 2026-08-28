//! Deterministic compiled execution-unit discovery and validation.

use std::collections::BTreeMap;

use thiserror::Error;

use crate::config::ResolvedExecutionUnitParameters;
use crate::observation::BoundObservationPlan;
use crate::state::SystemStateSchema;

use super::definition::Task;
use super::result::TaskResult;
use super::unit::ExecutionUnit;

/// One immutable association between a manifest execution-unit key and compiled Rust behavior.
#[derive(Clone, Copy)]
pub struct ExecutionUnitRegistration {
    key: &'static str,
    make_task: fn(
        ResolvedExecutionUnitParameters,
        Box<str>,
        SystemStateSchema,
        BoundObservationPlan,
    ) -> Task,
    preflight: fn(
        &ResolvedExecutionUnitParameters,
        &SystemStateSchema,
    ) -> TaskResult<BoundObservationPlan>,
}

impl ExecutionUnitRegistration {
    /// Creates a registration for `U` without initializing it or reading files.
    ///
    /// This constructor is public only because the downstream expansion of
    /// `#[scientific_workflow::execution_unit("key")]` must name it.
    /// Applications must use that attribute rather than construct registration
    /// metadata directly.
    pub const fn new<U>(key: &'static str) -> Self
    where
        U: ExecutionUnit,
    {
        Self {
            key,
            make_task: Task::for_execution_unit::<U>,
            preflight: preflight_execution_unit::<U>,
        }
    }

    pub(crate) fn make_task(
        self,
        parameters: ResolvedExecutionUnitParameters,
        state: Box<str>,
        schema: SystemStateSchema,
        observation_plan: BoundObservationPlan,
    ) -> Task {
        (self.make_task)(parameters, state, schema, observation_plan)
    }

    pub(crate) fn preflight(
        self,
        parameters: &ResolvedExecutionUnitParameters,
        schema: &SystemStateSchema,
    ) -> TaskResult<BoundObservationPlan> {
        (self.preflight)(parameters, schema)
    }
}

inventory::collect!(ExecutionUnitRegistration);

/// An immutable, key-sorted collection of compiled execution-unit registrations.
#[derive(Clone)]
pub(crate) struct ExecutionUnitCatalog {
    registrations: BTreeMap<&'static str, ExecutionUnitRegistration>,
}

impl ExecutionUnitCatalog {
    /// Discovers every linked `#[execution_unit]` declaration, then validates and sorts it.
    pub(crate) fn discovered() -> Result<Self, ExecutionUnitCatalogError> {
        Self::from_registrations(
            inventory::iter::<ExecutionUnitRegistration>
                .into_iter()
                .copied(),
        )
    }

    /// Applies the discovery validation path to one registration iterator.
    pub(crate) fn from_registrations(
        registrations: impl IntoIterator<Item = ExecutionUnitRegistration>,
    ) -> Result<Self, ExecutionUnitCatalogError> {
        let mut by_key = BTreeMap::new();
        for registration in registrations {
            validate_key(registration.key)?;
            if by_key.insert(registration.key, registration).is_some() {
                return Err(ExecutionUnitCatalogError::DuplicateKey {
                    key: registration.key.to_owned(),
                });
            }
        }
        Ok(Self {
            registrations: by_key,
        })
    }

    pub(crate) fn get(&self, key: &str) -> Option<ExecutionUnitRegistration> {
        self.registrations.get(key).copied()
    }
}

impl std::fmt::Debug for ExecutionUnitCatalog {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("ExecutionUnitCatalog")
            .field("keys", &self.registrations.keys())
            .finish()
    }
}

/// A failure while validating compiled execution-unit declarations.
#[derive(Debug, Error)]
#[non_exhaustive]
pub(crate) enum ExecutionUnitCatalogError {
    /// A registration key is empty or contains surrounding whitespace.
    #[error(
        "execution-unit registration key `{key}` must be nonempty and contain no surrounding whitespace"
    )]
    InvalidKey {
        /// Rejected compiled key.
        key: String,
    },
    /// Two compiled execution units use the same stable key.
    #[error("execution-unit registration key `{key}` is declared more than once")]
    DuplicateKey {
        /// Repeated compiled key.
        key: String,
    },
}

fn validate_key(key: &str) -> Result<(), ExecutionUnitCatalogError> {
    if key.is_empty() || key.trim() != key {
        Err(ExecutionUnitCatalogError::InvalidKey {
            key: key.to_owned(),
        })
    } else {
        Ok(())
    }
}

fn preflight_execution_unit<U>(
    parameters: &ResolvedExecutionUnitParameters,
    schema: &SystemStateSchema,
) -> TaskResult<BoundObservationPlan>
where
    U: ExecutionUnit,
{
    let constants: U::Constants = parameters.decode()?;
    let plan = U::preflight(&constants, schema)?;
    Ok(BoundObservationPlan::bind(plan, schema)?)
}
