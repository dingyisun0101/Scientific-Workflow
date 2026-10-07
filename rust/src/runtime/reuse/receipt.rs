//! Current completed-task receipts and verified task-owned input snapshots.

use std::fs::{self, File, OpenOptions};
use std::io::{self, Write};
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::persistence::{
    JsonPayloadDecoderRegistry, StoredStateSeriesReader, TaskInputReferences, TaskInputSnapshots,
};

type Error = Box<dyn std::error::Error + Send + Sync>;
type Result<T> = std::result::Result<T, Error>;
const FORMAT: &str = "scientific-workflow-task-result.v2";
const RECEIPT: &str = "workflow-result.json";

#[derive(Clone, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct CompletedTaskResult {
    format: String,
    pub(crate) identity: Box<str>,
    pub(crate) configuration: usize,
    pub(crate) output_directory: PathBuf,
    pub(crate) workload: Value,
    inputs: TaskInputReferences,
}

pub(crate) struct TaskReuseExpectation<'a> {
    pub(crate) identity: &'a str,
    pub(crate) configuration: usize,
    pub(crate) snapshot: &'a [u8],
    pub(crate) execution_unit: Option<&'a str>,
    pub(crate) executable: Option<&'a Path>,
    pub(crate) python_script: Option<&'a Path>,
    pub(crate) constants: Option<&'a Value>,
    pub(crate) state: Option<&'a str>,
    pub(crate) parameter_ordinal: Option<u64>,
    pub(crate) parameters: Value,
    pub(crate) kind: &'a str,
    pub(crate) npy_filter_is_input: bool,
}

fn invalid(reason: impl Into<String>) -> Error {
    io::Error::new(io::ErrorKind::InvalidData, reason.into()).into()
}

fn document(path: &Path) -> Result<Value> {
    let bytes = fs::read(path)?;
    Ok(serde_json::from_slice(&bytes)?)
}

fn comparable(mut snapshot: Value, npy_filter_is_input: bool) -> Value {
    if let Some(study) = snapshot.get_mut("study").and_then(Value::as_object_mut) {
        study.remove("active_phases");
        study.remove("reuse_from");
        // Allocation and host policy are provenance, not scientific inputs.
        for key in ["threads", "compute", "disk", "persistence"] {
            study.remove(key);
        }
        if let Some(replicates) = study.get_mut("replicates").and_then(Value::as_object_mut) {
            replicates.remove("scheduling");
            replicates.remove("failure_policy");
            if replicates.is_empty() {
                study.remove("replicates");
            }
        }
        if let Some(phases) = study.get_mut("phases").and_then(Value::as_object_mut) {
            for phase in phases.values_mut().filter_map(Value::as_object_mut) {
                for key in [
                    "max_concurrency",
                    "start_interval_ms",
                    "timeout_ms",
                    "failure_policy",
                    "threads",
                    "mode",
                ] {
                    phase.remove(key);
                }
                if let Some(tasks) = phase.get_mut("tasks").and_then(Value::as_array_mut) {
                    for task in tasks.iter_mut().filter_map(Value::as_object_mut) {
                        task.remove("active");
                        task.remove("resources");
                        task.remove("timeout_ms");
                        if let Some(python) = task.get_mut("python").and_then(Value::as_object_mut)
                        {
                            python.remove("environment");
                        }
                    }
                }
            }
        }
        // Conversion selection cannot change an upstream recording or artifact.
        // Retain it for NPY itself and every phase consuming its output.
        if !npy_filter_is_input
            && let Some(npy) = study
                .get_mut("phases")
                .and_then(|phases| phases.get_mut("$npy"))
                .and_then(Value::as_object_mut)
        {
            npy.remove("exclude_streams");
        }
    }
    snapshot
}

/// Loads only the current committed receipt and verifies its original input files.
pub(crate) fn load_result(
    directory: &Path,
    expected: &TaskReuseExpectation<'_>,
) -> Result<CompletedTaskResult> {
    let result: CompletedTaskResult = serde_json::from_value(document(&directory.join(RECEIPT))?)?;
    if result.format != FORMAT
        || result.identity.as_ref() != expected.identity
        || result.configuration != expected.configuration
    {
        return Err(invalid(
            "unsupported receipt or completed task identity differs from this plan",
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
    let [config, _] =
        TaskInputSnapshots::verify_references(&result.output_directory, &result.inputs)?;
    if comparable(
        serde_json::from_slice(&config)?,
        expected.npy_filter_is_input,
    ) != comparable(
        serde_json::from_slice(expected.snapshot)?,
        expected.npy_filter_is_input,
    ) {
        return Err(invalid("captured study inputs differ from this plan"));
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
        if program["format"] != "scientific-workflow-program-v2"
            || serde_json::from_value::<TaskInputReferences>(program["inputs"].clone())?
                != result.inputs
            || program["kind"] != kind
            || program["status"] != "complete"
            || program["exit_code"] != 0
            || !result.output_directory.join("artifacts").is_dir()
        {
            return Err(invalid(
                "program output is missing or did not complete successfully",
            ));
        }
        if kind == "program" {
            let executable: PathBuf =
                serde_json::from_value(result.workload["executable"].clone())?;
            if Some(executable.as_path()) != expected.executable
                || serde_json::from_value::<PathBuf>(program["program"].clone())? != executable
            {
                return Err(invalid(
                    "completed resolved executable differs from this plan",
                ));
            }
        } else if kind == "python" {
            let script: PathBuf = serde_json::from_value(result.workload["python_script"].clone())?;
            if Some(script.as_path()) != expected.python_script
                || serde_json::from_value::<PathBuf>(program["python_script"].clone())? != script
            {
                return Err(invalid(
                    "completed resolved scientific script differs from this plan",
                ));
            }
        }
        if kind == "npy" {
            let processed: PathBuf =
                serde_json::from_value(result.workload["processed_directory"].clone())?;
            if !processed.is_absolute() || processed.to_str().is_none() || !processed.is_dir() {
                return Err(invalid("completed NumPy output is missing"));
            }
            let interpreter = expected
                .executable
                .ok_or_else(|| invalid("NPY plan has no interpreter"))?;
            let probe = "import sys, json, scientific_workflow\nif scientific_workflow.__version__ != '0.6.0': raise RuntimeError('NPY reuse requires scientific-workflow[npy] 0.6.0')\nfrom scientific_workflow.npy import open_npy_batch\nbatch = open_npy_batch(sys.argv[1])\nwith open(sys.argv[2], encoding='utf-8') as source: config = json.load(source)\nexpected = sorted(config['study']['phases']['$npy'].get('exclude_streams', []))\nif batch.manifest['exclude_streams'] != expected: raise RuntimeError('completed NPY exclusions differ from captured input')";
            let output = std::process::Command::new(interpreter)
                .args(["-c", probe])
                .arg(&processed)
                .arg(result.output_directory.join("workflow-config.json"))
                .output()?;
            if !output.status.success() {
                return Err(invalid(format!(
                    "completed NPY batch verification failed: {}",
                    String::from_utf8_lossy(&output.stderr).trim()
                )));
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
        inputs: TaskInputSnapshots::open(output_directory)?
            .references()
            .clone(),
    };
    let temporary = directory.join(format!(".{RECEIPT}.tmp-{}", std::process::id()));
    let mut file = OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&temporary)?;
    file.write_all(&serde_json::to_vec_pretty(&result)?)?;
    file.sync_all()?;
    // Publish without replacing a receipt created by another writer.
    fs::hard_link(&temporary, &path)?;
    fs::remove_file(temporary)?;
    File::open(directory)?.sync_all()?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::comparable;
    use serde_json::{Value, json};

    fn snapshot() -> Value {
        json!({
            "study": {
                "workflow_schema": 1,
                "active_phases": [0, 1, 2],
                "seed": 1101,
                "threads": 2,
                "phases": {
                    "prepare": {"tasks": [{"program": "/bin/true"}]},
                    "evolve": {
                        "after": ["prepare"],
                        "tasks": [{"execution_unit": "model"}]
                    },
                    "$npy": {"after": ["evolve"]}
                }
            },
            "config": {"parameters.json": {"model": {"maximum_iterations": 36000}}}
        })
    }

    #[test]
    fn upstream_reuse_ignores_only_conversion_exclusions() {
        let original = snapshot();
        for exclusions in [
            json!([]),
            json!(["checkpoint"]),
            json!(["space", "checkpoint"]),
        ] {
            let mut filtered = original.clone();
            filtered["study"]["phases"]["$npy"]["exclude_streams"] = exclusions;
            assert_eq!(
                comparable(original.clone(), false),
                comparable(filtered, false)
            );
        }
    }

    #[test]
    fn npy_and_its_consumers_keep_filter_identity() {
        let original = snapshot();
        let mut filtered = original.clone();
        filtered["study"]["phases"]["$npy"]["exclude_streams"] = json!(["checkpoint"]);
        assert_ne!(
            comparable(original, true),
            comparable(filtered.clone(), true)
        );
        let mut changed = filtered.clone();
        changed["study"]["phases"]["$npy"]["exclude_streams"] = json!(["space"]);
        assert_ne!(
            comparable(filtered.clone(), true),
            comparable(changed, true)
        );
        assert_eq!(
            comparable(filtered.clone(), true),
            comparable(filtered, true)
        );
    }

    #[test]
    fn scientific_inputs_and_other_phase_settings_still_must_match() {
        let original = snapshot();
        for (pointer, value) in [
            ("/study/seed", json!(1102)),
            (
                "/config/parameters.json/model/maximum_iterations",
                json!(36001),
            ),
            ("/study/phases/prepare/tasks/0/program", json!("/bin/false")),
            ("/study/phases/$npy/after", json!(["prepare"])),
        ] {
            let mut changed = original.clone();
            *changed.pointer_mut(pointer).unwrap() = value;
            changed["study"]["phases"]["$npy"]["exclude_streams"] = json!(["checkpoint"]);
            assert_ne!(
                comparable(original.clone(), false),
                comparable(changed, false)
            );
        }
    }

    #[test]
    fn selection_and_reuse_source_remain_compatible() {
        let original = snapshot();
        let mut resumed = original.clone();
        resumed["study"]["active_phases"] = json!([1, 2]);
        resumed["study"]["reuse_from"] = json!("output/execution-previous");
        resumed["study"]["phases"]["evolve"]["tasks"][0]["active"] = json!(true);
        for npy_filter_is_input in [false, true] {
            assert_eq!(
                comparable(original.clone(), npy_filter_is_input),
                comparable(resumed.clone(), npy_filter_is_input)
            );
        }
    }

    #[test]
    fn operational_settings_do_not_invalidate_completed_science() {
        let original = snapshot();
        let mut changed = original.clone();
        changed["study"]["threads"] = json!(8);
        changed["study"]["compute"] = json!({"mode": "isolated"});
        changed["study"]["disk"] = json!({"pause_at_percent": null});
        changed["study"]["persistence"] = json!({"chunk_target_mb": 8});
        let phase = &mut changed["study"]["phases"]["prepare"];
        phase["max_concurrency"] = json!(4);
        phase["timeout_ms"] = json!(1000);
        phase["tasks"][0]["resources"] = json!({"threads": 3});
        phase["tasks"][0]["timeout_ms"] = json!(500);
        assert_eq!(comparable(original, true), comparable(changed, true));
    }
}
