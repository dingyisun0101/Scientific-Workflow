//! Selects complete prerequisite phases and preserves their existing output references.

use std::collections::{BTreeMap, HashSet};
use std::path::{Path, PathBuf};

use serde::Deserialize;

use crate::persistence::reuse::{self, TaskReuseExpectation};
use crate::study::{Study, StudyPhase};

use super::RuntimeError;
use super::execution::dependency_workload;
use super::summary::{MemberRunSummary, PhaseRunSummary, TaskRunKind, TaskRunSummary};

pub(super) type ReusedPhases = BTreeMap<(u64, usize), PhaseRunSummary>;

fn ancestors<'a>(phase: &'a StudyPhase, study: &'a Study, found: &mut HashSet<&'a str>) {
    for name in phase.dependencies() {
        if found.insert(name) {
            let dependency = study
                .phases()
                .iter()
                .find(|phase| phase.name() == name)
                .expect("Config validates dependency names");
            ancestors(dependency, study, found);
        }
    }
}

pub(super) fn prepare(study: &Study) -> Result<ReusedPhases, RuntimeError> {
    let mut needed = HashSet::new();
    let active = study
        .phase_order()
        .iter()
        .enumerate()
        .filter(|(index, _)| study.phase_is_active(*index))
        .map(|(_, &position)| study.phases()[position].name())
        .collect::<HashSet<_>>();
    for phase in study
        .phases()
        .iter()
        .filter(|phase| active.contains(phase.name()))
    {
        ancestors(phase, study, &mut needed);
    }
    let skipped = study
        .phase_order()
        .iter()
        .enumerate()
        .filter(|(index, position)| {
            !study.phase_is_active(*index) && needed.contains(study.phases()[**position].name())
        })
        .collect::<Vec<_>>();
    if skipped.is_empty() {
        return Ok(ReusedPhases::new());
    }
    let source = study.reuse_from().ok_or_else(|| RuntimeError::Reuse {
        phase: study.phases()[*skipped[0].1].name().to_owned(),
        path: study.project_root().to_path_buf(),
        reason: "inactive prerequisite phases require an explicit reuse_from execution directory"
            .into(),
    })?;
    for &(_, &position) in &skipped {
        let phase = &study.phases()[position];
        let mut predecessors = HashSet::new();
        ancestors(phase, study, &mut predecessors);
        if predecessors.iter().any(|name| active.contains(name)) {
            return Err(RuntimeError::Reuse {
                phase: phase.name().to_owned(),
                path: source.to_path_buf(),
                reason: "cannot reuse a phase whose prerequisite is selected to run again".into(),
            });
        }
    }
    let mut reused = ReusedPhases::new();
    for replicate in 0..study.replicate_policy().count() {
        let directory = source.join(format!("replicate-{replicate:06}"));
        let legacy = reuse::legacy_results(&directory).map_err(|error| RuntimeError::Reuse {
            phase: study.phases()[*skipped[0].1].name().to_owned(),
            path: directory.clone(),
            reason: error.to_string(),
        })?;
        for &(index, &position) in &skipped {
            let phase = &study.phases()[position];
            let mut predecessors = HashSet::new();
            ancestors(phase, study, &mut predecessors);
            let npy_filter_is_input = phase.name() == "$npy" || predecessors.contains("$npy");
            let mut tasks = Vec::with_capacity(phase.tasks().len());
            for task in phase.tasks() {
                let path = directory.join(format!("task-{:06}", task.output_ordinal()));
                let snapshot = task.config_snapshot();
                let provenance = task.execution_unit_provenance();
                let mut parameters = snapshot.parameters().clone();
                if let Some(provenance) = &provenance {
                    parameters[provenance.execution_unit()] = provenance.constants().clone();
                }
                let expected = TaskReuseExpectation {
                    identity: task.identity(),
                    configuration: task.configuration(),
                    snapshot: snapshot.bytes(),
                    execution_unit: task.execution_unit(),
                    constants: provenance.as_ref().map(|p| p.constants()),
                    state: provenance.as_ref().map(|p| p.state()),
                    parameter_ordinal: provenance.as_ref().map(|p| p.parameter_ordinal()),
                    parameters,
                    npy_filter_is_input,
                    kind: if task.is_npy() {
                        "npy"
                    } else {
                        task.kind_name()
                    },
                };
                let result = reuse::load_result(&path, &expected, &legacy)
                    .and_then(|result| {
                        let workload: StoredWorkload = serde_json::from_value(result.workload)?;
                        Ok(TaskRunSummary {
                            identity: result.identity,
                            configuration: result.configuration,
                            output_directory: result.output_directory,
                            kind: workload.into(),
                        })
                    })
                    .map_err(|error| RuntimeError::Reuse {
                        phase: phase.name().to_owned(),
                        path: path.clone(),
                        reason: error.to_string(),
                    })?;
                tasks.push(result);
            }
            reused.insert(
                (replicate, index),
                PhaseRunSummary {
                    name: phase.name().into(),
                    tasks: tasks.into_boxed_slice(),
                    reused: true,
                },
            );
        }
    }
    Ok(reused)
}

pub(super) fn persist_phase(
    phase: &StudyPhase,
    summary: &PhaseRunSummary,
    replicate: &Path,
) -> Result<(), RuntimeError> {
    for (task, result) in phase.tasks().iter().zip(summary.tasks()) {
        let directory = replicate.join(format!("task-{:06}", task.output_ordinal()));
        reuse::write_result(
            &directory,
            task.identity(),
            task.configuration(),
            result.output_directory(),
            dependency_workload(result.kind()),
            task.config_snapshot().bytes(),
        )
        .map_err(|source| RuntimeError::Task {
            task: task.identity().to_owned(),
            source,
        })?;
    }
    Ok(())
}

#[derive(Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
enum StoredWorkload {
    ExecutionUnit {
        execution_unit: Box<str>,
        members: Vec<StoredMember>,
    },
    Program {
        executable: PathBuf,
        python_script: Option<PathBuf>,
    },
    Python {
        executable: PathBuf,
        python_script: Option<PathBuf>,
    },
    Npy {
        launcher: PathBuf,
        processed_directory: PathBuf,
    },
}

#[derive(Deserialize)]
struct StoredMember {
    identity: Box<str>,
    final_iteration: u64,
    output_directory: PathBuf,
}

impl From<StoredWorkload> for TaskRunKind {
    fn from(value: StoredWorkload) -> Self {
        match value {
            StoredWorkload::ExecutionUnit {
                execution_unit,
                members,
            } => Self::ExecutionUnit {
                execution_unit,
                members: members
                    .into_iter()
                    .map(|member| MemberRunSummary {
                        identity: member.identity,
                        final_iteration: member.final_iteration,
                        output_directory: member.output_directory,
                    })
                    .collect(),
            },
            StoredWorkload::Program {
                executable,
                python_script,
            }
            | StoredWorkload::Python {
                executable,
                python_script,
            } => Self::Program {
                executable,
                python_script,
            },
            StoredWorkload::Npy {
                launcher,
                processed_directory,
            } => Self::Npy {
                launcher,
                processed_directory,
            },
        }
    }
}
