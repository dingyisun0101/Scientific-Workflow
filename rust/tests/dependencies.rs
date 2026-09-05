use scientific_workflow::task::{dependencies::Dependencies, project};
use std::path::Path;

#[test]
fn downstream_can_import_supported_dependency_and_layout_entry_points() {
    let deps = Dependencies::from_json(serde_json::json!([])).unwrap();
    assert!(deps.recordings().optional().unwrap().is_none());
    assert!(project::study_path(Path::new("/nonexistent-workflow-study")).is_err());
}
