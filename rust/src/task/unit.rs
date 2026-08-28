//! Application-owned scientific execution contract.

use serde::de::DeserializeOwned;

use crate::observation::advanced::ObservationPlan;
use crate::state::advanced::{SystemState, SystemStateSchema};

use super::result::TaskResult;

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
    fn initialize(constants: Self::Constants, schema: &SystemStateSchema) -> TaskResult<Self>;

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
