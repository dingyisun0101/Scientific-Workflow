//! Application-owned scientific execution contract.

use std::collections::{BTreeMap, BTreeSet};
use std::sync::{Mutex, MutexGuard};

use serde::de::DeserializeOwned;
use serde_json::{Map, Value};
use sha2::{Digest, Sha256};
use thiserror::Error;

use crate::observation::advanced::ObservationPlan;
use crate::state::advanced::{SystemState, SystemStateSchema};

use super::result::TaskResult;

const SEED_DERIVATION_ALGORITHM: &str = "scientific-workflow.seed.v1";

/// Immutable execution facts and optional deterministic seed derivation.
///
/// Workflow creates one context for each execution-unit initialization. A
/// deterministic unit may ignore it. A stochastic unit requests only the
/// named seeds it actually needs; Workflow derives them without counters or
/// request-order dependence and records successful requests with the affected
/// model's output metadata.
pub struct InitializationContext {
    master_seed: Option<u64>,
    replicate_ordinal: u64,
    task_identity: Box<str>,
    execution_unit_key: Box<str>,
    requests: Mutex<BTreeMap<SeedRequest, u64>>,
}

impl InitializationContext {
    pub(crate) fn new(
        master_seed: Option<u64>,
        replicate_ordinal: u64,
        task_identity: &str,
        execution_unit_key: &str,
    ) -> Self {
        Self {
            master_seed,
            replicate_ordinal,
            task_identity: task_identity.into(),
            execution_unit_key: execution_unit_key.into(),
            requests: Mutex::new(BTreeMap::new()),
        }
    }

    /// Returns whether `wf_configs/study.json` supplied a master seed.
    pub const fn has_master_seed(&self) -> bool {
        self.master_seed.is_some()
    }

    /// Derives a seed shared by all models in this execution unit.
    ///
    /// `purpose` is a stable, nonempty semantic name such as `"pairing"`.
    /// Repeating the same request returns the same seed and metadata entry.
    pub fn shared_seed(&self, purpose: &str) -> Result<u64, SeedError> {
        self.seed(SeedScope::Shared, purpose)
    }

    /// Derives a seed belonging to one model exposed by this execution unit.
    ///
    /// `model_identity` must exactly match that model's [`ModelView`] identity;
    /// Workflow rejects an initialization that requested a seed for an unknown
    /// model. `purpose` is a stable, nonempty semantic name such as
    /// `"initialization"` or `"replacement"`.
    pub fn model_seed(&self, model_identity: &str, purpose: &str) -> Result<u64, SeedError> {
        validate_name(model_identity, "model identity")?;
        self.seed(SeedScope::Model(model_identity.into()), purpose)
    }

    fn seed(&self, scope: SeedScope, purpose: &str) -> Result<u64, SeedError> {
        validate_name(purpose, "purpose")?;
        let master_seed = self.master_seed.ok_or(SeedError::MissingMasterSeed)?;
        let request = SeedRequest {
            scope,
            purpose: purpose.into(),
        };
        if let Some(seed) = self.request_ledger().get(&request) {
            return Ok(*seed);
        }

        let mut digest = Sha256::new();
        hash_part(&mut digest, SEED_DERIVATION_ALGORITHM.as_bytes());
        digest.update(master_seed.to_le_bytes());
        digest.update(self.replicate_ordinal.to_le_bytes());
        hash_part(&mut digest, self.task_identity.as_bytes());
        hash_part(&mut digest, self.execution_unit_key.as_bytes());
        match &request.scope {
            SeedScope::Shared => digest.update([0]),
            SeedScope::Model(identity) => {
                digest.update([1]);
                hash_part(&mut digest, identity.as_bytes());
            }
        }
        hash_part(&mut digest, request.purpose.as_bytes());
        let seed = u64::from_le_bytes(digest.finalize()[..8].try_into().expect("SHA-256 prefix"));
        self.request_ledger().insert(request, seed);
        Ok(seed)
    }

    pub(crate) fn validate_model_identities<'a>(
        &self,
        identities: impl IntoIterator<Item = &'a str>,
    ) -> Result<(), SeedError> {
        let identities = identities.into_iter().collect::<BTreeSet<_>>();
        for request in self.request_ledger().keys() {
            if let SeedScope::Model(identity) = &request.scope
                && !identities.contains(identity.as_ref())
            {
                return Err(SeedError::UnknownModelIdentity {
                    identity: identity.to_string(),
                });
            }
        }
        Ok(())
    }

    pub(crate) fn metadata_for_model(&self, model_identity: &str) -> Option<Value> {
        let requests = self
            .request_ledger()
            .iter()
            .filter_map(|(request, seed)| match &request.scope {
                SeedScope::Shared => Some(Value::Object(Map::from_iter([
                    ("scope".to_owned(), "shared".into()),
                    ("purpose".to_owned(), request.purpose.to_string().into()),
                    ("seed".to_owned(), (*seed).into()),
                ]))),
                SeedScope::Model(identity) if identity.as_ref() == model_identity => {
                    Some(Value::Object(Map::from_iter([
                        ("scope".to_owned(), "model".into()),
                        ("model_identity".to_owned(), identity.to_string().into()),
                        ("purpose".to_owned(), request.purpose.to_string().into()),
                        ("seed".to_owned(), (*seed).into()),
                    ])))
                }
                SeedScope::Model(_) => None,
            })
            .collect::<Vec<_>>();
        (!requests.is_empty()).then(|| {
            Value::Object(Map::from_iter([
                ("algorithm".to_owned(), SEED_DERIVATION_ALGORITHM.into()),
                (
                    "master_seed".to_owned(),
                    self.master_seed
                        .expect("a recorded request has a seed")
                        .into(),
                ),
                ("requests".to_owned(), requests.into()),
            ]))
        })
    }

    fn request_ledger(&self) -> MutexGuard<'_, BTreeMap<SeedRequest, u64>> {
        self.requests
            .lock()
            .unwrap_or_else(|error| error.into_inner())
    }
}

fn hash_part(digest: &mut Sha256, bytes: &[u8]) {
    digest.update((bytes.len() as u64).to_le_bytes());
    digest.update(bytes);
}

fn validate_name(value: &str, field: &'static str) -> Result<(), SeedError> {
    if value.is_empty() || value.trim() != value {
        return Err(SeedError::InvalidName {
            field,
            value: value.to_owned(),
        });
    }
    Ok(())
}

#[derive(Clone, Debug, Eq, Ord, PartialEq, PartialOrd)]
struct SeedRequest {
    scope: SeedScope,
    purpose: Box<str>,
}

#[derive(Clone, Debug, Eq, Ord, PartialEq, PartialOrd)]
enum SeedScope {
    Shared,
    Model(Box<str>),
}

/// Failure to make or validate a deterministic initialization seed request.
#[derive(Debug, Error)]
#[non_exhaustive]
pub enum SeedError {
    /// The execution unit requested a seed but the study has none.
    #[error("this execution unit requires `seed` in `wf_configs/study.json`")]
    MissingMasterSeed,
    /// A stable seed name was empty or had surrounding whitespace.
    #[error("invalid seed {field} `{value}`: it must be nonempty with no surrounding whitespace")]
    InvalidName {
        /// The invalid semantic field.
        field: &'static str,
        /// The rejected value.
        value: String,
    },
    /// A model seed was requested for an identity the unit did not expose.
    #[error("seed requested for unknown model identity `{identity}`")]
    UnknownModelIdentity {
        /// The requested identity absent from the execution unit.
        identity: String,
    },
}

/// A borrowed view of one independently stateful model inside an execution unit.
///
/// The identity, state owner, and schema allocation must remain stable at the
/// same model index for the complete execution. `complete` and
/// `target_iteration` are declarations inspected by Workflow; constructing a
/// view has no side effects.
#[derive(Clone, Copy, Debug)]
pub struct ModelView<'a> {
    identity: &'a str,
    state: &'a SystemState,
    complete: bool,
    target_iteration: Option<u64>,
}

impl<'a> ModelView<'a> {
    /// Describes one model owned by an [`ExecutionUnit`].
    ///
    /// `identity` must be nonempty, contain no surrounding whitespace, and be
    /// unique within the unit. `state` must remain at the same address and use
    /// the schema supplied to [`ExecutionUnit::initialize`]. A target, when
    /// present, must not precede the state's current iteration.
    pub const fn new(
        identity: &'a str,
        state: &'a SystemState,
        complete: bool,
        target_iteration: Option<u64>,
    ) -> Self {
        Self {
            identity,
            state,
            complete,
            target_iteration,
        }
    }

    /// Returns the stable identity of this model within its execution unit.
    pub const fn identity(self) -> &'a str {
        self.identity
    }

    /// Borrows this model's directly owned canonical state.
    pub const fn state(self) -> &'a SystemState {
        self.state
    }

    /// Returns whether this model requires no further transition.
    pub const fn is_complete(self) -> bool {
        self.complete
    }

    /// Returns this model's optional expected final iteration.
    pub const fn target_iteration(self) -> Option<u64> {
        self.target_iteration
    }
}

/// One schedulable scientific execution containing one or more models.
///
/// Workflow manages every implementation through the same lifecycle and does
/// not distinguish a standalone model from a coordinated ensemble. A normal
/// model returns one [`ModelView`]; an ensemble returns one view per member and
/// keeps all internal parallelism, shared inputs, and synchronization private.
/// Each exposed model owns a distinct [`SystemState`].
///
/// Model count, index order, identities, state owners, and schema allocations
/// must remain stable after initialization. One successful [`Self::step`] must
/// strictly advance at least one incomplete model and must never advance a
/// model that was already complete. Other incomplete models may wait during a
/// coordinated step, which permits synchronized ensembles and restored members
/// at different iterations.
pub trait ExecutionUnit: Send + Sized + 'static {
    /// One complete set of constants supplied by Config.
    type Constants: DeserializeOwned + 'static;

    /// Defines the observations recorded independently for every model.
    ///
    /// All members of one execution unit use the state schema selected by the
    /// task and this common observation plan. The default records every field
    /// at every iteration. This preflight operation must have no external side
    /// effects.
    fn observation_plan(_constants: &Self::Constants) -> TaskResult<ObservationPlan> {
        Ok(ObservationPlan::all_fields())
    }

    /// Builds a fully initialized standalone model or ensemble.
    ///
    /// Every state subsequently exposed through [`Self::model`] must have been
    /// created from this exact schema allocation.
    fn initialize(
        constants: Self::Constants,
        schema: &SystemStateSchema,
        context: &InitializationContext,
    ) -> TaskResult<Self>;

    /// Returns the stable positive number of independently stateful models.
    fn model_count(&self) -> usize;

    /// Borrows one model by stable zero-based index.
    ///
    /// The method must be side-effect free. Workflow calls it repeatedly before
    /// and after coordinated steps. An index below [`Self::model_count`] must
    /// always return `Some`; all other indices must return `None`.
    fn model(&self, index: usize) -> Option<ModelView<'_>>;

    /// Performs one coordinated scientific transition.
    ///
    /// A standalone model advances itself. An ensemble may advance members in
    /// parallel or share generated inputs, but it must return only after the
    /// complete logical step is visible through [`Self::model`].
    fn step(&mut self) -> TaskResult;
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn named_seed_derivation_is_stable_and_request_order_independent() {
        let first = InitializationContext::new(Some(99), 3, "phase/task", "ensemble");
        let first_model = first.model_seed("alpha", "initialization").unwrap();
        let first_shared = first.shared_seed("pairing").unwrap();
        assert_eq!(first_model, 16_741_472_295_366_384_357);
        assert_eq!(first_shared, 1_632_703_961_247_452_931);

        let second = InitializationContext::new(Some(99), 3, "phase/task", "ensemble");
        assert_eq!(second.shared_seed("pairing").unwrap(), first_shared);
        assert_eq!(
            second.model_seed("alpha", "initialization").unwrap(),
            first_model
        );
        assert_ne!(
            second.model_seed("beta", "initialization").unwrap(),
            first_model
        );

        let next_replicate = InitializationContext::new(Some(99), 4, "phase/task", "ensemble");
        assert_ne!(next_replicate.shared_seed("pairing").unwrap(), first_shared);
    }

    #[test]
    fn deterministic_units_need_no_master_seed_but_requests_do() {
        let context = InitializationContext::new(None, 0, "task", "unit");
        assert!(!context.has_master_seed());
        assert!(matches!(
            context.shared_seed("pairing"),
            Err(SeedError::MissingMasterSeed)
        ));
        assert!(context.metadata_for_model("model").is_none());
    }
}
