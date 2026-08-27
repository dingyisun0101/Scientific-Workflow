//! A complete model-plus-JSON Workflow project.

mod hopf_model;

fn main() -> Result<(), scientific_workflow::WorkflowError> {
    scientific_workflow::run(std::path::Path::new(env!("CARGO_MANIFEST_DIR")))
}

#[cfg(test)]
mod tests {
    use std::fs;
    use std::path::Path;

    #[test]
    fn simulation_tasks_retain_the_two_second_admission_interval() {
        let manifest = Path::new(env!("CARGO_MANIFEST_DIR")).join("study.json");
        let study: serde_json::Value =
            serde_json::from_slice(&fs::read(manifest).unwrap()).unwrap();

        assert_eq!(study["phases"]["simulate"]["start_interval_ms"], 2_000);
    }
}
