//! Deterministic compiled-model discovery and validation.

use std::collections::BTreeMap;

use thiserror::Error;

use crate::config::advanced::ResolvedModelParameters;
use crate::observation::advanced::BoundObservationPlan;
use crate::state::advanced::SystemStateSchema;

use super::definition::Task;
use super::model::ScientificModel;
use super::result::TaskResult;

/// One immutable association between a manifest model key and compiled Rust behavior.
#[derive(Clone, Copy)]
pub struct ModelRegistration {
    key: &'static str,
    make_task:
        fn(ResolvedModelParameters, Box<str>, SystemStateSchema, BoundObservationPlan) -> Task,
    preflight: fn(&ResolvedModelParameters, &SystemStateSchema) -> TaskResult<BoundObservationPlan>,
}

impl ModelRegistration {
    /// Creates a registration for `M` without initializing a model or reading files.
    ///
    /// This constructor is public only because the downstream expansion of
    /// `#[scientific_workflow::model("key")]` must name it. Applications must
    /// use that attribute rather than construct registration metadata directly.
    pub const fn new<M>(key: &'static str) -> Self
    where
        M: ScientificModel,
    {
        Self {
            key,
            make_task: Task::for_model::<M>,
            preflight: preflight_model::<M>,
        }
    }

    pub(crate) fn make_task(
        self,
        parameters: ResolvedModelParameters,
        state: Box<str>,
        schema: SystemStateSchema,
        observation_plan: BoundObservationPlan,
    ) -> Task {
        (self.make_task)(parameters, state, schema, observation_plan)
    }

    pub(crate) fn preflight(
        self,
        parameters: &ResolvedModelParameters,
        schema: &SystemStateSchema,
    ) -> TaskResult<BoundObservationPlan> {
        (self.preflight)(parameters, schema)
    }
}

inventory::collect!(ModelRegistration);

/// An immutable, key-sorted collection of compiled model registrations.
#[derive(Clone)]
pub(crate) struct ModelCatalog {
    registrations: BTreeMap<&'static str, ModelRegistration>,
}

impl ModelCatalog {
    /// Discovers every linked `#[model]` declaration, then validates and sorts it.
    pub(crate) fn discovered() -> Result<Self, ModelCatalogError> {
        Self::from_registrations(inventory::iter::<ModelRegistration>.into_iter().copied())
    }

    /// Applies the discovery validation path to one registration iterator.
    pub(crate) fn from_registrations(
        registrations: impl IntoIterator<Item = ModelRegistration>,
    ) -> Result<Self, ModelCatalogError> {
        let mut by_key = BTreeMap::new();
        for registration in registrations {
            validate_key(registration.key)?;
            if by_key.insert(registration.key, registration).is_some() {
                return Err(ModelCatalogError::DuplicateKey {
                    key: registration.key.to_owned(),
                });
            }
        }
        Ok(Self {
            registrations: by_key,
        })
    }

    pub(crate) fn get(&self, key: &str) -> Option<ModelRegistration> {
        self.registrations.get(key).copied()
    }
}

impl std::fmt::Debug for ModelCatalog {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("ModelCatalog")
            .field("keys", &self.registrations.keys())
            .finish()
    }
}

/// A failure while validating compiled model declarations.
#[derive(Debug, Error)]
#[non_exhaustive]
pub(crate) enum ModelCatalogError {
    /// A registration key is empty or contains surrounding whitespace.
    #[error(
        "model registration key `{key}` must be nonempty and contain no surrounding whitespace"
    )]
    InvalidKey {
        /// Rejected compiled key.
        key: String,
    },
    /// Two compiled models use the same stable key.
    #[error("model registration key `{key}` is declared more than once")]
    DuplicateKey {
        /// Repeated compiled key.
        key: String,
    },
}

fn validate_key(key: &str) -> Result<(), ModelCatalogError> {
    if key.is_empty() || key.trim() != key {
        Err(ModelCatalogError::InvalidKey {
            key: key.to_owned(),
        })
    } else {
        Ok(())
    }
}

fn preflight_model<M>(
    parameters: &ResolvedModelParameters,
    schema: &SystemStateSchema,
) -> TaskResult<BoundObservationPlan>
where
    M: ScientificModel,
{
    let constants: M::Constants = parameters.decode()?;
    let plan = M::observation_plan(&constants)?;
    Ok(BoundObservationPlan::bind(plan, schema)?)
}
