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
    fn project_files_use_the_current_named_state_and_parameter_specification() {
        let project_root = Path::new(env!("CARGO_MANIFEST_DIR"));
        let manifest = project_root.join("wf_configs/study.json");
        let study: serde_json::Value =
            serde_json::from_slice(&fs::read(manifest).unwrap()).unwrap();
        let parameters: serde_json::Value = serde_json::from_slice(
            &fs::read(project_root.join("wf_configs/parameters.json")).unwrap(),
        )
        .unwrap();

        assert_eq!(
            study["paths"]["states"]["attractor"],
            "wf_configs/states/attractor.json"
        );
        assert_eq!(
            study["phases"]["simulate"]["tasks"][0]["model"],
            "attractor"
        );
        assert_eq!(
            study["phases"]["simulate"]["tasks"][0]["state"],
            "attractor"
        );
        assert_eq!(study["phases"]["simulate"]["start_interval_ms"], 2_000);
        assert!(parameters["attractor"].is_object());
        assert!(parameters["plot"].is_object());
    }

    #[test]
    fn current_project_preflights_through_the_public_study_boundary() {
        let project_root = Path::new(env!("CARGO_MANIFEST_DIR"));
        let study = scientific_workflow::study::advanced::Study::load(project_root).unwrap();

        assert_eq!(study.project_root(), project_root.canonicalize().unwrap());
        assert_eq!(study.output_root(), study.project_root().join("output"));
    }
}
