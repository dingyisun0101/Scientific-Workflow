//! Typed, immutable results supplied to a task by its declared dependencies.
//!
//! Selectors preserve snapshot order and never read scientific files. Runtime
//! owns dependency scope; this API cannot discover unrelated output directories.

use serde::Deserialize;
use serde_json::Value;
use std::path::{Path, PathBuf};
use thiserror::Error;

/// Invalid input or a dependency selection with unexpected cardinality.
#[derive(Debug, Error)]
#[non_exhaustive]
pub enum DependencyError {
    /// An environment variable required by the standard launch contract is absent.
    #[error("missing {variable}; run this program through Workflow's standard study layout")]
    MissingEnvironment {
        /// Required variable name.
        variable: &'static str,
    },
    /// A snapshot could not be read.
    #[error("cannot read dependency snapshot `{path}`")]
    Io {
        /// Expected snapshot path.
        path: PathBuf,
        /// Underlying filesystem failure.
        #[source]
        source: std::io::Error,
    },
    /// A known result or the snapshot structure is malformed.
    #[error("invalid dependency snapshot: {0}")]
    Invalid(String),
    /// No result satisfies the selection.
    #[error("no dependency matches {selection}")]
    Missing {
        /// Applied filters.
        selection: String,
    },
    /// Several results satisfy a selection requiring at most one.
    #[error("ambiguous dependency selection {selection}: {matches:?}; select a phase or task")]
    Ambiguous {
        /// Applied filters.
        selection: String,
        /// Candidate source identifiers in snapshot order.
        matches: Vec<String>,
    },
}

/// One validated dependency snapshot. Unknown workload kinds remain in raw JSON.
#[derive(Clone, Debug)]
pub struct Dependencies {
    raw: Value,
    recordings: Vec<RecordingDependency>,
    programs: Vec<ProgramDependency>,
    npy_batches: Vec<NpyDependency>,
}

#[derive(Deserialize)]
struct Phase {
    phase: String,
    tasks: Vec<Task>,
}
#[derive(Deserialize)]
struct Task {
    identity: String,
    output_directory: PathBuf,
    workload: Value,
}
#[derive(Deserialize)]
struct Member {
    identity: String,
    final_iteration: u64,
    output_directory: PathBuf,
}
#[derive(Deserialize)]
struct Unit {
    execution_unit: String,
    members: Vec<Member>,
}
#[derive(Deserialize)]
struct Program {
    executable: PathBuf,
    python_script: Option<PathBuf>,
}
#[derive(Deserialize)]
struct Npy {
    processed_directory: PathBuf,
}

impl Dependencies {
    /// Validates an owned JSON snapshot without opening its referenced files.
    pub fn from_json(raw: Value) -> Result<Self, DependencyError> {
        let phases: Vec<Phase> = decode(raw.clone())?;
        let mut result = Self {
            raw,
            recordings: Vec::new(),
            programs: Vec::new(),
            npy_batches: Vec::new(),
        };
        let mut phases_seen = std::collections::HashSet::new();
        for phase in phases {
            name(&phase.phase)?;
            if !phases_seen.insert(phase.phase.clone()) {
                return Err(DependencyError::Invalid("duplicate phase identity".into()));
            }
            let mut tasks_seen = std::collections::HashSet::new();
            for task in phase.tasks {
                name(&task.identity)?;
                path(&task.output_directory)?;
                if !tasks_seen.insert(task.identity.clone()) {
                    return Err(DependencyError::Invalid(
                        "duplicate task identity in phase".into(),
                    ));
                }
                let source = Source {
                    phase: phase.phase.clone(),
                    task: task.identity,
                };
                let kind = task
                    .workload
                    .get("kind")
                    .and_then(Value::as_str)
                    .ok_or_else(|| {
                        DependencyError::Invalid("workload.kind must be a nonempty string".into())
                    })?;
                name(kind)?;
                match kind {
                    "execution_unit" => {
                        let unit: Unit = decode(task.workload)?;
                        name(&unit.execution_unit)?;
                        let mut members_seen = std::collections::HashSet::new();
                        if unit.members.is_empty() {
                            return Err(DependencyError::Invalid(
                                "execution unit has no members".into(),
                            ));
                        }
                        for member in unit.members {
                            name(&member.identity)?;
                            path(&member.output_directory)?;
                            if !members_seen.insert(member.identity.clone()) {
                                return Err(DependencyError::Invalid(
                                    "duplicate member identity".into(),
                                ));
                            }
                            result.recordings.push(RecordingDependency {
                                source: source.clone(),
                                execution_unit: unit.execution_unit.clone(),
                                member: member.identity,
                                final_iteration: member.final_iteration,
                                directory: member.output_directory,
                            });
                        }
                    }
                    "program" | "python" => {
                        let program: Program = decode(task.workload.clone())?;
                        path(&program.executable)?;
                        if kind == "python" && program.python_script.is_none() {
                            return Err(DependencyError::Invalid(
                                "python workload requires python_script".into(),
                            ));
                        }
                        if let Some(script) = &program.python_script {
                            path(script)?;
                        }
                        result.programs.push(ProgramDependency {
                            source,
                            directory: task.output_directory.join("artifacts"),
                            executable: program.executable,
                            python_script: program.python_script,
                        });
                    }
                    "npy" => {
                        let npy: Npy = decode(task.workload)?;
                        path(&npy.processed_directory)?;
                        result.npy_batches.push(NpyDependency {
                            source,
                            directory: npy.processed_directory,
                        });
                    }
                    _ => {} // Forward-compatible raw access; known kinds are always validated.
                }
            }
        }
        Ok(result)
    }

    /// Loads and validates a standard dependency snapshot from an explicit file.
    pub fn load(path: &Path) -> Result<Self, DependencyError> {
        let bytes = std::fs::read(path).map_err(|source| DependencyError::Io {
            path: path.to_owned(),
            source,
        })?;
        Self::from_json(
            serde_json::from_slice(&bytes)
                .map_err(|error| DependencyError::Invalid(error.to_string()))?,
        )
    }

    /// Loads `WORKFLOW_DEPENDENCIES_PATH`, supplied by a Workflow program launch.
    pub fn from_env() -> Result<Self, DependencyError> {
        let path = std::env::var_os("WORKFLOW_DEPENDENCIES_PATH").ok_or(
            DependencyError::MissingEnvironment {
                variable: "WORKFLOW_DEPENDENCIES_PATH",
            },
        )?;
        Self::load(Path::new(&path))
    }

    /// Borrows the original snapshot, including unknown extension fields/kinds.
    pub const fn raw_json(&self) -> &Value {
        &self.raw
    }
    /// Selects completed member recordings in deterministic snapshot order.
    pub fn recordings(&self) -> Selection<'_, RecordingDependency> {
        Selection::new(&self.recordings)
    }
    /// Selects program and Python task artifact directories.
    pub fn programs(&self) -> Selection<'_, ProgramDependency> {
        Selection::new(&self.programs)
    }
    /// Selects aggregate NPY batches; this does not filter members within a batch.
    pub fn npy_batches(&self) -> Selection<'_, NpyDependency> {
        Selection::new(&self.npy_batches)
    }
}

fn decode<T: serde::de::DeserializeOwned>(value: Value) -> Result<T, DependencyError> {
    serde_json::from_value(value).map_err(|error| DependencyError::Invalid(error.to_string()))
}
fn name(value: &str) -> Result<(), DependencyError> {
    if value.is_empty() || value.trim() != value {
        return Err(DependencyError::Invalid(
            "identifiers must be nonempty without surrounding whitespace".into(),
        ));
    }
    Ok(())
}
fn path(value: &Path) -> Result<(), DependencyError> {
    if !value.is_absolute() {
        return Err(DependencyError::Invalid(format!(
            "expected absolute path, got {}",
            value.display()
        )));
    }
    Ok(())
}

#[derive(Clone, Debug)]
struct Source {
    phase: String,
    task: String,
}

mod sealed {
    pub trait Sealed {}
}
/// Shared identity access for Workflow-owned dependency result types.
///
/// This trait is sealed; downstream implementations are not supported.
pub trait Dependency: sealed::Sealed {
    /// Declared phase key.
    fn phase(&self) -> &str;
    /// Runtime-generated task identity.
    fn task(&self) -> &str;
    /// Human-readable source used in ambiguity diagnostics.
    fn description(&self) -> String;
}

/// Borrowed, filtered dependency results. Filters combine by intersection.
#[derive(Clone, Debug)]
pub struct Selection<'a, T> {
    entries: Vec<&'a T>,
    filters: Vec<String>,
}
impl<'a, T: Dependency> Selection<'a, T> {
    fn new(entries: &'a [T]) -> Self {
        Self {
            entries: entries.iter().collect(),
            filters: Vec::new(),
        }
    }
    fn filter(mut self, label: String, predicate: impl Fn(&T) -> bool) -> Self {
        self.entries.retain(|entry| predicate(entry));
        self.filters.push(label);
        self
    }
    /// Restricts matches to a declared phase key.
    pub fn in_phase(self, phase: &str) -> Self {
        self.filter(format!("phase={phase:?}"), |entry| entry.phase() == phase)
    }
    /// Restricts matches to an exact runtime task identity.
    pub fn task(self, task: &str) -> Self {
        self.filter(format!("task={task:?}"), |entry| entry.task() == task)
    }
    /// Returns exactly one result, reporting missing or ambiguous matches.
    pub fn one(&self) -> Result<&'a T, DependencyError> {
        self.optional()?.ok_or_else(|| DependencyError::Missing {
            selection: self.label(),
        })
    }
    /// Returns zero or one result; several matches produce an ambiguity error.
    pub fn optional(&self) -> Result<Option<&'a T>, DependencyError> {
        match self.entries.as_slice() {
            [] => Ok(None),
            [entry] => Ok(Some(*entry)),
            _ => Err(DependencyError::Ambiguous {
                selection: self.label(),
                matches: self
                    .entries
                    .iter()
                    .map(|entry| entry.description())
                    .collect(),
            }),
        }
    }
    /// Iterates all matching results in snapshot order, including zero matches.
    pub fn iter(&self) -> impl ExactSizeIterator<Item = &'a T> + '_ {
        self.entries.iter().copied()
    }
    fn label(&self) -> String {
        format!(
            "{} [{}]",
            std::any::type_name::<T>(),
            self.filters.join(", ")
        )
    }
}

impl Selection<'_, RecordingDependency> {
    /// Restricts recordings to an exact execution-unit registration key.
    pub fn execution_unit(self, key: &str) -> Self {
        self.filter(format!("execution_unit={key:?}"), |entry| {
            entry.execution_unit == key
        })
    }
    /// Restricts recordings to an exact member identity.
    pub fn member(self, member: &str) -> Self {
        self.filter(format!("member={member:?}"), |entry| entry.member == member)
    }
}

/// A completed member recording.
#[derive(Clone, Debug)]
pub struct RecordingDependency {
    source: Source,
    execution_unit: String,
    member: String,
    final_iteration: u64,
    directory: PathBuf,
}
impl RecordingDependency {
    /// Declared phase key.
    pub fn phase(&self) -> &str {
        &self.source.phase
    }
    /// Runtime-generated task identity.
    pub fn task(&self) -> &str {
        &self.source.task
    }
    /// Execution-unit registration key.
    pub fn execution_unit(&self) -> &str {
        &self.execution_unit
    }
    /// Member identity within its execution unit.
    pub fn member(&self) -> &str {
        &self.member
    }
    /// Final scientific iteration.
    pub fn final_iteration(&self) -> u64 {
        self.final_iteration
    }
    /// Completed recording directory.
    pub fn directory(&self) -> &Path {
        &self.directory
    }
}
impl sealed::Sealed for RecordingDependency {}
impl Dependency for RecordingDependency {
    fn phase(&self) -> &str {
        self.phase()
    }
    fn task(&self) -> &str {
        self.task()
    }
    fn description(&self) -> String {
        format!("{}/{}", self.phase(), self.task()) + &format!("/{}", self.member)
    }
}

/// A completed external executable or Python task.
#[derive(Clone, Debug)]
pub struct ProgramDependency {
    source: Source,
    directory: PathBuf,
    executable: PathBuf,
    python_script: Option<PathBuf>,
}
impl ProgramDependency {
    /// Declared phase key.
    pub fn phase(&self) -> &str {
        &self.source.phase
    }
    /// Runtime-generated task identity.
    pub fn task(&self) -> &str {
        &self.source.task
    }
    /// Standard artifacts directory, excluding logs and snapshots.
    pub fn directory(&self) -> &Path {
        &self.directory
    }
    /// Resolved launcher executable.
    pub fn executable(&self) -> &Path {
        &self.executable
    }
    /// Canonical Python script when this was a Python task.
    pub fn python_script(&self) -> Option<&Path> {
        self.python_script.as_deref()
    }
}
impl sealed::Sealed for ProgramDependency {}
impl Dependency for ProgramDependency {
    fn phase(&self) -> &str {
        self.phase()
    }
    fn task(&self) -> &str {
        self.task()
    }
    fn description(&self) -> String {
        format!("{}/{}", self.phase(), self.task())
    }
}

/// An aggregate converted NPY batch.
#[derive(Clone, Debug)]
pub struct NpyDependency {
    source: Source,
    directory: PathBuf,
}
impl NpyDependency {
    /// Declared phase key.
    pub fn phase(&self) -> &str {
        &self.source.phase
    }
    /// Runtime-generated task identity.
    pub fn task(&self) -> &str {
        &self.source.task
    }
    /// Verified-reader entry directory containing the batch manifest.
    pub fn directory(&self) -> &Path {
        &self.directory
    }
}
impl sealed::Sealed for NpyDependency {}
impl Dependency for NpyDependency {
    fn phase(&self) -> &str {
        self.phase()
    }
    fn task(&self) -> &str {
        self.task()
    }
    fn description(&self) -> String {
        format!("{}/{}", self.phase(), self.task())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn snapshot() -> Value {
        serde_json::json!([
            {"phase":"initialize", "tasks":[{"identity":"a", "output_directory":"/run/a", "workload":{"kind":"execution_unit","execution_unit":"init","members":[{"identity":"one","final_iteration":4,"output_directory":"/run/a"}]}}]},
            {"phase":"other", "tasks":[{"identity":"b", "output_directory":"/run/b", "workload":{"kind":"execution_unit","execution_unit":"init","members":[{"identity":"one","final_iteration":5,"output_directory":"/run/b"}]}}]}
        ])
    }
    #[test]
    fn selection_is_explicit_and_preserves_order_and_source() {
        let deps = Dependencies::from_json(snapshot()).unwrap();
        assert!(
            matches!(deps.recordings().execution_unit("init").one(), Err(DependencyError::Ambiguous { matches, .. }) if matches == ["initialize/a/one", "other/b/one"])
        );
        let selected = deps
            .recordings()
            .in_phase("initialize")
            .member("one")
            .one()
            .unwrap();
        assert_eq!(selected.directory(), Path::new("/run/a"));
        assert_eq!(
            deps.recordings()
                .iter()
                .map(|r| r.final_iteration())
                .collect::<Vec<_>>(),
            [4, 5]
        );
        assert!(
            deps.recordings()
                .in_phase("missing")
                .optional()
                .unwrap()
                .is_none()
        );
        assert!(matches!(
            deps.recordings().in_phase("missing").one(),
            Err(DependencyError::Missing { .. })
        ));
        assert_eq!(deps.raw_json(), &snapshot());
    }
    #[test]
    fn malformed_known_results_fail_and_extensions_remain_available() {
        let mut value = snapshot();
        value[0]["tasks"][0]["workload"]["members"][0]["final_iteration"] = "bad".into();
        assert!(Dependencies::from_json(value).is_err());
        let value = serde_json::json!([{"phase":"extension","tasks":[{"identity":"task","output_directory":"/output","workload":{"kind":"future","extra":1}}]}]);
        let deps = Dependencies::from_json(value.clone()).unwrap();
        assert_eq!(deps.raw_json(), &value);
        assert_eq!(deps.recordings().iter().len(), 0);
    }
}
