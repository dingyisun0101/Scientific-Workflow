//! Phase-level completion examination.
//!
//! Completion examination answers one orchestration question: whether an
//! entire declared phase already has a valid application-owned result. The
//! application supplies the examination because only it understands artifact
//! identity, schema, configuration compatibility, and scientific validity.
//! Workflow evaluates the answer once per relevant phase and reuses that same
//! snapshot for selection, dependency resolution, execution, display, and
//! recording.
//!
//! # Boundary
//!
//! Examination never inspects or resumes individual tasks. A phase reported as
//! [`PhaseCompletion::Incomplete`] is invoked normally. Any validation, reuse,
//! cleanup, or continuation within that phase remains the responsibility of
//! its application workloads. This keeps Workflow's authority at phase level
//! while allowing model libraries to retain their native recovery semantics.

use std::collections::HashMap;
use std::fmt;
use std::panic::{AssertUnwindSafe, catch_unwind};
use std::sync::Arc;

use super::error::StudyError;
use super::phase::{Phase, PhaseId};

/// Application verdict for one complete phase declaration.
///
/// Examiners must be read-only and deterministic for the duration of one study
/// launch. Workflow may examine a selected phase before opening the durable
/// execution record so invalid state fails without starting new work.
/// Workflow does not lock application resources; the application must keep a
/// completed result stable after returning [`PhaseCompletion::Complete`].
#[derive(Clone, Debug, Eq, PartialEq)]
#[non_exhaustive]
pub enum PhaseCompletion {
    /// No prior phase result exists; execute the phase normally.
    Missing,
    /// Some prior state exists, but the phase is not complete.
    ///
    /// Workflow emits an explicit warning and invokes the phase. The detail
    /// should identify the partial state without prescribing how tasks resume.
    Incomplete(String),
    /// The application verified the whole phase result; reuse it without
    /// entering the task scheduler.
    Complete,
    /// Prior state exists but is invalid, incompatible, or unverifiable.
    ///
    /// Invalid state fails closed before any selected phase starts.
    Invalid(String),
}

impl PhaseCompletion {
    /// Creates an incomplete verdict with concise application-owned context.
    pub fn incomplete(detail: impl Into<String>) -> Self {
        Self::Incomplete(detail.into())
    }

    /// Creates an invalid verdict with a concise failure reason.
    pub fn invalid(reason: impl Into<String>) -> Self {
        Self::Invalid(reason.into())
    }

    pub(crate) const fn is_complete(&self) -> bool {
        matches!(self, Self::Complete)
    }
}

pub(crate) type Examiner = Arc<dyn Fn() -> PhaseCompletion + Send + Sync + 'static>;

/// Debuggable type-erased examiner stored by an immutable phase.
#[derive(Clone)]
pub(crate) struct PhaseCompletionExaminer(Examiner);

impl PhaseCompletionExaminer {
    pub(crate) fn new<F>(examiner: F) -> Self
    where
        F: Fn() -> PhaseCompletion + Send + Sync + 'static,
    {
        Self(Arc::new(examiner))
    }

    fn examine(&self) -> Result<PhaseCompletion, ()> {
        catch_unwind(AssertUnwindSafe(|| (self.0)())).map_err(|_| ())
    }
}

impl fmt::Debug for PhaseCompletionExaminer {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("PhaseCompletionExaminer(..)")
    }
}

/// One launch-local cache shared by selection and execution preparation.
///
/// The cache prevents filesystem state from being interpreted differently by
/// separate dependency and skip checks during one launch.
pub(crate) struct CompletionExamination {
    enabled: bool,
    outcomes: HashMap<PhaseId, PhaseCompletion>,
}

impl CompletionExamination {
    pub(crate) fn new(enabled: bool) -> Self {
        Self {
            enabled,
            outcomes: HashMap::new(),
        }
    }

    pub(crate) const fn is_enabled(&self) -> bool {
        self.enabled
    }

    pub(crate) fn examine(&mut self, phase: &Phase) -> Result<PhaseCompletion, StudyError> {
        if let Some(outcome) = self.outcomes.get(&phase.id()) {
            return Ok(outcome.clone());
        }
        let outcome = if self.enabled {
            match phase.completion_examiner() {
                Some(examiner) => examiner.examine().map_err(|()| {
                    StudyError::PhaseCompletionExaminationPanicked {
                        phase: phase.id().get(),
                    }
                })?,
                None => PhaseCompletion::Missing,
            }
        } else {
            PhaseCompletion::Missing
        };
        if let PhaseCompletion::Invalid(reason) = &outcome {
            return Err(StudyError::InvalidPhaseCompletion {
                phase: phase.id().get(),
                reason: reason.clone(),
            });
        }
        self.outcomes.insert(phase.id(), outcome.clone());
        Ok(outcome)
    }

    pub(crate) fn into_outcomes(self) -> HashMap<PhaseId, PhaseCompletion> {
        self.outcomes
    }
}
