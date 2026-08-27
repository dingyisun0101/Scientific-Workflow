//! Deterministic compiled-model discovery and explicit catalog injection.

use std::collections::BTreeMap;

use thiserror::Error;

use crate::config::advanced::ResolvedTaskInput;
use crate::observation::advanced::BoundObservationPlan;
use crate::state::advanced::SystemStateSchema;

use super::definition::Task;
use super::model::ScientificModel;
use super::result::TaskResult;

/// One immutable association between a manifest model key and compiled Rust behavior.
#[derive(Clone, Copy)]
pub struct ModelRegistration {
    key: &'static str,
    make_task: fn(ResolvedTaskInput, BoundObservationPlan) -> Task,
    preflight: fn(&ResolvedTaskInput, &SystemStateSchema) -> TaskResult<BoundObservationPlan>,
}

impl ModelRegistration {
    /// Creates a registration for `M` without initializing a model or reading files.
    ///
    /// Ordinary applications acquire registrations through
    /// `#[scientific_workflow::model("key")]`. This constructor supports
    /// explicit advanced catalogs in tests and embedded runtimes.
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

    /// Returns the stable key used in `study.json`.
    pub const fn key(self) -> &'static str {
        self.key
    }

    pub(crate) fn make_task(
        self,
        input: ResolvedTaskInput,
        observation_plan: BoundObservationPlan,
    ) -> Task {
        (self.make_task)(input, observation_plan)
    }

    pub(crate) fn preflight(
        self,
        input: &ResolvedTaskInput,
        schema: &SystemStateSchema,
    ) -> TaskResult<BoundObservationPlan> {
        (self.preflight)(input, schema)
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

    /// Builds an explicit deterministic catalog for embedding and tests.
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
    input: &ResolvedTaskInput,
    schema: &SystemStateSchema,
) -> TaskResult<BoundObservationPlan>
where
    M: ScientificModel,
{
    let constants: M::Constants = input.decode()?;
    let plan = M::observation_plan(&constants)?;
    Ok(BoundObservationPlan::bind(plan, schema)?)
}
