//! Conversion-only changes must not invalidate completed scientific prerequisites.

#[cfg(unix)]
mod unix {
    use std::fs;
    use std::path::{Path, PathBuf};
    use std::time::{SystemTime, UNIX_EPOCH};

    use serde_json::{Value, json};

    struct Project(PathBuf);

    impl Project {
        fn new() -> Self {
            let stamp = SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_nanos();
            let path = std::env::temp_dir()
                .join(format!("workflow-reuse-npy-{}-{stamp}", std::process::id()));
            fs::create_dir_all(path.join("wf_configs")).unwrap();
            Self(path)
        }

        fn write(&self, name: &str, value: &Value) {
            fs::write(
                self.0.join("wf_configs").join(name),
                serde_json::to_vec_pretty(value).unwrap(),
            )
            .unwrap();
        }
    }

    impl Drop for Project {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }

    fn executions(project: &Path) -> Vec<PathBuf> {
        fs::read_dir(project.join("output"))
            .unwrap()
            .map(|entry| entry.unwrap().path())
            .collect()
    }

    #[test]
    fn changed_npy_filters_allow_chained_upstream_reuse_but_not_changed_science() {
        let project = Project::new();
        let mut study = json!({
            "workflow_schema": 1,
            "active_phases": [0],
            "threads": 2,
            "compute": {"mode": "auto"},
            "seed": 1101,
            "phases": {
                "10_prepare": {"tasks": [{
                    "program": "/bin/sh",
                    "args": ["-c", "printf complete > prepared.txt"]
                }]},
                "20_evolve": {
                    "after": ["10_prepare"],
                    "tasks": [{
                        "program": "/bin/sh",
                        "args": ["-c", "printf complete > evolved.txt"]
                    }]
                },
                "$npy": {"after": ["20_evolve"]}
            }
        });
        project.write("study.json", &study);
        project.write("parameters.json", &json!({"maximum_iterations": 36000}));
        scientific_workflow::run(&project.0).unwrap();
        let source = executions(&project.0).pop().unwrap();
        let original_task = source.join("replicate-000000/task-000000");
        let original_receipt = fs::read(original_task.join("workflow-result.json")).unwrap();

        study["active_phases"] = json!([1]);
        study["reuse_from"] = json!(source);
        study["phases"]["$npy"]["exclude_streams"] = json!(["checkpoint"]);
        project.write("study.json", &study);
        scientific_workflow::run(&project.0).unwrap();
        let resumed = executions(&project.0)
            .into_iter()
            .find(|path| *path != source)
            .unwrap();
        let reused = resumed.join("replicate-000000/task-000000");
        let receipt: Value =
            serde_json::from_slice(&fs::read(reused.join("workflow-result.json")).unwrap())
                .unwrap();
        assert_eq!(receipt["output_directory"], json!(original_task));
        assert!(!reused.join("artifacts").exists());
        assert!(
            resumed
                .join("replicate-000000/task-000001/artifacts/evolved.txt")
                .is_file()
        );

        study["reuse_from"] = json!(resumed);
        study["phases"]["$npy"]["exclude_streams"] = json!(["space"]);
        project.write("study.json", &study);
        scientific_workflow::run(&project.0).unwrap();
        assert_eq!(executions(&project.0).len(), 3);
        assert_eq!(
            fs::read(original_task.join("workflow-result.json")).unwrap(),
            original_receipt
        );

        project.write("parameters.json", &json!({"maximum_iterations": 36001}));
        let error = scientific_workflow::run(&project.0).unwrap_err();
        assert!(error.to_string().contains("captured study inputs differ"));
        assert_eq!(executions(&project.0).len(), 3);
    }
}
