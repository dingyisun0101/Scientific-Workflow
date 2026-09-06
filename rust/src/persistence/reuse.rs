//! Private completed-task receipts and read-only import of legacy task outputs.

use std::collections::BTreeMap;
use std::fs::{self, File, OpenOptions};
use std::io::{self, Write};
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};
use serde_json::Value;

use super::{JsonPayloadDecoderRegistry, StoredStateSeriesReader};

type Error = Box<dyn std::error::Error + Send + Sync>;
type Result<T> = std::result::Result<T, Error>;
const FORMAT: &str = "scientific-workflow-task-result.v1";
const RECEIPT: &str = "workflow-result.json";

#[derive(Clone, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct CompletedTaskResult {
    format: String,
    pub(crate) identity: Box<str>,
    pub(crate) configuration: usize,
    pub(crate) output_directory: PathBuf,
    pub(crate) workload: Value,
    snapshot: Value,
}

pub(crate) type LegacyResults = BTreeMap<String, Vec<CompletedTaskResult>>;

pub(crate) struct TaskReuseExpectation<'a> {
    pub(crate) identity: &'a str,
    pub(crate) configuration: usize,
    pub(crate) snapshot: &'a [u8],
    pub(crate) execution_unit: Option<&'a str>,
    pub(crate) constants: Option<&'a Value>,
    pub(crate) state: Option<&'a str>,
    pub(crate) parameter_ordinal: Option<u64>,
    pub(crate) parameters: Value,
    pub(crate) kind: &'a str,
}

fn invalid(reason: impl Into<String>) -> Error {
    io::Error::new(io::ErrorKind::InvalidData, reason.into()).into()
}

fn document(path: &Path) -> Result<Value> {
    let bytes = fs::read(path)?;
    Ok(serde_json::from_slice(&bytes)?)
}

fn comparable(mut snapshot: Value) -> Value {
    if let Some(study) = snapshot.get_mut("study").and_then(Value::as_object_mut) {
        study.remove("active_phases");
        study.remove("reuse_from");
    }
    snapshot
}

/// Captures authoritative legacy execution-unit summaries written to dependent programs.
pub(crate) fn legacy_results(replicate: &Path) -> Result<LegacyResults> {
    let mut results = LegacyResults::new();
    for entry in fs::read_dir(replicate)? {
        let entry = entry?;
        if !entry.file_type()?.is_dir() {
            continue;
        }
        let directory = entry.path();
        let dependencies = directory.join("workflow-dependencies.json");
        if !dependencies.is_file() {
            continue;
        }
        let snapshot = document(&directory.join("workflow-config.json"))?;
        // Modern executions require a committed result receipt. Never treat an
        // incomplete new phase as a legacy completed phase.
        if snapshot["study"].get("active_phases").is_some() {
            continue;
        }
        let dependencies = document(&dependencies)?;
        let phases = dependencies
            .as_array()
            .ok_or_else(|| invalid("invalid dependency snapshot"))?;
        for phase in phases {
            let tasks = phase["tasks"]
                .as_array()
                .ok_or_else(|| invalid("invalid dependency tasks"))?;
            for task in tasks {
                let identity = task["identity"]
                    .as_str()
                    .ok_or_else(|| invalid("missing dependency identity"))?;
                let output_directory = serde_json::from_value(task["output_directory"].clone())?;
                results
                    .entry(identity.to_owned())
                    .or_default()
                    .push(CompletedTaskResult {
                        format: FORMAT.into(),
                        identity: identity.into(),
                        configuration: 0,
                        output_directory,
                        workload: task["workload"].clone(),
                        snapshot: snapshot.clone(),
                    });
            }
        }
    }
    Ok(results)
}

/// Loads a completed receipt, or the bounded legacy evidence needed by old studies.
pub(crate) fn load_result(
    directory: &Path,
    expected: &TaskReuseExpectation<'_>,
    legacy: &LegacyResults,
) -> Result<CompletedTaskResult> {
    let snapshot: Value = serde_json::from_slice(expected.snapshot)?;
    let receipt = directory.join(RECEIPT);
    let result: CompletedTaskResult = if receipt.is_file() {
        serde_json::from_value(document(&receipt)?)?
    } else if expected.execution_unit.is_none() && expected.kind != "npy" {
        let stored_snapshot = document(&directory.join("workflow-config.json"))?;
        if stored_snapshot["study"].get("active_phases").is_some() {
            return Err(invalid("phase has no committed task result"));
        }
        let program = document(&directory.join("program.json"))?;
        CompletedTaskResult {
            format: FORMAT.into(),
            identity: expected.identity.into(),
            configuration: expected.configuration,
            output_directory: directory.to_path_buf(),
            workload: serde_json::json!({
                "kind": program["kind"], "executable": program["program"],
                "python_script": program["python_script"],
            }),
            snapshot: stored_snapshot,
        }
    } else {
        let mut candidates = legacy
            .get(expected.identity)
            .into_iter()
            .flatten()
            .filter(|result| comparable(result.snapshot.clone()) == comparable(snapshot.clone()));
        let candidate = candidates.next().ok_or_else(|| {
            invalid("no committed result or matching legacy dependent-program summary")
        })?;
        if candidates.any(|other| {
            other.workload != candidate.workload
                || other.output_directory != candidate.output_directory
        }) {
            return Err(invalid("ambiguous legacy completed-task summaries"));
        }
        let mut result = candidate.clone();
        result.configuration = expected.configuration;
        if fs::canonicalize(&result.output_directory)? != fs::canonicalize(directory)? {
            return Err(invalid(
                "legacy task directory does not match the planned output ordinal",
            ));
        }
        result
    };
    if result.format != FORMAT
        || result.identity.as_ref() != expected.identity
        || result.configuration != expected.configuration
        || comparable(result.snapshot.clone()) != comparable(snapshot)
    {
        return Err(invalid(
            "completed task identity or captured study inputs differ from this plan",
        ));
    }
    if !result.output_directory.is_absolute()
        || result.output_directory.to_str().is_none()
        || !result.output_directory.is_dir()
    {
        return Err(invalid(
            "completed output must be an existing absolute UTF-8 directory",
        ));
    }
    validate_result(&result, expected)?;
    Ok(result)
}

fn validate_result(
    result: &CompletedTaskResult,
    expected: &TaskReuseExpectation<'_>,
) -> Result<()> {
    let kind = result.workload["kind"]
        .as_str()
        .ok_or_else(|| invalid("missing workload kind"))?;
    if kind != expected.kind {
        return Err(invalid("completed workload kind differs from this plan"));
    }
    if let Some(unit) = expected.execution_unit {
        if result.workload["execution_unit"].as_str() != Some(unit) {
            return Err(invalid("completed execution-unit registration differs"));
        }
        let members = result.workload["members"]
            .as_array()
            .ok_or_else(|| invalid("missing member summaries"))?;
        if members.is_empty() {
            return Err(invalid("completed execution unit has no members"));
        }
        let mut identities = std::collections::HashSet::new();
        for (index, member) in members.iter().enumerate() {
            let identity = member["identity"]
                .as_str()
                .ok_or_else(|| invalid("missing member identity"))?;
            if !identities.insert(identity) || member["final_iteration"].as_u64().is_none() {
                return Err(invalid("invalid member identity or final iteration"));
            }
            let directory: PathBuf = serde_json::from_value(member["output_directory"].clone())?;
            let canonical = fs::canonicalize(&directory)?;
            if !directory.is_absolute()
                || directory.to_str().is_none()
                || !canonical.starts_with(fs::canonicalize(&result.output_directory)?)
            {
                return Err(invalid("member recording escapes its task directory"));
            }
            let reader = StoredStateSeriesReader::open_completed_recording(
                &directory,
                JsonPayloadDecoderRegistry::new(),
            )?;
            let workflow = reader
                .user_metadata()
                .get("workflow")
                .ok_or_else(|| invalid("completed recording has no Workflow provenance"))?;
            if workflow["task_identity"].as_str() != Some(expected.identity)
                || workflow["execution_unit"].as_str() != Some(unit)
                || workflow["state"].as_str() != expected.state
                || workflow["parameter_ordinal"].as_u64() != expected.parameter_ordinal
                || workflow["member_index"].as_u64() != Some(index as u64)
                || workflow["member_identity"].as_str() != Some(identity)
                || workflow["parameters"] != expected.parameters
                || reader.user_metadata().get("constants") != expected.constants
            {
                return Err(invalid(
                    "completed member provenance differs from the planned scientific inputs",
                ));
            }
        }
    } else {
        let program = document(&result.output_directory.join("program.json"))?;
        if program["format"] != "scientific-workflow-program-v1"
            || program["status"] != "complete"
            || program["exit_code"] != 0
            || !result.output_directory.join("artifacts").is_dir()
        {
            return Err(invalid(
                "program output is missing or did not complete successfully",
            ));
        }
        if kind == "npy" {
            let processed: PathBuf =
                serde_json::from_value(result.workload["processed_directory"].clone())?;
            if !processed.is_absolute() || processed.to_str().is_none() || !processed.is_dir() {
                return Err(invalid("completed NumPy output is missing"));
            }
        }
    }
    Ok(())
}

/// Atomically commits successful results, preserving source paths for reused tasks.
pub(crate) fn write_result(
    directory: &Path,
    identity: &str,
    configuration: usize,
    output_directory: &Path,
    workload: Value,
    snapshot: &[u8],
) -> Result<()> {
    fs::create_dir_all(directory)?;
    let path = directory.join(RECEIPT);
    if path.exists() {
        return Err(invalid("task result receipt already exists"));
    }
    let result = CompletedTaskResult {
        format: FORMAT.into(),
        identity: identity.into(),
        configuration,
        output_directory: output_directory.to_path_buf(),
        workload,
        snapshot: serde_json::from_slice(snapshot)?,
    };
    let temporary = directory.join(format!(".{RECEIPT}.tmp-{}", std::process::id()));
    let mut file = OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&temporary)?;
    file.write_all(&serde_json::to_vec_pretty(&result)?)?;
    file.sync_all()?;
    fs::rename(temporary, path)?;
    File::open(directory)?.sync_all()?;
    Ok(())
}
