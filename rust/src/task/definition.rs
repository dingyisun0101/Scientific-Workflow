//! Opaque advanced task definitions created from registered models.

use std::fmt;
use std::sync::Arc;

use crate::config::advanced::ResolvedTaskInput;
use crate::observation::advanced::BoundObservationPlan;

use super::execution::{StatefulDefinition, TaskDefinition, TaskExecutionHost};
use super::model::ScientificModel;
use super::result::TaskResult;

/// A reusable type-erased compiled model definition.
///
/// `Task` deliberately carries no user-supplied identity, label, path,
/// lifecycle, progress callback, recording session, or scheduler policy.
/// Workflow derives those concerns from the validated project specification
/// and its runtime scope. Cloning a task clones only a shared definition
/// handle.
#[derive(Clone)]
pub(crate) struct Task {
    definition: Arc<dyn TaskDefinition>,
}

impl Task {
    /// Creates a type-erased definition for one compiled scientific model.
    ///
    /// Ordinary applications do not call this constructor: model registration
    /// and study composition invoke it automatically. It remains public through
    /// `task::advanced` for replacement study compilers and focused tests.
    pub(crate) fn for_model<M>() -> Self
    where
        M: ScientificModel,
    {
        Self {
            definition: Arc::new(StatefulDefinition::<M>::new()),
        }
    }
}

impl TaskDefinition for Task {
    fn execute(
        &self,
        input: &ResolvedTaskInput,
        observation_plan: &BoundObservationPlan,
        host: &mut dyn TaskExecutionHost,
    ) -> TaskResult {
        self.definition.execute(input, observation_plan, host)
    }
}

impl fmt::Debug for Task {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.debug_struct("Task").finish_non_exhaustive()
    }
}
