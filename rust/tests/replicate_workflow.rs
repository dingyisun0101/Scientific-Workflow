use std::fs;
use std::path::PathBuf;

use scientific_workflow::config::advanced::ProjectSpecification;
use scientific_workflow::execution::ReplicateExecutor;

const WORKER_INDEX_VARIABLE: &str = "SCIENTIFIC_WORKFLOW_REPLICATE_INDEX";

fn test_root() -> PathBuf {
    let executable = std::env::current_exe().unwrap();
    executable.with_extension("replicate-workflow")
}

#[test]
fn parallel_dispatch_reenters_one_isolated_worker_per_replicate() {
    let root = test_root();
    let is_controller = std::env::var_os(WORKER_INDEX_VARIABLE).is_none();
    if is_controller {
        let _ = fs::remove_dir_all(&root);
        fs::create_dir_all(root.join("config/inputs")).unwrap();
        fs::write(
            root.join("study.json"),
            r#"{
              "replicates": {
                "count": 3,
                "scheduling": "parallel",
                "failure_policy": "finish_all",
                "base_seed": 1101
              },
              "phases": {
                "test": {
                  "tasks": [{"definition": "test", "input": "inputs/test.json"}]
                }
              }
            }"#,
        )
        .unwrap();
        fs::write(root.join("config/state.json"), r#"{"fields":[]}"#).unwrap();
        fs::write(root.join("config/inputs/test.json"), "{}").unwrap();
    }

    let project = ProjectSpecification::load(&root).unwrap();
    let policy = project.manifest().replicate_policy();
    let dispatched = ReplicateExecutor::new(policy, root.join("output"))
        .dispatch_current_executable()
        .unwrap();

    if let Some(replicate) = dispatched {
        assert_eq!(replicate.count(), 3);
        assert_eq!(
            replicate.seed_deriver().unwrap().base_seed(),
            policy.base_seed().unwrap()
        );
        assert_eq!(
            replicate.seed_deriver().unwrap().replicate_index(),
            replicate.index()
        );
        fs::write(
            replicate.output_directory().join("worker.txt"),
            replicate.index().to_string(),
        )
        .unwrap();
        return;
    }

    for index in 0..3 {
        let directory = root.join("output").join(format!("replicate_{index}"));
        assert_eq!(
            fs::read_to_string(directory.join("worker.txt")).unwrap(),
            index.to_string()
        );
    }
    fs::remove_dir_all(root).unwrap();
}
