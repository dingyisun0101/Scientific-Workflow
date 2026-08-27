//! Application-owned scientific model contract.

use serde::de::DeserializeOwned;

use crate::observation::advanced::ObservationPlan;
use crate::state::advanced::{SystemState, SystemStateSchema};

use super::result::TaskResult;

/// A stateful scientific workload initialized from config-supplied constants.
///
/// # Direct state ownership
///
/// The implementing model **must directly own** the [`SystemState`] returned
/// by [`ScientificModel::state`] for its entire execution. Rust traits cannot
/// express field-level structural ownership, so this requirement is semantic;
/// the task runtime enforces the observable parts by requiring a stable state
/// address and the exact schema allocation supplied to [`Self::initialize`].
/// Returning a temporary, swapping the state owner, or changing schemas is a
/// contract violation.
///
/// The `#[scientific_workflow::model("key")]` attribute only registers the
/// implementation under a manifest key. It does not generate, wrap, proxy, or
/// provide access to model state. The model defines an ordinary `SystemState`
/// field, initializes its concrete payloads, and returns a direct borrow of
/// that field from [`Self::state`]. Coupled fields remain ordinary typed tuple
/// borrows through [`SystemState::borrow_payloads`] and
/// [`SystemState::borrow_payloads_mut`]; no field-access macro is involved.
///
/// A successful [`Self::step`] represents exactly one scientifically
/// observable transition and must strictly advance the state's iteration.
/// Workflow observes the initial state, every successful step, and the final
/// state automatically. Implementations do not report progress or invoke the
/// observation plan themselves.
pub trait ScientificModel: Send + Sized + 'static {
    /// One complete set of model constants supplied by config.
    type Constants: DeserializeOwned + Send + Sync + 'static;

    /// Defines the scientific observations recorded for this model.
    ///
    /// The default records every declared state field at every iteration.
    /// Implementations may derive stream selection and sampling cadence from
    /// model constants. This function is called once during effect-free Study
    /// preflight; Study binds and retains that exact plan for execution. It
    /// must not perform external side effects.
    fn observation_plan(_constants: &Self::Constants) -> TaskResult<ObservationPlan> {
        Ok(ObservationPlan::all_fields())
    }

    /// Builds a fully initialized model from resolved constants and the
    /// selected, centrally loaded state schema.
    ///
    /// The returned model must already contain a usable, fully populated state
    /// created from this exact `schema` allocation. No state observation occurs
    /// until this method succeeds.
    fn initialize(constants: Self::Constants, schema: &SystemStateSchema) -> TaskResult<Self>;

    /// Borrows the model's directly owned canonical state.
    ///
    /// This method must be side-effect free and must return the same state
    /// owner and schema allocation throughout execution.
    fn state(&self) -> &SystemState;

    /// Reports whether no further scientific transition is required.
    ///
    /// This method must be side-effect free. Once it returns `true` during one
    /// execution, Workflow performs the final observation without calling
    /// [`Self::step`] again.
    fn is_complete(&self) -> bool;

    /// Performs exactly one scientifically observable transition.
    ///
    /// Success must strictly increase [`SystemState::time`]'s iteration while
    /// preserving the state owner and schema. Failure must not claim a
    /// completed transition: Workflow emits neither a successful-step snapshot
    /// nor a successful-step observation for an error result.
    fn step(&mut self) -> TaskResult;

    /// Optionally reports the expected final iteration for inferred progress.
    ///
    /// The default is unknown. A reported target must be at least the current
    /// iteration. Once present during an execution, it must never decrease or
    /// disappear. This method must be side-effect free.
    fn target_iteration(&self) -> Option<u64> {
        None
    }
}
