//! Application-owned scientific execution contract.

use std::collections::{BTreeMap, BTreeSet};
use std::sync::{Mutex, MutexGuard};

use serde::de::DeserializeOwned;
use serde_json::{Map, Value};
use sha2::{Digest, Sha256};
use thiserror::Error;

use crate::observation::ObservationPlan;
use crate::state::{SystemState, SystemStateSchema};

use super::result::TaskResult;

const SEED_DERIVATION_ALGORITHM: &str = "scientific-workflow.seed.v1";

/// Immutable execution facts and optional deterministic seed derivation.
///
/// Workflow creates one context for each execution-unit initialization. A
/// deterministic unit may ignore it. A stochastic unit requests only the
/// named seeds it actually needs; Workflow derives them without counters or
/// request-order dependence and records successful requests with the affected
/// member's output metadata.
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

    /// Derives a seed shared by all members in this execution unit.
    ///
    /// `purpose` is a stable, nonempty semantic name such as `"pairing"`.
    /// Repeating the same request returns the same seed and metadata entry.
    pub fn shared_seed(&self, purpose: &str) -> Result<u64, SeedError> {
        self.seed(SeedScope::Shared, purpose)
    }

    /// Derives a seed belonging to one member exposed by this execution unit.
    ///
    /// `member_identity` must exactly match that member's [`MemberView`] identity;
    /// Workflow rejects an initialization that requested a seed for an unknown
    /// member. `purpose` is a stable, nonempty semantic name such as
    /// `"initialization"` or `"replacement"`.
    pub fn member_seed(&self, member_identity: &str, purpose: &str) -> Result<u64, SeedError> {
        validate_name(member_identity, "member identity")?;
        self.seed(SeedScope::Member(member_identity.into()), purpose)
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
            SeedScope::Member(identity) => {
                digest.update([1]);
                hash_part(&mut digest, identity.as_bytes());
            }
        }
        hash_part(&mut digest, request.purpose.as_bytes());
        let seed = u64::from_le_bytes(digest.finalize()[..8].try_into().expect("SHA-256 prefix"));
        self.request_ledger().insert(request, seed);
        Ok(seed)
    }

    pub(crate) fn validate_member_identities<'a>(
        &self,
        identities: impl IntoIterator<Item = &'a str>,
    ) -> Result<(), SeedError> {
        let identities = identities.into_iter().collect::<BTreeSet<_>>();
        for request in self.request_ledger().keys() {
            if let SeedScope::Member(identity) = &request.scope
                && !identities.contains(identity.as_ref())
            {
                return Err(SeedError::UnknownMemberIdentity {
                    identity: identity.to_string(),
                });
            }
        }
        Ok(())
    }

    pub(crate) fn metadata_for_member(&self, member_identity: &str) -> Option<Value> {
        let requests = self
            .request_ledger()
            .iter()
            .filter_map(|(request, seed)| match &request.scope {
                SeedScope::Shared => Some(Value::Object(Map::from_iter([
                    ("scope".to_owned(), "shared".into()),
                    ("purpose".to_owned(), request.purpose.to_string().into()),
                    ("seed".to_owned(), (*seed).into()),
                ]))),
                SeedScope::Member(identity) if identity.as_ref() == member_identity => {
                    Some(Value::Object(Map::from_iter([
                        ("scope".to_owned(), "member".into()),
                        ("member_identity".to_owned(), identity.to_string().into()),
                        ("purpose".to_owned(), request.purpose.to_string().into()),
                        ("seed".to_owned(), (*seed).into()),
                    ])))
                }
                SeedScope::Member(_) => None,
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
    Member(Box<str>),
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
    /// A member seed was requested for an identity the unit did not expose.
    #[error("seed requested for unknown member identity `{identity}`")]
    UnknownMemberIdentity {
        /// The requested identity absent from the execution unit.
        identity: String,
    },
}

/// The optional completion declaration for one execution-unit member.
///
/// This borrowed value distinguishes an incomplete member from a completed
/// member with or without a structured reason. The execution unit retains the
/// reason object; Workflow copies it only when the member first completes.
#[derive(Clone, Copy, Debug)]
pub struct MemberCompletion<'a> {
    reason: Option<&'a Map<String, Value>>,
}

impl<'a> MemberCompletion<'a> {
    /// Declares completion without a structured reason.
    pub const fn without_reason() -> Self {
        Self { reason: None }
    }

    /// Declares completion with a borrowed structured JSON reason.
    pub const fn with_reason(reason: &'a Map<String, Value>) -> Self {
        Self {
            reason: Some(reason),
        }
    }

    /// Returns the structured reason, when the execution unit supplied one.
    pub const fn reason(self) -> Option<&'a Map<String, Value>> {
        self.reason
    }
}

/// A borrowed view of one independently stateful member inside an execution unit.
///
/// The identity, state owner, and schema allocation must remain stable at the
/// same member index for the complete execution. `completion` and
/// `target_iteration` are declarations inspected by Workflow; constructing a
/// view has no side effects.
#[derive(Clone, Copy, Debug)]
pub struct MemberView<'a> {
    identity: &'a str,
    state: &'a SystemState,
    completion: Option<MemberCompletion<'a>>,
    target_iteration: Option<u64>,
}

impl<'a> MemberView<'a> {
    /// Describes one member owned by an [`ExecutionUnit`].
    ///
    /// `identity` must be nonempty, contain no surrounding whitespace, and be
    /// unique within the unit. `state` must remain at the same address and use
    /// the schema supplied to [`ExecutionUnit::initialize`]. A target, when
    /// present, must not precede the state's current iteration.
    pub const fn new(
        identity: &'a str,
        state: &'a SystemState,
        completion: Option<MemberCompletion<'a>>,
        target_iteration: Option<u64>,
    ) -> Self {
        Self {
            identity,
            state,
            completion,
            target_iteration,
        }
    }

    /// Returns the stable identity of this member within its execution unit.
    pub const fn identity(self) -> &'a str {
        self.identity
    }

    /// Borrows this member's directly owned canonical state.
    pub const fn state(self) -> &'a SystemState {
        self.state
    }

    /// Returns completion details, or `None` while the member is incomplete.
    pub const fn completion(self) -> Option<MemberCompletion<'a>> {
        self.completion
    }

    /// Returns this member's optional expected final iteration.
    pub const fn target_iteration(self) -> Option<u64> {
        self.target_iteration
    }
}

/// One schedulable scientific execution containing one or more members.
///
/// Workflow manages every implementation through the same lifecycle and does
/// not distinguish a standalone unit from a coordinated ensemble. A normal
/// unit returns one [`MemberView`]; an ensemble returns one view per member and
/// keeps all internal parallelism, shared inputs, and synchronization private.
/// Each exposed member owns a distinct [`SystemState`].
///
/// Member count, index order, identities, state owners, and schema allocations
/// must remain stable after initialization. One successful [`Self::step`] must
/// strictly advance at least one incomplete member and must never advance a
/// member that was already complete. Other incomplete members may wait during a
/// coordinated step, which permits synchronized ensembles and restored members
/// at different iterations.
pub trait ExecutionUnit: Send + Sized + 'static {
    /// One complete set of constants supplied by Config.
    type Constants: DeserializeOwned + 'static;

    /// Validates unit-owned configuration and defines per-member observations.
    ///
    /// All members of one execution unit use the state schema selected by the
    /// task and this common observation plan. The default records every field
    /// at every iteration without additional validation. An override owns all
    /// domain validation that can be performed before initialization; Study
    /// trusts a successful result. This preflight operation must not create
    /// output, initialize the unit, or mutate external state.
    fn preflight(
        _constants: &Self::Constants,
        _schema: &SystemStateSchema,
    ) -> TaskResult<ObservationPlan> {
        Ok(ObservationPlan::all_fields())
    }

    /// Builds a fully initialized standalone unit or ensemble.
    ///
    /// Every state subsequently exposed through [`Self::member`] must have been
    /// created from this exact schema allocation.
    fn initialize(
        constants: Self::Constants,
        schema: &SystemStateSchema,
        context: &InitializationContext,
    ) -> TaskResult<Self>;

    /// Returns the stable positive number of independently stateful members.
    fn member_count(&self) -> usize;

    /// Borrows one member by stable zero-based index.
    ///
    /// The method must be side-effect free. Workflow calls it repeatedly before
    /// and after coordinated steps. An index below [`Self::member_count`] must
    /// always return `Some`; all other indices must return `None`.
    fn member(&self, index: usize) -> Option<MemberView<'_>>;

    /// Performs one coordinated scientific transition.
    ///
    /// A standalone unit advances itself. An ensemble may advance members in
    /// parallel or share generated inputs, but it must return only after the
    /// complete logical step is visible through [`Self::member`].
    fn step(&mut self) -> TaskResult;
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn named_seed_derivation_is_stable_and_request_order_independent() {
        let first = InitializationContext::new(Some(99), 3, "phase/task", "ensemble");
        let first_member = first.member_seed("alpha", "initialization").unwrap();
        let first_shared = first.shared_seed("pairing").unwrap();
        assert_eq!(first_member, 16_741_472_295_366_384_357);
        assert_eq!(first_shared, 1_632_703_961_247_452_931);

        let second = InitializationContext::new(Some(99), 3, "phase/task", "ensemble");
        assert_eq!(second.shared_seed("pairing").unwrap(), first_shared);
        assert_eq!(
            second.member_seed("alpha", "initialization").unwrap(),
            first_member
        );
        assert_ne!(
            second.member_seed("beta", "initialization").unwrap(),
            first_member
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
        assert!(context.metadata_for_member("member").is_none());
    }
}
