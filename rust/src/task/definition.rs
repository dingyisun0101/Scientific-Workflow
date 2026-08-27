//! Opaque application-facing task definitions.

use std::fmt;
use std::sync::Arc;

use serde::de::DeserializeOwned;

use crate::config::advanced::ResolvedTaskInput;
use crate::writer::advanced::Writer;

use super::execution::{
    OneShotDefinition, StatefulDefinition, TaskDefinition, TaskDescriptor, TaskExecutionHost,
};
use super::model::ScientificModel;
use super::result::TaskResult;

/// A reusable compiled task definition.
///
/// `Task` deliberately carries no user-supplied identity, label, path,
/// lifecycle, progress callback, recording session, or scheduler policy.
/// Workflow derives those concerns from the validated project specification
/// and its runtime scope. Cloning a task clones only a shared definition
/// handle.
#[derive(Clone)]
pub struct Task {
    definition: Arc<dyn TaskDefinition>,
}

impl Task {
    /// Defines stateful scientific work and derives its writer from the same
    /// typed constants used to initialize the model.
    ///
    /// Config supplies constants from one resolved task input; task validates
    /// the writer against the runtime-loaded state schema, initializes `M`,
    /// and then automatically observes the initial state, every successful
    /// step, and the final state. The writer factory borrows constants and
    /// cannot retain that borrow.
    pub fn stateful<M, W>(writer: W) -> Self
    where
        M: ScientificModel,
        W: Fn(&M::Constants) -> TaskResult<Writer> + Send + Sync + 'static,
    {
        Self {
            definition: Arc::new(StatefulDefinition::<M, W>::new(writer)),
        }
    }

    /// Defines typed one-shot work that needs neither scientific state nor a
    /// writer.
    ///
    /// Config supplies one `C` through its resolved-input decoding contract;
    /// task checks cancellation before invoking the callback and invokes it
    /// once.
    pub fn one_shot<C, F>(run: F) -> Self
    where
        C: DeserializeOwned + Send + Sync + 'static,
        F: Fn(C) -> TaskResult + Send + Sync + 'static,
    {
        Self {
            definition: Arc::new(OneShotDefinition::<C, F>::new(run)),
        }
    }
}

impl TaskDefinition for Task {
    fn descriptor(&self) -> &TaskDescriptor {
        self.definition.descriptor()
    }

    fn execute(&self, input: &ResolvedTaskInput, host: &mut dyn TaskExecutionHost) -> TaskResult {
        self.definition.execute(input, host)
    }
}

impl fmt::Debug for Task {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("Task")
            .field("descriptor", self.descriptor())
            .finish_non_exhaustive()
    }
}
